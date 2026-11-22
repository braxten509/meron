//! Controls the response timing of a local IMAP fetch for engine tests.
//! The reader and the prefetcher use the same simulated connection stalls.
//! The assertions distinguish an idle transfer from one making progress.

use super::*;

pub(crate) async fn slow_fetch_engine(
    connections: usize,
    delay: Duration,
    fail: bool,
) -> (Arc<Engine>, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    struct Host;
    impl EngineHost for Host {
        fn open_db(&self) -> anyhow::Result<rusqlite::Connection> {
            store::open_at(":memory:")
        }
        fn apply_secret(&self, _: &rusqlite::Connection, _: &str, _: &mut imap::Creds) {}
        fn store_secret(
            &self,
            _: &rusqlite::Connection,
            _: &str,
            _: &crate::secrets::Secrets,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        for _ in 0..connections {
            let (socket, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = socket.into_split();
            writer.write_all(b"* OK ready\r\n").await.unwrap();
            let mut lines = tokio::io::BufReader::new(reader).lines();
            while let Some(line) = lines.next_line().await.unwrap() {
                let (tag, command) = line.split_once(' ').unwrap();
                if command.starts_with("UID FETCH") {
                    if fail {
                        tokio::time::sleep(delay).await;
                        // Close in the middle of a literal so FETCH fails
                        // before delivering any complete message.
                        let _ = writer
                            .write_all(b"* 1 FETCH (UID 1 BODY[] {100}\r\npartial")
                            .await;
                        break;
                    }
                    let body = "Message-ID: <slow@test>\r\nSubject: Slow\r\n\r\nSlow body";
                    writer
                        .write_all(
                            format!("* 1 FETCH (UID 1 BODY[] {{{}}}\r\n", body.len()).as_bytes(),
                        )
                        .await
                        .unwrap();
                    let mut closed = false;
                    for chunk in body.as_bytes().chunks(4) {
                        tokio::time::sleep(delay).await;
                        if writer.write_all(chunk).await.is_err() {
                            closed = true;
                            break;
                        }
                    }
                    if !closed {
                        let _ = writer
                            .write_all(format!(")\r\n{tag} OK done\r\n").as_bytes())
                            .await;
                    }
                    break;
                }
                if command.starts_with("SELECT") {
                    writer.write_all(b"* 1 EXISTS\r\n").await.unwrap();
                }
                writer
                    .write_all(format!("{tag} OK done\r\n").as_bytes())
                    .await
                    .unwrap();
            }
        }
    });
    let engine = Arc::new(Engine::new(Box::new(Host)).unwrap());
    let creds = {
        let db = engine.db.lock().unwrap();
        let config = serde_json::json!({"host":"127.0.0.1", "port":port, "tls":false, "user":"u", "proxy":{"mode":"direct"}}).to_string();
        db.execute(
            "INSERT INTO accounts(id, config) VALUES('acc', ?1)",
            [&config],
        )
        .unwrap();
        store::upsert_messages(
            &db,
            "acc",
            "INBOX",
            &[imap::MessageHeader {
                uid: 1,
                subject: "Slow".into(),
                from_addr: "a@test".into(),
                ..Default::default()
            }],
        )
        .unwrap();
        store::load_account(&db, "acc").unwrap().unwrap()
    };
    engine.accounts.lock().await.insert("acc".into(), creds);
    (engine, server)
}

#[tokio::test]
async fn bytes_extend_the_idle_budget_before_a_whole_message_arrives() {
    let (engine, server) = slow_fetch_engine(1, Duration::from_millis(25), false).await;
    let wanted = BTreeMap::from([("INBOX".to_string(), vec![1])]);
    let fetched = fetch_bodies_keeping_partial(
        &engine,
        "acc",
        &wanted,
        &parse::test_media_root(),
        TransferLimits {
            idle: Duration::from_millis(100),
            total: Duration::from_secs(3),
        },
    )
    .await;
    fetched.result.unwrap();
    assert_eq!(fetched.messages.len(), 1);
    assert_eq!(fetched.messages[0].2.body.trim(), "Slow body");
    server.await.unwrap();
}

#[tokio::test]
async fn a_silent_socket_still_expires_before_the_total_budget() {
    let (engine, server) = slow_fetch_engine(1, Duration::from_millis(300), true).await;
    let wanted = BTreeMap::from([("INBOX".to_string(), vec![1])]);
    let fetched = fetch_bodies_keeping_partial(
        &engine,
        "acc",
        &wanted,
        &parse::test_media_root(),
        TransferLimits {
            idle: Duration::from_millis(50),
            total: Duration::from_secs(3),
        },
    )
    .await;
    assert!(fetched.result.unwrap_err().is::<TransferTimedOut>());
    assert!(fetched.messages.is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn ending_an_operation_disarms_the_idle_clock() {
    let clock: imap::ReadProgress = Arc::default();
    let mut operation = Box::pin(async {
        let _active = ActiveTransfer(clock.clone());
        *clock.lock().unwrap() = Some(tokio::time::Instant::now());
        std::future::pending::<()>().await;
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(1), &mut operation)
            .await
            .is_err()
    );
    // The timeout only dropped its reference. Dropping the operation is
    // what disarms the clock before a reconnect starts.
    assert!(clock.lock().unwrap().is_some());
    drop(operation);
    assert!(clock.lock().unwrap().is_none());
}
#[tokio::test]
async fn cancelled_prefetch_keeps_messages_received_before_the_batch_finishes() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    struct Host;
    impl EngineHost for Host {
        fn open_db(&self) -> anyhow::Result<rusqlite::Connection> {
            store::open_at(":memory:")
        }
        fn apply_secret(&self, _: &rusqlite::Connection, _: &str, _: &mut imap::Creds) {}
        fn store_secret(
            &self,
            _: &rusqlite::Connection,
            _: &str,
            _: &crate::secrets::Secrets,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = socket.into_split();
        writer.write_all(b"* OK ready\r\n").await.unwrap();
        let mut lines = tokio::io::BufReader::new(reader).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            let (tag, command) = line.split_once(' ').unwrap();
            if command.starts_with("UID FETCH") {
                let body = "Message-ID: <one@test>\r\nSubject: One\r\n\r\nFirst body";
                writer
                    .write_all(
                        format!("* 1 FETCH (UID 1 BODY[] {{{}}}\r\n{body})\r\n", body.len())
                            .as_bytes(),
                    )
                    .await
                    .unwrap();
                // The second message and the command completion never arrive.
                let _ = released.await;
                return;
            }
            if command.starts_with("SELECT") {
                writer.write_all(b"* 2 EXISTS\r\n").await.unwrap();
            }
            writer
                .write_all(format!("{tag} OK done\r\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let engine = Arc::new(Engine::new(Box::new(Host)).unwrap());
    let config = serde_json::json!({"host":"127.0.0.1", "port":port, "tls":false, "user":"u", "proxy":{"mode":"direct"}}).to_string();
    let creds = {
        let db = engine.db.lock().unwrap();
        db.execute(
            "INSERT INTO accounts(id, config) VALUES('acc', ?1)",
            [&config],
        )
        .unwrap();
        store::load_account(&db, "acc").unwrap().unwrap()
    };
    engine.accounts.lock().await.insert("acc".into(), creds);
    let worker = tokio::spawn({
        let engine = engine.clone();
        async move { prefetch_batch(&engine, "acc", "INBOX", &[1, 2], &parse::test_media_root()).await }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if store::has_cached_body(&engine.db.lock().unwrap(), "acc", "INBOX", 1).unwrap() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!worker.is_finished());
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    let db = engine.db.lock().unwrap();
    assert_eq!(
        store::get_cached_message(&db, "acc", "INBOX", 1)
            .unwrap()
            .unwrap()
            .body
            .trim(),
        "First body"
    );
    assert!(!store::has_cached_body(&db, "acc", "INBOX", 2).unwrap());
    drop(db);
    let _ = release.send(());
    server.await.unwrap();
}
