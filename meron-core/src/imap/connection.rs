//! Connection timeouts, socket setup and authentication.

use super::*;

/// Cap on DNS plus every TCP attempt for one host. Without it a sick resolver
/// (getaddrinfo retrying across nameservers) or a run of black-holed addresses
/// can hold a sync for the better part of a minute before erroring; failing
/// fast surfaces the error banner while a retry is still worth offering. It has
/// to cover several sequential [`CONNECT_ATTEMPT_TIMEOUT`] rounds, so it is
/// deliberately larger than one attempt: capping it near a single attempt would
/// abandon a host whose later addresses are the reachable ones. This is the
/// only bound on the SMTP and certificate-probe paths, which do not run under
/// [`imap_connect_timeout`].
const TCP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Cap on the TLS handshake alone, for the same reason.
const TLS_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// Total budget for DNS through authentication. Leave 20% of the configured
/// background-sync budget for connection coordination and the actual mailbox
/// command. `MERON_SYNC_TIMEOUT` is raised precisely for gateways whose
/// handshake is slow (see [`crate::engine::background_sync_timeout`]), so this
/// scales with it rather than pinning a ceiling such a gateway could never
/// finish under.
fn imap_connect_timeout() -> std::time::Duration {
    crate::engine::background_sync_timeout().mul_f32(0.8)
}

/// Floor under [`imap_protocol_timeout`]. A stage cap is a fallback for a stage
/// that never completes, not a bar a slow-but-working server has to clear:
/// Gmail's XOAUTH2 exchange routinely takes 4-7s on a congested link, so an
/// even share of the default budget (8s) fails it often enough to keep the
/// connectivity banner up on an account that is fine. Keep the floor several
/// times the slowest healthy stage seen in the wild.
pub(super) const MIN_PROTOCOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The TCP/TLS stages may complete while the server never sends its greeting or
/// never finishes authentication. Keep those protocol stages independently
/// bounded so callers receive the concrete failing stage rather than only an
/// outer operation-budget timeout. Each stage gets a share of the connect
/// budget, floored at [`MIN_PROTOCOL_TIMEOUT`] and never above the budget
/// itself; it scales with `MERON_SYNC_TIMEOUT` for the same reason the connect
/// budget does, so a DavMail-style LOGIN that needs 15s is not capped into
/// permanent failure that no setting can lift. The stages run in sequence under
/// [`imap_connect_timeout`], which stays the real ceiling for the whole connect.
fn imap_protocol_timeout() -> std::time::Duration {
    protocol_timeout_for(imap_connect_timeout())
}

pub(super) fn protocol_timeout_for(connect: std::time::Duration) -> std::time::Duration {
    (connect / 3).max(MIN_PROTOCOL_TIMEOUT).min(connect)
}

/// Cap per resolved address: one black-hole address (typically an unroutable
/// IPv6 route ahead of a fine IPv4 one) must not eat the whole connect budget.
const CONNECT_ATTEMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Delay before the second of the two initially-dialed addresses. Alternating
/// families and staggering the opening attempts follows Happy Eyeballs'
/// important property: a broken preferred family cannot hold a working fallback
/// behind several full connection timeouts. Attempts that replace a failed one
/// start immediately.
const CONNECT_ATTEMPT_STAGGER: std::time::Duration = std::time::Duration::from_millis(250);

pub(super) fn interleave_address_families(
    addrs: Vec<std::net::SocketAddr>,
) -> Vec<std::net::SocketAddr> {
    let prefer_ipv6 = addrs.first().is_some_and(std::net::SocketAddr::is_ipv6);
    let (ipv6, ipv4): (Vec<_>, Vec<_>) = addrs.into_iter().partition(|addr| addr.is_ipv6());
    let (preferred, fallback) = if prefer_ipv6 {
        (ipv6, ipv4)
    } else {
        (ipv4, ipv6)
    };
    let mut preferred = preferred.into_iter();
    let mut fallback = fallback.into_iter();
    let mut interleaved = Vec::new();
    loop {
        let mut added = false;
        if let Some(addr) = preferred.next() {
            interleaved.push(addr);
            added = true;
        }
        if let Some(addr) = fallback.next() {
            interleaved.push(addr);
            added = true;
        }
        if !added {
            break;
        }
    }
    interleaved
}

