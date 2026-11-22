//! Exercises the connection lifecycle during message transfers.
//! The test host and local protocol fixtures control credential refresh and reconnection.
//! Assertions cover copy IDs, timeouts, retry limits and reconnect notifications.

use crate::engine::TransferTimedOut;
use crate::imap;
use rusqlite::Connection;
use std::time::Duration;

#[test]
fn moved_copy_uids_picks_only_the_copies_a_move_created() {
    use crate::engine::sync::moved_copy_uids;
    let header = |uid: u32, message_id: &str| imap::MessageHeader {
        uid,
        message_id: message_id.to_string(),
        ..Default::default()
    };
    let id = |value: &str| value.to_string();

    // Arrivals past the pre-move UIDNEXT: the two moved copies, plus mail
    // delivered meanwhile — with another Message-ID, and with none at all.
    let arrivals = [
        header(9, "<Moved@Example.com>"),
        header(10, "<other@example.com>"),
        header(11, ""),
        header(12, "<second@example.com>"),
    ];
    let moved = [id("<moved@example.com>"), id("<second@example.com>")];
    assert_eq!(moved_copy_uids(&arrivals, &moved), vec![9, 12]);

    // A moved copy that isn't among the arrivals: incomplete, so nothing.
    let moved = [id("<moved@example.com>"), id("<missing@example.com>")];
    assert!(moved_copy_uids(&arrivals, &moved).is_empty());

    // An id-less move is picked out only when the id-less arrivals match it
    // one for one; a second one arriving meanwhile makes it ambiguous.
    assert_eq!(moved_copy_uids(&arrivals, &[id("")]), vec![11]);
    let crowded = [header(11, ""), header(13, "")];
    assert!(moved_copy_uids(&crowded, &[id("")]).is_empty());

    // Duplicate Message-IDs count per copy.
    let twins = [
        header(20, "<twin@example.com>"),
        header(21, "<twin@example.com>"),
    ];
    let moved = [id("<twin@example.com>"), id("<twin@example.com>")];
    assert_eq!(moved_copy_uids(&twins, &moved), vec![20, 21]);
    assert!(moved_copy_uids(&twins, &moved[..1]).is_empty());
}

#[tokio::test]
async fn transfer_deadline_bounds_credential_wait_without_cancelling_refresh() {
    use std::io::{Read, Write};
    use std::sync::Arc;

    struct RefreshHost(Arc<tokio::sync::Notify>);
    impl crate::engine::EngineHost for RefreshHost {
        fn open_db(&self) -> anyhow::Result<Connection> {
            crate::store::open_at(":memory:")
        }
        fn apply_secret(&self, _: &Connection, _: &str, _: &mut imap::Creds) {}
        fn store_secret(
            &self,
            _: &Connection,
            _: &str,
            secret: &crate::secrets::Secrets,
        ) -> anyhow::Result<()> {
            assert_eq!(secret.access_token.as_deref(), Some("refreshed"));
            self.0.notify_one();
            Ok(())
        }
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            headers.push(byte[0]);
        }
        let content_length = std::str::from_utf8(&headers)
            .unwrap()
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .expect("OAuth request must have Content-Length")
            .1
            .trim()
            .parse::<usize>()
            .unwrap();
        // Closing with unread POST data can reset the connection before the
        // client finishes reading the response, making persistence time out.
        let mut request_body = vec![0; content_length];
        stream.read_exact(&mut request_body).unwrap();
        started_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let body = r#"{"access_token":"refreshed","expires_in":3600}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let persisted = Arc::new(tokio::sync::Notify::new());
    let engine =
        Arc::new(crate::engine::Engine::new(Box::new(RefreshHost(persisted.clone()))).unwrap());
    let mut creds = {
        let db = engine.db.lock().unwrap();
        db.execute(
            "INSERT INTO accounts(id, config) VALUES('acc', ?1)",
            [r#"{"host":"localhost","user":"u","proxy":{"mode":"direct"}}"#],
        )
        .unwrap();
        crate::store::load_account(&db, "acc").unwrap().unwrap()
    };
    creds.auth_type = "gmail_oauth".into();
    creds.refresh_token = Some("refresh".into());
    creds.oauth_client_id = "client".into();
    creds.oauth_token_url = format!("http://{address}/token");
    engine.accounts.lock().await.insert("acc".into(), creds);
    let waiter = tokio::spawn({
        let engine = engine.clone();
        async move {
            engine
                .with_transfer_session("acc", Duration::from_millis(200), |_| {
                    Box::pin(async { Ok(()) })
                })
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), started_rx)
        .await
        .unwrap()
        .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("session preparation timed out"));
    assert!(!error.is::<TransferTimedOut>());
    release_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), persisted.notified())
        .await
        .unwrap();
    assert_eq!(
        engine.accounts.lock().await["acc"].access_token.as_deref(),
        Some("refreshed")
    );
    assert!(
        crate::store::load_accounts(&engine.db.lock().unwrap()).unwrap()[0]
            .1
            .token_expires_at
            > 0
    );
    server.join().unwrap();
}

#[tokio::test]
async fn pooled_transfer_server_no_is_retried_once_fresh() {
    pooled_transfer_recovery(false, false, false).await;
}

#[tokio::test]
async fn stale_pooled_transfer_reuses_a_session_returned_during_coordination() {
    pooled_transfer_recovery(true, false, false).await;
}

