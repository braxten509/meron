//! Outbound proxy support: HTTP CONNECT and SOCKS5 for the mail sockets, and
//! the matching `ureq` agent for the blocking HTTP paths (feeds, OAuth token
//! exchange, avatars, changelog).
//!
//! Two levels of configuration. A single app-wide proxy lives in the `settings`
//! table under [`SETTING_KEY`] and is mirrored into a process-global slot
//! ([`set_global`]) so socket code deep in the IMAP/SMTP stack can reach it
//! without threading a DB handle through every call. Each account then carries a
//! [`ProxyChoice`]: inherit the global one (the default), force a direct
//! connection, or use its own proxy.
//!
//! Hostnames are always resolved by the proxy, never locally: SOCKS5 gets the
//! domain address type and HTTP CONNECT gets `host:port`. A proxy that leaks
//! every mail server's name to the local resolver would defeat the point of
//! routing the traffic through it at all.

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use std::sync::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Settings-table key holding the app-wide proxy (see [`parse_global`]).
pub const SETTING_KEY: &str = "proxy";

/// How to speak to the proxy server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProxyKind {
    /// HTTP CONNECT tunnel (RFC 7231 §4.3.6).
    Http,
    /// SOCKS5 (RFC 1928) with optional username/password auth (RFC 1929).
    Socks5,
}

impl ProxyKind {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "http" => Some(ProxyKind::Http),
            "socks5" => Some(ProxyKind::Socks5),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            ProxyKind::Http => "http",
            ProxyKind::Socks5 => "socks5",
        }
    }
}

/// A resolved proxy endpoint. Empty `username` means no authentication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyConfig {
    pub kind: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

impl ProxyConfig {
    fn from_json(v: &Value) -> Option<Self> {
        let kind = ProxyKind::parse(v["mode"].as_str().unwrap_or(""))?;
        let host = v["host"].as_str().unwrap_or("").trim().to_string();
        let port = v["port"].as_u64().unwrap_or(0) as u16;
        // A half-filled form is not a proxy. Treating it as one would silently
        // send every connection to port 0 on the empty host.
        if host.is_empty() || port == 0 {
            return None;
        }
        Some(ProxyConfig {
            kind,
            host,
            port,
            username: v["username"].as_str().unwrap_or("").to_string(),
            password: v["password"].as_str().unwrap_or("").to_string(),
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "mode": self.kind.as_str(),
            "host": self.host,
            "port": self.port,
            "username": self.username,
            "password": self.password,
        })
    }

    /// `host:port`, for logs and CONNECT targets.
    fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// What an individual account does about proxying.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProxyChoice {
    /// Follow the app-wide setting. The default, and what every pre-existing
    /// account deserializes to.
    #[default]
    Global,
    /// Ignore the app-wide setting and connect directly.
    Direct,
    /// This account's own proxy.
    Custom(ProxyConfig),
}

impl ProxyChoice {
    pub fn from_json(v: &Value) -> Self {
        match v["mode"].as_str().unwrap_or("global") {
            "direct" => ProxyChoice::Direct,
            "global" => ProxyChoice::Global,
            // Anything else is a proxy kind; a malformed custom entry falls back
            // to the global setting rather than to an unproxied connection.
            _ => ProxyConfig::from_json(v).map_or(ProxyChoice::Global, ProxyChoice::Custom),
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            ProxyChoice::Global => json!({ "mode": "global" }),
            ProxyChoice::Direct => json!({ "mode": "direct" }),
            ProxyChoice::Custom(cfg) => cfg.to_json(),
        }
    }

    /// The proxy this account actually connects through, if any.
    pub fn resolve(&self) -> Option<ProxyConfig> {
        match self {
            ProxyChoice::Global => global(),
            ProxyChoice::Direct => None,
            ProxyChoice::Custom(cfg) => Some(cfg.clone()),
        }
    }
}

/// Parse the app-wide setting. `{"mode":"off"}`, a missing row, or a
/// half-filled form all mean "no proxy".
pub fn parse_global(v: &Value) -> Option<ProxyConfig> {
    ProxyConfig::from_json(v)
}

static GLOBAL: RwLock<Option<ProxyConfig>> = RwLock::new(None);

/// Publish the app-wide proxy. Called once at engine start from the persisted
/// setting, and again whenever the user saves a new one.
///
/// Existing pooled IMAP sessions and IDLE watchers keep their current sockets;
/// the new setting takes effect as those reconnect.
pub fn set_global(cfg: Option<ProxyConfig>) {
    if let Ok(mut slot) = GLOBAL.write() {
        *slot = cfg;
    }
}

/// Serializes the tests that write the process-global slot; the default test
/// runner is multi-threaded, and a stray publish from another test would make
/// assertions about the global flaky.
#[cfg(test)]
pub(crate) static TEST_GLOBAL_LOCK: RwLock<()> = RwLock::new(());

/// The app-wide proxy, if one is configured.
pub fn global() -> Option<ProxyConfig> {
    GLOBAL.read().ok().and_then(|slot| slot.clone())
}

