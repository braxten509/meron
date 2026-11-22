//! Regression tests for proxy.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;
use tokio::net::TcpListener;

fn socks5(host: &str, port: u16) -> ProxyConfig {
    ProxyConfig {
        kind: ProxyKind::Socks5,
        host: host.to_string(),
        port,
        username: String::new(),
        password: String::new(),
    }
}

#[test]
fn account_choice_defaults_to_global() {
    assert_eq!(ProxyChoice::from_json(&json!({})), ProxyChoice::Global);
    assert_eq!(ProxyChoice::from_json(&Value::Null), ProxyChoice::Global);
    assert_eq!(
        ProxyChoice::from_json(&json!({ "mode": "direct" })),
        ProxyChoice::Direct
    );
}

#[test]
fn account_choice_round_trips_a_custom_proxy() {
    let choice = ProxyChoice::Custom(ProxyConfig {
        kind: ProxyKind::Http,
        host: "gateway.corp".into(),
        port: 3128,
        username: "u".into(),
        password: "p".into(),
    });
    assert_eq!(ProxyChoice::from_json(&choice.to_json()), choice);
}

#[test]
fn half_filled_config_is_not_a_proxy() {
    // Host without port, port without host, and an unknown scheme all fall
    // back rather than producing a proxy that can never connect.
    assert_eq!(
        parse_global(&json!({ "mode": "socks5", "host": "h" })),
        None
    );
    assert_eq!(
        parse_global(&json!({ "mode": "socks5", "port": 1080 })),
        None
    );
    assert_eq!(
        parse_global(&json!({ "mode": "off", "host": "h", "port": 1080 })),
        None
    );
    assert_eq!(
        ProxyChoice::from_json(&json!({ "mode": "socks5", "host": "h" })),
        ProxyChoice::Global
    );
}

#[test]
fn custom_proxy_wins_over_global_and_direct_disables_it() {
    let _guard = TEST_GLOBAL_LOCK
        .write()
        .unwrap_or_else(|err| err.into_inner());
    set_global(Some(socks5("global.example", 1080)));
    let custom = ProxyConfig {
        kind: ProxyKind::Http,
        host: "account.example".into(),
        port: 8080,
        username: String::new(),
        password: String::new(),
    };
    assert_eq!(
        ProxyChoice::Custom(custom.clone()).resolve(),
        Some(custom.clone())
    );
    assert_eq!(ProxyChoice::Direct.resolve(), None);
    assert_eq!(
        ProxyChoice::Global.resolve(),
        Some(socks5("global.example", 1080))
    );
    set_global(None);
    assert_eq!(ProxyChoice::Global.resolve(), None);
}

/// Accept one connection, run `handler` against it, and hand back what the
/// client wrote so the request bytes can be asserted on.
async fn with_proxy_server<F>(handler: F) -> (u16, tokio::task::JoinHandle<Vec<u8>>)
where
    F: FnOnce(Vec<u8>) -> Vec<u8> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // Read the first request, reply, then echo back everything read.
        let mut buf = vec![0u8; 512];
        let n = sock.read(&mut buf).await.unwrap();
        buf.truncate(n);
        let reply = handler(buf.clone());
        sock.write_all(&reply).await.unwrap();
        sock.flush().await.unwrap();
        buf
    });
    (port, task)
}

#[tokio::test]
async fn socks5_sends_the_hostname_for_the_proxy_to_resolve() {
    // Two-step handshake: method selection, then the CONNECT reply.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut greeting = [0u8; 3];
        sock.read_exact(&mut greeting).await.unwrap();
        sock.write_all(&[SOCKS5_VERSION, SOCKS5_AUTH_NONE])
            .await
            .unwrap();
        let mut head = [0u8; 5];
        sock.read_exact(&mut head).await.unwrap();
        let mut host = vec![0u8; head[4] as usize + 2];
        sock.read_exact(&mut host).await.unwrap();
        sock.write_all(&[
            SOCKS5_VERSION,
            0x00,
            0x00,
            SOCKS5_ADDR_IPV4,
            0,
            0,
            0,
            0,
            0,
            0,
        ])
        .await
        .unwrap();
        (greeting.to_vec(), head.to_vec(), host)
    });

    connect_through(&socks5("127.0.0.1", port), "imap.example.com", 993)
        .await
        .unwrap();

    let (greeting, head, host) = server.await.unwrap();
    assert_eq!(greeting, vec![SOCKS5_VERSION, 1, SOCKS5_AUTH_NONE]);
    assert_eq!(head[1], SOCKS5_CMD_CONNECT);
    assert_eq!(
        head[3], SOCKS5_ADDR_DOMAIN,
        "hostname must not be pre-resolved"
    );
    assert_eq!(&host[..host.len() - 2], b"imap.example.com");
    assert_eq!(&host[host.len() - 2..], &993u16.to_be_bytes());
}

