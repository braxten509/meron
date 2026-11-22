//! Guard tests in the mail synchronization engine.
//!
//! Coordinates the corresponding IMAP and store operations during folder updates.
//! The parent module exposes these operations to foreground and background sync.

use super::*;

struct TestHost;

impl EngineHost for TestHost {
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

async fn metadata_test_server(
    total: u32,
    poison_uids: Vec<u32>,
) -> (
    Arc<Engine>,
    tokio::task::JoinHandle<()>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    metadata_test_server_with_stall(total, poison_uids, None).await
}

async fn metadata_test_server_with_stall(
    total: u32,
    poison_uids: Vec<u32>,
    stalled_uid: Option<u32>,
) -> (
    Arc<Engine>,
    tokio::task::JoinHandle<()>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let commands_after_poison = Arc::new(AtomicUsize::new(0));
    let server_counter = commands_after_poison.clone();
    let server = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let counter = server_counter.clone();
            let poison_uids = poison_uids.clone();
            tokio::spawn(async move {
                let (reader, mut writer) = socket.into_split();
                writer.write_all(b"* OK IMAP4rev1 ready\r\n").await.unwrap();
                let mut lines = tokio::io::BufReader::new(reader).lines();
                let mut poisoned = false;
                while let Ok(Some(line)) = lines.next_line().await {
                    if poisoned {
                        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    let (tag, command) = line.split_once(' ').unwrap();
                    let mut reply = if command.starts_with("SELECT") {
                        format!(
                            "* {total} EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n* OK [UIDNEXT {}] next\r\n",
                            total + 1
                        )
                    } else if command.starts_with("CAPABILITY") {
                        "* CAPABILITY IMAP4rev1\r\n".to_string()
                    } else if command.starts_with("UID SEARCH") {
                        if command.contains("UNSEEN") {
                            "* SEARCH\r\n".to_string()
                        } else {
                            format!(
                                "* SEARCH {}\r\n",
                                (1..=total)
                                    .map(|uid| uid.to_string())
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            )
                        }
                    } else if command.starts_with("UID FETCH") {
                        let set = command.split_whitespace().nth(2).unwrap();
                        let mut uids = Vec::new();
                        // `*` is the newest message, as in "1:*" (the whole folder).
                        let bound = |text: &str| match text {
                            "*" => total,
                            number => number.parse::<u32>().unwrap(),
                        };
                        for segment in set.split(',') {
                            if let Some((lo, hi)) = segment.split_once(':') {
                                uids.extend(bound(lo)..=bound(hi));
                            } else {
                                uids.push(bound(segment));
                            }
                        }
                        // Only a read that asks for BODYSTRUCTURE can carry the
                        // broken one: a flags-only read neither poisons nor stalls.
                        if !command.contains("BODYSTRUCTURE") {
                            uids.iter()
                                .map(|uid| format!("* {uid} FETCH (UID {uid} FLAGS (\\Seen))\r\n"))
                                .collect()
                        } else if let Some(uid) = uids.iter().find(|uid| poison_uids.contains(uid))
                        {
                            poisoned = true;
                            format!(
                                "* {uid} FETCH (UID {uid} BODYSTRUCTURE (\"APPLICATION\" \"café\" NIL NIL NIL \"7BIT\" 1))\r\n"
                            )
                        } else if stalled_uid.is_some_and(|uid| uids.contains(&uid)) {
                            // Hold a partial FETCH open until cancellation drops
                            // the connection. No later command may reuse it.
                            writer.write_all(b"* 1 FETCH (UID 1 ").await.unwrap();
                            if let Ok(Some(_)) = lines.next_line().await {
                                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            }
                            break;
                        } else {
                            uids.iter().map(|uid| {
                                if *uid == 2 { format!("* 2 FETCH (UID 2 FLAGS (\\Seen) BODYSTRUCTURE (\"APPLICATION\" \"PDF\" (\"NAME\" \"older.pdf\") NIL NIL \"BASE64\" 4))\r\n") }
                                else if *uid == total { format!("* {uid} FETCH (UID {uid} FLAGS (\\Seen) BODYSTRUCTURE (\"TEXT\" \"PLAIN\" NIL NIL NIL \"7BIT\" 4 1))\r\n") }
                                else { format!("* {uid} FETCH (UID {uid} FLAGS (\\Seen))\r\n") }
                            }).collect()
                        }
                    } else if command.starts_with("FETCH") {
                        format!(
                            "* {total} FETCH (UID {total} FLAGS (\\Seen) BODYSTRUCTURE (\"TEXT\" \"PLAIN\" NIL NIL NIL \"7BIT\" 4 1))\r\n"
                        )
                    } else {
                        String::new()
                    };
                    reply.push_str(&format!("{tag} OK done\r\n"));
                    if writer.write_all(reply.as_bytes()).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let engine = Arc::new(Engine::new(Box::new(TestHost)).unwrap());
    let mut creds = {
        let db = engine.db.lock().unwrap();
        let config = serde_json::json!({"host":"127.0.0.1", "port":port, "tls":false, "user":"u", "proxy":{"mode":"direct"}}).to_string();
        db.execute(
            "INSERT INTO accounts(id, config) VALUES('acct', ?1)",
            [&config],
        )
        .unwrap();
        store::set_folder_state(&db, "acct", "INBOX", 42, total + 1).unwrap();
        let headers: Vec<_> = (1..=total)
            .map(|uid| imap::MessageHeader {
                uid,
                date: uid as i64,
                seen: true,
                files: (uid == total).then(Vec::new),
                ..Default::default()
            })
            .collect();
        store::upsert_messages(&db, "acct", "INBOX", &headers).unwrap();
        store::load_account(&db, "acct").unwrap().unwrap()
    };
    creds.password = "p".into();
    engine.accounts.lock().await.insert("acct".into(), creds);
    (engine, server, commands_after_poison)
}

/// UID 4 poisons every FETCH containing it, UID 3 omits BODYSTRUCTURE,
/// and UID 2 carries a file. Catch-up must reach UID 2 on the next sync.
#[tokio::test]
async fn catchup_discards_poison_sessions_advances_and_preserves_sync_count() {
    use std::sync::atomic::Ordering;
    let (engine, server, commands_after_poison) = metadata_test_server(5, vec![4]).await;
    for (expected_missing, expected_read) in [(vec![2, 1], 1), (vec![], 0)] {
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            sync_messages(&engine, "acct", "INBOX", 2),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.count, 2);
        assert_eq!(result.messages.len(), expected_read);
        let db = engine.db.lock().unwrap();
        assert_eq!(
            store::missing_attachment_metadata_uids(&db, "acct", "INBOX", 10).unwrap(),
            expected_missing
        );
        let count: i64 = db
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 5, "pruning must retain all live cached rows");
    }
    assert_eq!(commands_after_poison.load(Ordering::SeqCst), 0);
    assert_eq!(
        store::get_recent_page(
            &engine.db.lock().unwrap(),
            "acct",
            "INBOX",
            50,
            None,
            store::RecentFilter {
                unread_only: false,
                attachments_only: true
            }
        )
        .unwrap()
        .0[0]
            .uid,
        2
    );
    server.abort();
}

#[tokio::test]
async fn catchup_leaves_unread_chunks_eligible_at_isolation_limit() {
    let (engine, server, commands_after_poison) =
        metadata_test_server(51, (26..=50).collect()).await;
    let sync = || {
        tokio::time::timeout(
            Duration::from_secs(15),
            sync_messages(&engine, "acct", "INBOX", 50),
        )
    };
    assert_eq!(sync().await.unwrap().unwrap().count, 50);
    let missing =
        store::missing_attachment_metadata_uids(&engine.db.lock().unwrap(), "acct", "INBOX", 100)
            .unwrap();
    assert!(
        missing.contains(&1),
        "healthy UID in an unread chunk must stay eligible"
    );
    assert!(
        missing.len() < 50,
        "isolated poison UIDs should advance progress"
    );
    // Further bounded syncs finish the poison range and reach the healthy file.
    for _ in 0..5 {
        if store::missing_attachment_metadata_uids(&engine.db.lock().unwrap(), "acct", "INBOX", 100)
            .unwrap()
            .is_empty()
        {
            break;
        }
        sync().await.unwrap().unwrap();
    }
    let db = engine.db.lock().unwrap();
    assert!(
        store::missing_attachment_metadata_uids(&db, "acct", "INBOX", 100)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store::get_recent_page(
            &db,
            "acct",
            "INBOX",
            50,
            None,
            store::RecentFilter {
                unread_only: false,
                attachments_only: true
            }
        )
        .unwrap()
        .0[0]
            .uid,
        2
    );
    assert_eq!(
        commands_after_poison.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    server.abort();
}

#[tokio::test]
async fn catchup_timeout_preserves_main_sync_and_completed_progress() {
    let (engine, server, commands_after_failure) =
        metadata_test_server_with_stall(5, vec![4], Some(1)).await;
    // The poison UID is isolated and UID 3 completes before the healthy
    // [1,2] chunk stalls. Both completed rows must survive the deadline.
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        sync_messages_with_policy(&engine, "acct", "INBOX", 4, true, Duration::from_secs(2)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.count, 4);
    assert_eq!(
        store::missing_attachment_metadata_uids(&engine.db.lock().unwrap(), "acct", "INBOX", 10)
            .unwrap(),
        vec![1]
    );
    for _ in 0..2 {
        {
            let db = engine.db.lock().unwrap();
            db.execute("DELETE FROM messages WHERE uid = 5", [])
                .unwrap();
            store::set_folder_state(&db, "acct", "INBOX", 42, 5).unwrap();
        }
        // Repeated catch-up timeouts must not hold new mail or sync state
        // hostage, nor mark cancelled UIDs as attempted.
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            sync_messages_with_policy(
                &engine,
                "acct",
                "INBOX",
                4,
                true,
                Duration::from_millis(150),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.count, 4);
        let db = engine.db.lock().unwrap();
        assert_eq!(
            store::get_folder_state(&db, "acct", "INBOX").unwrap(),
            Some((42, 6))
        );
        let seen: bool = db
            .query_row("SELECT seen FROM messages WHERE uid = 5", [], |r| r.get(0))
            .unwrap();
        assert!(
            seen,
            "new recent headers must be persisted despite catch-up timeout"
        );
        assert_eq!(
            store::missing_attachment_metadata_uids(&db, "acct", "INBOX", 10).unwrap(),
            vec![1]
        );
    }
    assert_eq!(
        commands_after_failure.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    server.abort();
}

#[tokio::test]
async fn post_write_refresh_skips_a_failed_guard_registration() {
    let engine = Arc::new(Engine::new(Box::new(TestHost)).unwrap());
    // Force the registration INSERT to fail before any network request.
    engine
        .db
        .lock()
        .unwrap()
        .execute_batch("CREATE TEMP TABLE active_message_syncs (invalid_column INTEGER);")
        .unwrap();
    assert!(
        refresh_written_folder(&engine, "acct", "Drafts", 20)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn cancelling_a_snapshot_releases_its_removal_markers() {
    let engine = Arc::new(Engine::new(Box::new(TestHost)).unwrap());
    let worker_engine = engine.clone();
    let (ready, started) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let _guard = MessageSyncGuard::begin(&worker_engine).unwrap();
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    started.await.unwrap();
    {
        let db = engine.db.lock().unwrap();
        store::delete_messages_by_uid(&db, "acct", "INBOX", &[7]).unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM removed_message_uids", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let db = engine.db.lock().unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM removed_message_uids", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
}