/// Resolve `host` and connect to each address in resolver order with a short
/// per-attempt cap. Logs slow stages so a stalling first sync can be traced to
/// DNS vs TCP from device logs alone.
pub(crate) async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream> {
    tokio::time::timeout(TCP_CONNECT_TIMEOUT, connect_tcp_inner(host, port))
        .await
        .map_err(|_| {
            anyhow!(
                "TCP connect timed out after {}s",
                TCP_CONNECT_TIMEOUT.as_secs()
            )
        })?
}

async fn connect_tcp_inner(host: &str, port: u16) -> Result<TcpStream> {
    let dns_started = std::time::Instant::now();
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .context("dns lookup")?
        .collect();
    let dns_ms = dns_started.elapsed().as_millis();
    if dns_ms > 1_000 {
        crate::mlog!(
            crate::log::Level::Warn,
            "net",
            "slow DNS for {host}: {dns_ms}ms"
        );
    }
    let mut pending = interleave_address_families(addrs).into_iter();
    let mut attempts = tokio::task::JoinSet::new();
    let spawn_attempt = |attempts: &mut tokio::task::JoinSet<_>, addr, delay| {
        attempts.spawn(async move {
            tokio::time::sleep(delay).await;
            let started = std::time::Instant::now();
            let result =
                tokio::time::timeout(CONNECT_ATTEMPT_TIMEOUT, TcpStream::connect(addr)).await;
            (addr, started.elapsed(), result)
        });
    };
    if let Some(addr) = pending.next() {
        spawn_attempt(&mut attempts, addr, std::time::Duration::ZERO);
    }
    if let Some(addr) = pending.next() {
        spawn_attempt(&mut attempts, addr, CONNECT_ATTEMPT_STAGGER);
    }

    // Replacement attempts start as soon as a slot frees: the stagger exists to
    // avoid firing both families at once, not to add delay after an attempt has
    // already failed. Re-delaying here would push later addresses past
    // TCP_CONNECT_TIMEOUT and leave a reachable third address never tried.
    let mut last_err = anyhow!("dns lookup: no addresses for {host}");
    while let Some(attempt) = attempts.join_next().await {
        let (addr, elapsed, result) = match attempt {
            Ok(attempt) => attempt,
            Err(error) => {
                last_err = anyhow::Error::new(error).context("tcp connect task");
                if let Some(addr) = pending.next() {
                    spawn_attempt(&mut attempts, addr, std::time::Duration::ZERO);
                }
                continue;
            }
        };
        match result {
            Ok(Ok(tcp)) => {
                if elapsed.as_millis() > 1_000 {
                    crate::mlog!(
                        crate::log::Level::Warn,
                        "net",
                        "slow TCP connect to {addr}: {}ms",
                        elapsed.as_millis()
                    );
                }
                attempts.abort_all();
                // Send each command at once. With the default (Nagle) a
                // command written in two pieces waits ~40 ms for the server's
                // delayed ACK, on every command. Best effort: a refusal only
                // costs the speed-up.
                let _ = tcp.set_nodelay(true);
                return Ok(tcp);
            }
            Ok(Err(err)) => last_err = anyhow::Error::new(err).context(format!("connect {addr}")),
            Err(_) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "net",
                    "connect to {addr} timed out after {}ms",
                    elapsed.as_millis()
                );
                last_err = anyhow!("connect {addr}: timed out");
            }
        }
        if let Some(addr) = pending.next() {
            spawn_attempt(&mut attempts, addr, std::time::Duration::ZERO);
        }
    }
    Err(last_err.context("tcp connect"))
}