#[tokio::test]
async fn pooled_transfer_command_is_not_run_a_third_time() {
    pooled_transfer_recovery(true, true, false).await;
}

#[tokio::test]
async fn read_with_a_second_stale_pooled_session_reconnects_before_retrying_the_command() {
    pooled_transfer_recovery(true, false, true).await;
}

async fn pooled_transfer_recovery(
    reuse_returned: bool,
    second_fails: bool,
    read_with_stale_second: bool,
) {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    struct Host;
    impl crate::engine::EngineHost for Host {
        fn open_db(&self) -> anyhow::Result<Connection> {
            crate::store::open_at(":memory:")
        }
        fn apply_secret(&self, _: &Connection, _: &str, _: &mut imap::Creds) {}
        fn store_secret(
            &self,
            _: &Connection,
            _: &str,
            _: &crate::secrets::Secrets,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let failed = Arc::new(tokio::sync::Notify::new());
    let server_failed = failed.clone();
    let server = tokio::spawn(async move {
        for connection in 0..if read_with_stale_second { 3 } else { 2 } {
            let (socket, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = socket.into_split();
            writer.write_all(b"* OK ready\r\n").await.unwrap();
            let mut lines = tokio::io::BufReader::new(reader).lines();
            while let Some(line) = lines.next_line().await.unwrap() {
                let (tag, command) = line.split_once(' ').unwrap();
                let operation = command.starts_with("SELECT");
                let fail = ((connection == 0 || second_fails) && operation)
                    || (read_with_stale_second && connection == 1 && command.starts_with("NOOP"));
                if connection >= 1 && operation {
                    writer.write_all(b"* 0 EXISTS\r\n").await.unwrap();
                }
                let status = if fail {
                    "NO genuine server refusal"
                } else {
                    "OK done"
                };
                writer
                    .write_all(format!("{tag} {status}\r\n").as_bytes())
                    .await
                    .unwrap();
                if fail {
                    server_failed.notify_one();
                }
                if fail || (connection >= 1 && operation) {
                    break;
                }
            }
        }
    });
    let engine = Arc::new(crate::engine::Engine::new(Box::new(Host)).unwrap());
    let creds = {
        let db = engine.db.lock().unwrap();
        let config = serde_json::json!({"host":"127.0.0.1", "port":port, "tls":false, "user":"u", "proxy":{"mode":"direct"}}).to_string();
        db.execute(
            "INSERT INTO accounts(id, config) VALUES('acc', ?1)",
            [&config],
        )
        .unwrap();
        crate::store::load_account(&db, "acc").unwrap().unwrap()
    };
    engine
        .accounts
        .lock()
        .await
        .insert("acc".into(), creds.clone());
    engine.return_pooled("acc", imap::connect(&creds).await.unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let coordination = engine.connect_lock("acc");
    let guard = if reuse_returned {
        Some(coordination.lock().await)
    } else {
        None
    };
    let worker = tokio::spawn({
        let engine = engine.clone();
        let calls = calls.clone();
        async move {
            if read_with_stale_second {
                engine
                    .with_read_session("acc", |session| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Box::pin(async move {
                            session.select("INBOX").await?;
                            Ok(())
                        })
                    })
                    .await
            } else {
                engine
                    .with_transfer_session("acc", Duration::from_secs(3), |session| {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Box::pin(async move {
                            session.select("INBOX").await?;
                            Ok(())
                        })
                    })
                    .await
            }
        }
    });
    if reuse_returned {
        tokio::time::timeout(Duration::from_secs(1), failed.notified())
            .await
            .unwrap();
        engine.return_pooled("acc", imap::connect(&creds).await.unwrap());
    }
    drop(guard);
    let result = worker.await.unwrap();
    if second_fails {
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("genuine server refusal")
        );
    } else {
        result.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn transfer_deadline_bounds_connection_coordination() {
    struct Host;
    impl crate::engine::EngineHost for Host {
        fn open_db(&self) -> anyhow::Result<Connection> {
            crate::store::open_at(":memory:")
        }
        fn apply_secret(&self, _: &Connection, _: &str, _: &mut imap::Creds) {}
        fn store_secret(
            &self,
            _: &Connection,
            _: &str,
            _: &crate::secrets::Secrets,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let engine = crate::engine::Engine::new(Box::new(Host)).unwrap();
    let coordination = engine.connect_lock("acc");
    let _connect_guard = coordination.lock().await;
    let _accounts_guard = engine.accounts.lock().await;
    let error = tokio::time::timeout(
        Duration::from_millis(500),
        engine.with_transfer_session::<(), _>("acc", Duration::from_millis(50), |_| {
            Box::pin(async { panic!("No command should run before preparation succeeds") })
        }),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(error.to_string().contains("session preparation timed out"));
    assert!(!error.is::<TransferTimedOut>());
}

#[test]
fn only_a_reconnect_failure_becomes_a_reconnect_event() {
    let refused = anyhow::anyhow!("account needs reconnect: a@example.com").context("sync INBOX");
    let (name, detail) = crate::engine::needs_reconnect_event("a@example.com", &refused).unwrap();
    assert_eq!(name, "account.needsReconnect");
    assert_eq!(detail["account"], "a@example.com");

    let offline = anyhow::anyhow!("connection refused");
    assert!(crate::engine::needs_reconnect_event("a@example.com", &offline).is_none());
}