/// Load the persisted app-wide proxy into the process-global slot.
pub fn load_global(conn: &rusqlite::Connection) -> Result<()> {
    // Every engine/init in this process republishes from its own store. Tests
    // build many of those against throwaway databases, so hold the shared lock
    // to stay out of the way of a test asserting on the global.
    #[cfg(test)]
    let _serialize = TEST_GLOBAL_LOCK
        .read()
        .unwrap_or_else(|err| err.into_inner());
    let stored = crate::store::settings_get(conn, &[SETTING_KEY.to_string()])?;
    set_global(parse_global(&stored[SETTING_KEY]));
    Ok(())
}

/// Open a TCP connection to `host:port` *through* `proxy`.
///
/// The socket to the proxy itself is made with the ordinary connect path, so it
/// inherits the per-address timeouts and slow-DNS logging.
pub async fn connect_through(proxy: &ProxyConfig, host: &str, port: u16) -> Result<TcpStream> {
    connect_through_with_timeout(proxy, host, port, std::time::Duration::from_secs(15)).await
}

async fn connect_through_with_timeout(
    proxy: &ProxyConfig,
    host: &str,
    port: u16,
    handshake_timeout: std::time::Duration,
) -> Result<TcpStream> {
    let mut tcp = crate::imap::connect_tcp(&proxy.host, proxy.port)
        .await
        .with_context(|| format!("connect to proxy {}", proxy.endpoint()))?;
    let handshake = tokio::time::timeout(handshake_timeout, async {
        match proxy.kind {
            ProxyKind::Http => http_connect(&mut tcp, proxy, host, port).await,
            ProxyKind::Socks5 => socks5_connect(&mut tcp, proxy, host, port).await,
        }
    })
    .await
    .map_err(|_| anyhow!("proxy handshake timed out"))?;
    handshake.with_context(|| {
        format!(
            "{} proxy {} to {host}:{port}",
            proxy.kind.as_str(),
            proxy.endpoint()
        )
    })?;
    Ok(tcp)
}

// ---- HTTP CONNECT -----------------------------------------------------------

async fn http_connect(
    tcp: &mut TcpStream,
    proxy: &ProxyConfig,
    host: &str,
    port: u16,
) -> Result<()> {
    let mut req = format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n");
    if !proxy.username.is_empty() {
        use base64::Engine as _;
        let token = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", proxy.username, proxy.password));
        req.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
    }
    req.push_str("\r\n");
    tcp.write_all(req.as_bytes())
        .await
        .context("send CONNECT")?;
    tcp.flush().await.context("send CONNECT")?;

    // Read until the blank line ending the response head. Bounded, so a proxy
    // that answers with an endless header stream can't be used to exhaust
    // memory here.
    const MAX_HEAD: usize = 8 * 1024;
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HEAD {
            return Err(anyhow!("response head too large"));
        }
        let n = tcp.read(&mut byte).await.context("read CONNECT response")?;
        if n == 0 {
            return Err(anyhow!("proxy closed the connection"));
        }
        head.push(byte[0]);
    }

    let status_line = String::from_utf8_lossy(&head)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    // "HTTP/1.1 200 Connection established"
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("malformed response: {status_line:?}"))?;
    match code {
        200..=299 => Ok(()),
        407 => Err(anyhow!("proxy authentication required")),
        _ => Err(anyhow!("proxy refused: {status_line}")),
    }
}

// ---- SOCKS5 -----------------------------------------------------------------

const SOCKS5_VERSION: u8 = 0x05;
const SOCKS5_AUTH_NONE: u8 = 0x00;
const SOCKS5_AUTH_USERPASS: u8 = 0x02;
const SOCKS5_AUTH_UNACCEPTABLE: u8 = 0xFF;
const SOCKS5_CMD_CONNECT: u8 = 0x01;
const SOCKS5_ADDR_IPV4: u8 = 0x01;
const SOCKS5_ADDR_DOMAIN: u8 = 0x03;
const SOCKS5_ADDR_IPV6: u8 = 0x04;