/// Open a TCP connection, optionally wrapped in implicit TLS. Shared by the
/// IMAP and SMTP paths.
///
/// `proxy` is the account's resolved proxy (see [`crate::proxy`]); `None`
/// connects directly. TLS is negotiated with the real destination host either
/// way, so the tunnel never terminates the mail server's certificate.
pub async fn connect_stream(
    host: &str,
    port: u16,
    tls: bool,
    proxy: Option<&crate::proxy::ProxyConfig>,
    cert_pin: Option<&str>,
) -> Result<Stream> {
    let tcp = open_socket(host, port, proxy).await?;
    if tls {
        Ok(Stream::tls(Box::new(
            upgrade_to_tls(host, tcp, cert_pin).await?,
        )))
    } else {
        Ok(Stream::plain(tcp))
    }
}

/// Connect the TCP socket (directly or through the proxy) and set the
/// keepalives every mail connection wants: NAT and firewalls drop idle IMAP
/// sessions silently otherwise, and an IDLE watcher never notices.
pub(crate) async fn open_socket(
    host: &str,
    port: u16,
    proxy: Option<&crate::proxy::ProxyConfig>,
) -> Result<TcpStream> {
    let tcp = match proxy {
        Some(proxy) => crate::proxy::connect_through(proxy, host, port).await?,
        None => connect_tcp(host, port).await?,
    };
    let sock_ref = socket2::SockRef::from(&tcp);
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(std::time::Duration::from_secs(60))
        .with_interval(std::time::Duration::from_secs(10));
    let _ = sock_ref.set_tcp_keepalive(&keepalive);
    Ok(tcp)
}

/// Wrap an established plaintext TCP socket in a TLS session. Used both by the
/// implicit-TLS path above and by the STARTTLS upgrade, which hands us the raw
/// socket after the cleartext negotiation completes.
pub async fn upgrade_to_tls(
    host: &str,
    tcp: TcpStream,
    cert_pin: Option<&str>,
) -> Result<TlsStream<TcpStream>> {
    let connector = crate::tls::connector(cert_pin)?;
    let server_name =
        rustls::pki_types::ServerName::try_from(host.to_string()).context("invalid server name")?;
    let handshake_started = std::time::Instant::now();
    let result = tokio::time::timeout(TLS_HANDSHAKE_TIMEOUT, connector.connect(server_name, tcp))
        .await
        .map_err(|_| anyhow::anyhow!("timed out"))
        .context("tls handshake")?;
    let handshake_ms = handshake_started.elapsed().as_millis();
    if handshake_ms > 1_000 {
        crate::mlog!(
            crate::log::Level::Warn,
            "net",
            "slow TLS handshake with {host}: {handshake_ms}ms"
        );
    }
    // Certificate rejections come back tagged, so the account dialog can offer
    // to show the certificate and pin it rather than dead-ending on a rustls
    // error. See [`crate::tls::UntrustedCertificate`].
    result.map_err(crate::tls::UntrustedCertificate::from_io)
}

pub struct XOAuth2Simple {
    auth_string: String,
    done: bool,
}

impl XOAuth2Simple {
    pub fn new(user: &str, access_token: &str) -> Self {
        Self {
            auth_string: format!("user={}\x01auth=Bearer {}\x01\x01", user, access_token),
            done: false,
        }
    }
}

impl async_imap::Authenticator for XOAuth2Simple {
    type Response = Vec<u8>;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        if self.done {
            Vec::new()
        } else {
            self.done = true;
            self.auth_string.clone().into_bytes()
        }
    }
}

/// Drive an IMAP session up to the point where the socket is ready for the TLS
/// handshake, and hand back the raw socket. Used only by the certificate probe
/// (see [`crate::tls::probe`]); no credentials are ever sent over it.
pub(crate) async fn starttls_socket(tcp: TcpStream) -> Result<TcpStream> {
    let mut client = async_imap::Client::new(Stream::plain(tcp));
    tokio::time::timeout(imap_protocol_timeout(), client.read_response())
        .await
        .map_err(|_| anyhow!("timed out"))
        .context("read IMAP greeting")?
        .context("read IMAP greeting")?
        .context("server closed before greeting")?;
    client
        .run_command_and_check_ok("STARTTLS", None)
        .await
        .context("STARTTLS")?;
    match client.into_inner() {
        Stream::Plain(tcp, _) => Ok(tcp),
        Stream::Tls(_, _) => Err(anyhow!("STARTTLS requested on an already-TLS stream")),
    }
}

