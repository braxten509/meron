//! Establishes a setup-transfer connection between two devices.
//! The host listens for a peer and the joining side resolves the supplied target.
//! Session state carries connection progress back to the setup screen.

use super::*;

/// The computer starts the copy, both screens show it, the kept side sends.
pub(super) fn start_copy(session: &Arc<Session>) {
    let status = session.snapshot();
    session.update(|status| {
        status.stage = "copying";
        status.starting = false;
    });
    session.send(json!({ "t": "start", "keep": status.keep }));
    if status.keep == status.me {
        spawn_send_payload(session.clone());
    }
}

pub(super) async fn host(engine: &Arc<Engine>, device: &str) -> Result<Value> {
    let addresses = lan_addresses();
    if addresses.is_empty() {
        let session = new_session(
            engine,
            true,
            Status {
                me: device.to_string(),
                ..Default::default()
            },
        );
        session.fail("no_network");
        retire(&session);
        return Ok(status());
    }
    // A fixed port, so a firewall can let it in with one rule; any free port
    // when something else already holds it.
    let listener = match TcpListener::bind(("0.0.0.0", SETUP_COPY_PORT)).await {
        Ok(listener) => listener,
        Err(_) => TcpListener::bind(("0.0.0.0", 0)).await?,
    };
    let port = listener.local_addr()?.port();
    let code = new_code();
    let mine = counts(engine).await;
    let list = addresses
        .iter()
        .map(Ipv4Addr::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let qr = format!("{QR_PREFIX}{list}:{port}/{code}");
    let session = new_session(
        engine,
        true,
        Status {
            stage: "waiting",
            role: "host",
            me: device.to_string(),
            qr_svg: qr_svg(&qr),
            qr,
            address: format!("{}:{port}", addresses[0]),
            code: format!("{}-{}", &code[..4], &code[4..]),
            expires_at: store::now_unix() + LISTEN_FOR.as_secs() as i64,
            keep: device.to_string(),
            mine,
            attempts_left: MAX_ATTEMPTS,
            ..Default::default()
        },
    );
    tokio::spawn(accept_loop(session.clone(), listener, code));
    Ok(status())
}

async fn accept_loop(session: Arc<Session>, listener: TcpListener, code: String) {
    let mut stop = session.stop.subscribe();
    let deadline = tokio::time::Instant::now() + LISTEN_FOR;
    loop {
        let accepted = tokio::select! {
            _ = stopped(&mut stop) => {
                retire(&session);
                return;
            }
            _ = tokio::time::sleep_until(deadline) => {
                session.fail("expired");
                retire(&session);
                return;
            }
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, _)) = accepted else {
            continue;
        };
        let left = session.snapshot().attempts_left;
        // The five minutes hold for the pairing too: a phone that connected
        // just before the end can't finish after it.
        let limit = deadline.min(tokio::time::Instant::now() + HANDSHAKE_WAIT);
        let shaken = tokio::select! {
            _ = stopped(&mut stop) => {
                retire(&session);
                return;
            }
            shaken = tokio::time::timeout_at(limit, host_handshake(stream, &code, left)) => shaken,
        };
        if tokio::time::Instant::now() >= deadline {
            session.fail("expired");
            retire(&session);
            return;
        }
        match shaken {
            Ok(Ok(Some(channel))) => {
                // One use: nobody else gets in.
                drop(listener);
                run(session.clone(), channel).await;
                retire(&session);
                return;
            }
            Ok(Ok(None)) => {
                let left = left.saturating_sub(1);
                session.update(|status| status.attempts_left = left);
                if left == 0 {
                    session.fail("too_many_attempts");
                    retire(&session);
                    return;
                }
            }
            // Something that isn't Meron, or a phone that gave up: keep waiting.
            _ => {}
        }
    }
}

/// Where to connect, from a scanned QR code or the typed address and code.
fn parse_target(params: &Value) -> Result<(Vec<Ipv4Addr>, u16, String)> {
    let qr = params
        .get("qr")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let (hosts, port, code) = if !qr.is_empty() {
        let rest = qr.strip_prefix(QR_PREFIX).ok_or_else(|| fail("bad_qr"))?;
        let (address, code) = rest.split_once('/').ok_or_else(|| fail("bad_qr"))?;
        let (hosts, port) = address.rsplit_once(':').ok_or_else(|| fail("bad_qr"))?;
        (hosts.to_string(), port.to_string(), code.to_string())
    } else {
        let address = params
            .get("address")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let code = params
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (hosts, port) = address
            .rsplit_once(':')
            .ok_or_else(|| fail("bad_address"))?;
        (hosts.to_string(), port.to_string(), code.to_string())
    };
    let port: u16 = port.trim().parse().map_err(|_| fail("bad_address"))?;
    let hosts: Vec<Ipv4Addr> = hosts
        .split(',')
        .filter_map(|host| host.trim().parse().ok())
        .collect();
    if hosts.is_empty() || port == 0 {
        return Err(fail("bad_address"));
    }
    let code = normalize_code(&code);
    if code.len() != CODE_LEN {
        return Err(fail("bad_code"));
    }
    Ok((hosts, port, code))
}

pub(super) async fn join(engine: &Arc<Engine>, params: &Value, device: &str) -> Result<Value> {
    let (hosts, port, code) = match parse_target(params) {
        Ok(target) => target,
        Err(error) => {
            let session = new_session(
                engine,
                false,
                Status {
                    me: device.to_string(),
                    role: "guest",
                    ..Default::default()
                },
            );
            session.fail(failure_code(&error));
            retire(&session);
            return Ok(status());
        }
    };
    let mine = counts(engine).await;
    let session = new_session(
        engine,
        false,
        Status {
            stage: "connecting",
            role: "guest",
            me: device.to_string(),
            mine,
            ..Default::default()
        },
    );
    tokio::spawn(async move {
        let mut stop = session.stop.subscribe();
        let connected = tokio::select! {
            _ = stopped(&mut stop) => {
                retire(&session);
                return;
            }
            connected = connect_any(&hosts, port) => connected,
        };
        let stream = match connected {
            Ok(stream) => stream,
            Err(error) => {
                session.fail(failure_code(&error));
                retire(&session);
                return;
            }
        };
        match tokio::time::timeout(HANDSHAKE_WAIT, guest_handshake(stream, &code)).await {
            Ok(Ok(channel)) => run(session.clone(), channel).await,
            Ok(Err(error)) => session.fail(failure_code(&error)),
            Err(_) => session.fail("unreachable"),
        }
        retire(&session);
    });
    Ok(status())
}

/// Connect to the first address that answers, those on this phone's own network first. Fails with
/// `refused` (nothing listens: the computer stopped waiting, or a firewall said no),
/// `other_network` (this phone isn't on a local network the computer could be on) or `unreachable`
/// (no answer: most likely the computer's firewall).
async fn connect_any(hosts: &[Ipv4Addr], port: u16) -> Result<TcpStream> {
    let mut ordered: Vec<Ipv4Addr> = hosts.to_vec();
    ordered.sort_by_key(|host| !same_network(*host));
    let mut refused = false;
    for host in &ordered {
        let addr = SocketAddr::from((*host, port));
        match tokio::time::timeout(CONNECT_WAIT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                refused = true
            }
            _ => {}
        }
    }
    if refused {
        return Err(fail("refused"));
    }
    if !ordered.iter().any(|host| same_network(*host)) {
        return Err(fail("other_network"));
    }
    Err(fail("unreachable"))
}