async fn socks5_connect(
    tcp: &mut TcpStream,
    proxy: &ProxyConfig,
    host: &str,
    port: u16,
) -> Result<()> {
    let authenticating = !proxy.username.is_empty();
    // Offer user/pass only when we have credentials, so an open proxy doesn't
    // see a method it would have to reject.
    let greeting: Vec<u8> = if authenticating {
        vec![SOCKS5_VERSION, 2, SOCKS5_AUTH_NONE, SOCKS5_AUTH_USERPASS]
    } else {
        vec![SOCKS5_VERSION, 1, SOCKS5_AUTH_NONE]
    };
    tcp.write_all(&greeting).await.context("send greeting")?;
    tcp.flush().await.context("send greeting")?;

    let mut reply = [0u8; 2];
    tcp.read_exact(&mut reply)
        .await
        .context("read method selection")?;
    if reply[0] != SOCKS5_VERSION {
        return Err(anyhow!("not a SOCKS5 proxy (version {})", reply[0]));
    }
    match reply[1] {
        SOCKS5_AUTH_NONE => {}
        SOCKS5_AUTH_USERPASS if authenticating => socks5_userpass(tcp, proxy).await?,
        SOCKS5_AUTH_UNACCEPTABLE => {
            return Err(anyhow!(if authenticating {
                "proxy rejected the offered authentication methods"
            } else {
                "proxy requires authentication"
            }));
        }
        method => return Err(anyhow!("unsupported authentication method {method:#04x}")),
    }

    // CONNECT with the domain address type: the proxy resolves, we don't.
    let host_bytes = host.as_bytes();
    if host_bytes.len() > 255 {
        return Err(anyhow!("hostname too long for SOCKS5"));
    }
    let mut request = vec![
        SOCKS5_VERSION,
        SOCKS5_CMD_CONNECT,
        0x00, // reserved
        SOCKS5_ADDR_DOMAIN,
        host_bytes.len() as u8,
    ];
    request.extend_from_slice(host_bytes);
    request.extend_from_slice(&port.to_be_bytes());
    tcp.write_all(&request).await.context("send connect")?;
    tcp.flush().await.context("send connect")?;

    let mut head = [0u8; 4];
    tcp.read_exact(&mut head).await.context("read connect")?;
    if head[0] != SOCKS5_VERSION {
        return Err(anyhow!("not a SOCKS5 proxy (version {})", head[0]));
    }
    if head[1] != 0x00 {
        return Err(anyhow!("{}", socks5_error(head[1])));
    }
    // Drain the bound address so the stream starts at the tunnelled payload.
    let addr_len = match head[3] {
        SOCKS5_ADDR_IPV4 => 4,
        SOCKS5_ADDR_IPV6 => 16,
        SOCKS5_ADDR_DOMAIN => {
            let mut len = [0u8; 1];
            tcp.read_exact(&mut len).await.context("read bound host")?;
            len[0] as usize
        }
        other => return Err(anyhow!("unknown address type {other:#04x} in reply")),
    };
    let mut rest = vec![0u8; addr_len + 2]; // address + port
    tcp.read_exact(&mut rest).await.context("read bound addr")?;
    Ok(())
}

async fn socks5_userpass(tcp: &mut TcpStream, proxy: &ProxyConfig) -> Result<()> {
    let user = proxy.username.as_bytes();
    let pass = proxy.password.as_bytes();
    if user.len() > 255 || pass.len() > 255 {
        return Err(anyhow!("proxy credentials too long"));
    }
    let mut msg = vec![0x01, user.len() as u8]; // sub-negotiation version 1
    msg.extend_from_slice(user);
    msg.push(pass.len() as u8);
    msg.extend_from_slice(pass);
    tcp.write_all(&msg).await.context("send credentials")?;
    tcp.flush().await.context("send credentials")?;

    let mut reply = [0u8; 2];
    tcp.read_exact(&mut reply)
        .await
        .context("read auth reply")?;
    if reply[1] != 0x00 {
        return Err(anyhow!("proxy rejected the credentials"));
    }
    Ok(())
}

fn socks5_error(code: u8) -> &'static str {
    match code {
        0x01 => "general SOCKS server failure",
        0x02 => "connection not allowed by ruleset",
        0x03 => "network unreachable",
        0x04 => "host unreachable",
        0x05 => "connection refused",
        0x06 => "TTL expired",
        0x07 => "command not supported",
        0x08 => "address type not supported",
        _ => "unknown SOCKS5 failure",
    }
}

// ---- Blocking HTTP ----------------------------------------------------------

/// A `ureq` agent honouring the app-wide proxy. Every outbound HTTP call in the
/// core goes through this so feed fetches, avatar downloads and OAuth token
/// exchanges don't quietly bypass a configured proxy.
///
/// Built per call rather than cached: the setting can change at runtime, and
/// these requests are network-bound anyway.
pub fn agent() -> Result<ureq::Agent> {
    agent_for(global().as_ref())
}

/// Build an HTTP agent for a resolved account route. `None` is deliberately
/// direct; callers that want the app-wide route should use [`agent`].
pub fn agent_for(cfg: Option<&ProxyConfig>) -> Result<ureq::Agent> {
    let Some(cfg) = cfg else {
        return Ok(ureq::Agent::new_with_defaults());
    };
    let protocol = match cfg.kind {
        ProxyKind::Http => ureq::ProxyProtocol::Http,
        // Socks5h: the proxy resolves the hostname, matching the mail path.
        ProxyKind::Socks5 => ureq::ProxyProtocol::Socks5h,
    };
    // ureq accepts a URI authority here, so an IPv6 literal needs brackets.
    // Keep the stored/raw host unchanged for Tokio's resolver.
    let http_host = match cfg.host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V6(_)) => format!("[{}]", cfg.host),
        _ => cfg.host.clone(),
    };
    let mut builder = ureq::Proxy::builder(protocol)
        .host(&http_host)
        .port(cfg.port);
    if !cfg.username.is_empty() {
        builder = builder.username(&cfg.username).password(&cfg.password);
    }
    let proxy = builder
        .build()
        .with_context(|| format!("invalid proxy {}", cfg.endpoint()))?;
    Ok(ureq::Agent::new_with_config(
        ureq::Agent::config_builder().proxy(Some(proxy)).build(),
    ))
}

#[cfg(test)]
mod tests;