pub async fn connect(creds: &Creds) -> Result<Session> {
    let timeout = imap_connect_timeout();
    tokio::time::timeout(timeout, connect_inner(creds))
        .await
        .map_err(|_| anyhow!("IMAP connect timed out after {}ms", timeout.as_millis()))?
}

async fn connect_inner(creds: &Creds) -> Result<Session> {
    // STARTTLS connects in cleartext and upgrades after the greeting; implicit
    // TLS wraps the socket up front. Plaintext (neither flag) is for local test
    // servers only.
    let implicit_tls = creds.tls && !creds.starttls;
    let stream = connect_stream(
        &creds.host,
        creds.port,
        implicit_tls,
        creds.proxy.resolve().as_ref(),
        creds.cert_pin.as_deref(),
    )
    .await?;
    let mut client = async_imap::Client::new(stream);
    // Consume the server greeting (e.g. "* OK ... ready"). Client::new does not
    // read it, and an unconsumed greeting shifts every response by one. That is
    // harmless for LOGIN (untagged lines are skipped), but it makes the XOAUTH2
    // handshake deadlock: the greeting is mistaken for the auth result and the
    // server's "+" continuation gets swallowed while we wait for a tagged reply.
    let greeting_started = std::time::Instant::now();
    tokio::time::timeout(imap_protocol_timeout(), client.read_response())
        .await
        .map_err(|_| anyhow!("timed out"))
        .context("read IMAP greeting")?
        .context("read IMAP greeting")?
        .context("server closed before greeting")?;
    let greeting_ms = greeting_started.elapsed().as_millis();
    if greeting_ms > 1_000 {
        crate::mlog!(
            crate::log::Level::Warn,
            "net",
            "slow IMAP greeting from {}: {greeting_ms}ms",
            creds.host
        );
    }

    // STARTTLS: ask the server to begin TLS, then upgrade the underlying socket
    // in place. There is no second greeting after STARTTLS, so we go straight to
    // auth on the new TLS client.
    if creds.starttls {
        client
            .run_command_and_check_ok("STARTTLS", None)
            .await
            .context("STARTTLS")?;
        let tcp = match client.into_inner() {
            Stream::Plain(tcp, _) => tcp,
            Stream::Tls(_, _) => {
                return Err(anyhow!("STARTTLS requested on an already-TLS stream"));
            }
        };
        let tls = upgrade_to_tls(&creds.host, tcp, creds.cert_pin.as_deref()).await?;
        client = async_imap::Client::new(Stream::tls(Box::new(tls)));
    }

    let auth_started = std::time::Instant::now();
    let session_result = if creds.is_oauth() {
        let auth = XOAuth2Simple::new(&creds.user, creds.access_token.as_deref().unwrap_or(""));
        tokio::time::timeout(
            imap_protocol_timeout(),
            client.authenticate("XOAUTH2", auth),
        )
        .await
        .map_err(|_| anyhow!("oauth login timed out"))?
        .map_err(|(e, _)| anyhow!("oauth login failed: {e}"))
    } else {
        tokio::time::timeout(
            imap_protocol_timeout(),
            client.login(&creds.user, &creds.password),
        )
        .await
        .map_err(|_| anyhow!("login timed out"))?
        .map_err(|(e, _)| anyhow!("login failed: {e}"))
    };
    let session = session_result?;
    let auth_ms = auth_started.elapsed().as_millis();
    if auth_ms > 1_000 {
        crate::mlog!(
            crate::log::Level::Warn,
            "net",
            "slow IMAP authentication with {}: {auth_ms}ms",
            creds.host
        );
    }
    Ok(session)
}