#[tokio::test]
async fn socks5_surfaces_a_refusal() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut greeting = [0u8; 3];
        sock.read_exact(&mut greeting).await.unwrap();
        sock.write_all(&[SOCKS5_VERSION, SOCKS5_AUTH_NONE])
            .await
            .unwrap();
        let mut buf = [0u8; 256];
        let _ = sock.read(&mut buf).await.unwrap();
        sock.write_all(&[
            SOCKS5_VERSION,
            0x05,
            0x00,
            SOCKS5_ADDR_IPV4,
            0,
            0,
            0,
            0,
            0,
            0,
        ])
        .await
        .unwrap();
    });

    let err = connect_through(&socks5("127.0.0.1", port), "imap.example.com", 993)
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("connection refused"),
        "unexpected error: {err:#}"
    );
}

#[tokio::test]
async fn socks5_requires_credentials_when_the_proxy_asks_for_them() {
    let (port, _server) =
        with_proxy_server(|_req| vec![SOCKS5_VERSION, SOCKS5_AUTH_UNACCEPTABLE]).await;
    let err = connect_through(&socks5("127.0.0.1", port), "imap.example.com", 993)
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("requires authentication"),
        "unexpected error: {err:#}"
    );
}

#[tokio::test]
async fn http_connect_tunnels_and_authenticates() {
    let (port, server) =
        with_proxy_server(|_req| b"HTTP/1.1 200 Connection established\r\n\r\n".to_vec()).await;
    let proxy = ProxyConfig {
        kind: ProxyKind::Http,
        host: "127.0.0.1".into(),
        port,
        username: "user".into(),
        password: "pass".into(),
    };
    connect_through(&proxy, "smtp.example.com", 587)
        .await
        .unwrap();

    let req = String::from_utf8(server.await.unwrap()).unwrap();
    assert!(
        req.starts_with("CONNECT smtp.example.com:587 HTTP/1.1\r\n"),
        "{req:?}"
    );
    // base64("user:pass")
    assert!(
        req.contains("Proxy-Authorization: Basic dXNlcjpwYXNz\r\n"),
        "{req:?}"
    );
}

#[tokio::test]
async fn http_connect_reports_a_rejection() {
    let (port, _server) =
        with_proxy_server(|_req| b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n".to_vec())
            .await;
    let proxy = ProxyConfig {
        kind: ProxyKind::Http,
        host: "127.0.0.1".into(),
        port,
        username: String::new(),
        password: String::new(),
    };
    let err = connect_through(&proxy, "smtp.example.com", 587)
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("proxy authentication required"),
        "unexpected error: {err:#}"
    );
}

#[tokio::test]
async fn proxy_handshake_is_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (_sock, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    });
    let proxy = ProxyConfig {
        kind: ProxyKind::Http,
        host: "127.0.0.1".into(),
        port,
        username: String::new(),
        password: String::new(),
    };

    let err = connect_through_with_timeout(
        &proxy,
        "imap.example.com",
        993,
        std::time::Duration::from_millis(25),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("proxy handshake timed out"),
        "unexpected error: {err:#}"
    );
    server.abort();
}

#[test]
fn ipv6_http_proxy_builds_and_invalid_proxy_does_not_fall_back_to_direct() {
    let ipv6 = ProxyConfig {
        kind: ProxyKind::Http,
        host: "::1".into(),
        port: 8080,
        username: String::new(),
        password: String::new(),
    };
    assert!(agent_for(Some(&ipv6)).is_ok());

    let invalid = ProxyConfig {
        host: "bad\nhost".into(),
        ..ipv6
    };
    assert!(agent_for(Some(&invalid)).is_err());
}
