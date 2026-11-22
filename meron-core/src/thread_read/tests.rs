//! Regression tests for thread read.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::{
    Slot, message_key, messages_missing_media, parse_thread_cursor, thread_cursor,
    thread_cursor_position, thread_message_json,
};
use crate::imap::MessageHeader;
use crate::parse::{Attachment, Message, missing_media_count};
use std::collections::HashSet;

fn header(uid: u32, seen: bool) -> MessageHeader {
    MessageHeader {
        uid,
        subject: "Hi".to_string(),
        from_addr: "ann@x.com".to_string(),
        seen,
        ..Default::default()
    }
}

struct Host;
impl crate::engine::EngineHost for Host {
    fn open_db(&self) -> anyhow::Result<rusqlite::Connection> {
        crate::store::open_at(":memory:")
    }
    fn apply_secret(&self, _: &rusqlite::Connection, _: &str, _: &mut crate::imap::Creds) {}
    fn store_secret(
        &self,
        _: &rusqlite::Connection,
        _: &str,
        _: &crate::secrets::Secrets,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn failed_fill_notifies_a_pending_reader_and_surfaces_its_error_once() {
    let (engine, server) =
        crate::engine::slow_fetch_engine(1, std::time::Duration::from_millis(150), true).await;
    let (notify, notified) = tokio::sync::oneshot::channel();
    let notify = std::sync::Mutex::new(Some(notify));
    super::spawn_fill_thread_bodies(
        &engine,
        "acc",
        vec![("INBOX".into(), 1)],
        crate::parse::test_media_root(),
        Some(Box::new(move || {
            if let Some(notify) = notify.lock().unwrap().take() {
                let _ = notify.send(());
            }
        })),
    );
    let read = || {
        super::read_thread_page(
            &engine,
            super::ThreadReadArgs {
                account: "acc",
                folder: "INBOX",
                thread_id: "acc#INBOX#uid:1",
                thread_key: "uid:1",
                subject_filter: None,
                limit: Some(20),
                for_print: false,
                before_cursor: None,
                media_root: crate::parse::test_media_root(),
            },
            None,
        )
    };
    let page = read().await.unwrap();
    assert_eq!(page["messages"][0]["body_missing"], true);
    // Still downloading: a placeholder, not a failure to retry.
    assert_eq!(page["messages"][0]["body_loading"], true);
    tokio::time::timeout(std::time::Duration::from_secs(3), notified)
        .await
        .unwrap()
        .unwrap();
    assert!(engine.body_fetches.lock().unwrap().is_empty());
    let error = read().await.unwrap_err();
    assert!(format!("{error:#}").contains("UID FETCH"), "{error:#}");
    assert!(engine.body_fetches.lock().unwrap().is_empty());
    assert!(engine.body_fetch_failures.lock().unwrap().is_empty());
    server.await.unwrap();
}

#[tokio::test]
async fn failed_fill_keeps_other_cached_messages_visible_without_an_automatic_retry() {
    let engine = std::sync::Arc::new(crate::engine::Engine::new(Box::new(Host)).unwrap());
    let headers: Vec<_> = [1, 2, 3]
        .into_iter()
        .map(|uid| MessageHeader {
            thread_key: "same-thread".into(),
            date: uid as i64,
            ..header(uid, true)
        })
        .collect();
    {
        let db = engine.db.lock().unwrap();
        crate::store::upsert_messages(&db, "acc", "INBOX", &headers).unwrap();
        crate::store::save_cached_message(
            &db,
            "acc",
            "INBOX",
            1,
            &Message {
                message_id: "cached@test".into(),
                body: "Cached body".into(),
                ..Default::default()
            },
        )
        .unwrap();
    }
    // One fill failed for both the older and the newest uncached message.
    for uid in [2, 3] {
        engine.record_body_failure(message_key("acc", "INBOX", uid), "download failed".into());
    }
    let page = super::read_thread_page(
        &engine,
        super::ThreadReadArgs {
            account: "acc",
            folder: "INBOX",
            thread_id: "acc#INBOX#same-thread",
            thread_key: "same-thread",
            subject_filter: None,
            limit: Some(20),
            for_print: false,
            before_cursor: None,
            media_root: crate::parse::test_media_root(),
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 3);
    assert_eq!(page["messages"][0]["body"], "Cached body");
    assert_eq!(page["messages"][1]["body_missing"], true);
    assert_eq!(page["messages"][2]["body_missing"], true);
    // Failed, not on the way.
    assert!(page["messages"][1].get("body_loading").is_none());
    assert!(page["messages"][2].get("body_loading").is_none());
    // Neither is downloaded again by the read its failure announced.
    assert!(engine.body_fetches.lock().unwrap().is_empty());
    assert!(engine.body_fetch_failures.lock().unwrap().is_empty());
}

#[tokio::test]
async fn paged_read_shows_first_body_when_fetch_completion_fails() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
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
                        format!(
                            "* 1 FETCH (UID 1 BODY[] {{{}}}\r\n{body})\r\n{tag} NO completion failed\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                return;
            }
            if command.starts_with("SELECT") {
                writer.write_all(b"* 1 EXISTS\r\n").await.unwrap();
            }
            writer
                .write_all(format!("{tag} OK done\r\n").as_bytes())
                .await
                .unwrap();
        }
    });
    let engine = std::sync::Arc::new(crate::engine::Engine::new(Box::new(Host)).unwrap());
    let creds = {
        let db = engine.db.lock().unwrap();
        let config = serde_json::json!({
            "host": "127.0.0.1", "port": port, "tls": false,
            "user": "u", "proxy": {"mode": "direct"}
        })
        .to_string();
        db.execute(
            "INSERT INTO accounts(id, config) VALUES('acc', ?1)",
            [&config],
        )
        .unwrap();
        crate::store::upsert_messages(&db, "acc", "INBOX", &[header(1, true)]).unwrap();
        crate::store::load_account(&db, "acc").unwrap().unwrap()
    };
    engine.accounts.lock().await.insert("acc".into(), creds);
    let page = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        super::read_thread_page(
            &engine,
            super::ThreadReadArgs {
                account: "acc",
                folder: "INBOX",
                thread_id: "acc#INBOX#uid:1",
                thread_key: "uid:1",
                subject_filter: None,
                limit: Some(20),
                for_print: false,
                before_cursor: None,
                media_root: crate::parse::test_media_root(),
            },
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page["messages"][0]["body_missing"], false);
    assert_eq!(
        page["messages"][0]["body"].as_str().unwrap().trim(),
        "First body"
    );
    assert!(engine.body_fetches.lock().unwrap().is_empty());
    assert!(crate::store::has_cached_body(&engine.db.lock().unwrap(), "acc", "INBOX", 1).unwrap());
    server.await.unwrap();
}

#[tokio::test]
async fn paged_read_does_not_restart_a_body_already_filling_in_background() {
    let engine = std::sync::Arc::new(crate::engine::Engine::new(Box::new(Host)).unwrap());
    crate::store::upsert_messages(
        &engine.db.lock().unwrap(),
        "acc",
        "INBOX",
        &[header(1, true)],
    )
    .unwrap();
    engine
        .body_fetches
        .lock()
        .unwrap()
        .insert(message_key("acc", "INBOX", 1));
    // No credentials are configured: an accidental inline attempt fails.
    let page = super::read_thread_page(
        &engine,
        super::ThreadReadArgs {
            account: "acc",
            folder: "INBOX",
            thread_id: "acc#INBOX#uid:1",
            thread_key: "uid:1",
            subject_filter: None,
            limit: Some(20),
            for_print: false,
            before_cursor: None,
            media_root: crate::parse::test_media_root(),
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page["messages"][0]["body_missing"], true);
    assert!(
        engine
            .body_fetches
            .lock()
            .unwrap()
            .contains(&message_key("acc", "INBOX", 1))
    );
}

#[tokio::test]
async fn an_active_fill_does_not_block_a_new_message_in_the_same_thread() {
    let engine = std::sync::Arc::new(crate::engine::Engine::new(Box::new(Host)).unwrap());
    let headers: Vec<_> = [1, 2]
        .into_iter()
        .map(|uid| MessageHeader {
            thread_key: "same-thread".into(),
            date: uid as i64,
            ..header(uid, true)
        })
        .collect();
    crate::store::upsert_messages(&engine.db.lock().unwrap(), "acc", "INBOX", &headers).unwrap();
    let older = message_key("acc", "INBOX", 1);
    let newest = message_key("acc", "INBOX", 2);
    engine.body_fetches.lock().unwrap().insert(older.clone());
    // The missing credentials prove that the new UID gets an inline attempt
    // rather than being silently skipped because an older UID is filling.
    let error = super::read_thread_page(
        &engine,
        super::ThreadReadArgs {
            account: "acc",
            folder: "INBOX",
            thread_id: "acc#INBOX#same-thread",
            thread_key: "same-thread",
            subject_filter: None,
            limit: Some(1),
            for_print: false,
            before_cursor: None,
            media_root: crate::parse::test_media_root(),
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("account needs reconnect"),
        "{error:#}"
    );
    let filling = engine.body_fetches.lock().unwrap();
    assert!(filling.contains(&older));
    // Its longer background retry is claimed independently of the old fill.
    assert!(filling.contains(&newest));
}

#[test]
fn successful_fetch_omissions_are_held_until_restart_but_failures_expire() {
    let engine = crate::engine::Engine::new(Box::new(Host)).unwrap();
    let asked = std::collections::BTreeMap::from([("INBOX".to_string(), vec![1, 2])]);
    let attempted = HashSet::from(["INBOX".to_string()]);
    let root = crate::parse::test_media_root();
    let cached = Message {
        message_id: "cached-1".into(),
        body: "Cached text".into(),
        ..Default::default()
    };
    crate::store::save_cached_message(&engine.db.lock().unwrap(), "acc", "INBOX", 1, &cached)
        .unwrap();
    super::store_thread_bodies(&engine, "acc", &asked, vec![], &attempted, false, &root);
    let key = message_key("acc", "INBOX", 1);
    assert!(engine.media_recovery_hold.lock().unwrap()[&key].is_some());
    let slots = (0..3)
        .map(|index| Slot {
            folder: "INBOX".into(),
            cached: if index == 0 {
                Some(Message {
                    message_id: "cached-1".into(),
                    ..Default::default()
                })
            } else {
                None
            },
            complete: false,
            media_missing: 0,
        })
        .collect::<Vec<_>>();
    let headers = [header(1, true), header(2, true), header(3, true)];
    let pending = || {
        super::pending_body_recovery(
            &engine.media_recovery_hold.lock().unwrap(),
            "acc",
            &slots,
            &headers,
            &[0, 1, 2],
        )
    };
    assert_eq!(pending(), vec![("INBOX".into(), 2), ("INBOX".into(), 3)]);
    assert!(
        !engine
            .media_recovery_hold
            .lock()
            .unwrap()
            .contains_key(&message_key("acc", "INBOX", 2))
    );
    // A later successful FETCH omitting the same UID proves it is gone.
    super::store_thread_bodies(&engine, "acc", &asked, vec![], &attempted, true, &root);
    assert_eq!(engine.media_recovery_hold.lock().unwrap()[&key], None);
    assert_eq!(pending(), vec![("INBOX".into(), 2), ("INBOX".into(), 3)]);
    assert!(
        !engine
            .media_recovery_hold
            .lock()
            .unwrap()
            .contains_key(&message_key("acc", "INBOX", 2))
    );
    // Even a leftover permanent media hold cannot block an uncached body.
    engine
        .media_recovery_hold
        .lock()
        .unwrap()
        .insert(message_key("acc", "INBOX", 2), None);
    assert_eq!(pending(), vec![("INBOX".into(), 2), ("INBOX".into(), 3)]);
    // A subsequent connection failure must not shorten that permanent hold.
    super::store_thread_bodies(&engine, "acc", &asked, vec![], &attempted, false, &root);
    assert_eq!(engine.media_recovery_hold.lock().unwrap()[&key], None);
}

#[test]
fn an_old_body_failure_does_not_stand_in_for_a_new_attempt() {
    let engine = crate::engine::Engine::new(Box::new(Host)).unwrap();
    let key = message_key("acc", "INBOX", 1);
    engine.record_body_failure(key.clone(), "download failed".into());
    assert_eq!(
        engine.take_body_failure(&key).as_deref(),
        Some("download failed")
    );
    assert_eq!(engine.take_body_failure(&key), None);
    if let Some(long_ago) =
        std::time::Instant::now().checked_sub(std::time::Duration::from_secs(3600))
    {
        engine
            .body_fetch_failures
            .lock()
            .unwrap()
            .insert(key.clone(), ("download failed".into(), long_ago));
        assert_eq!(engine.take_body_failure(&key), None);
    }
}

#[test]
fn media_recovery_only_refetches_referenced_files() {
    let root = std::env::temp_dir().join(format!("meron-media-recovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("acc/INBOX/3")).unwrap();
    std::fs::write(root.join("acc/INBOX/3/0.png"), [1]).unwrap();
    let slot = |key: Option<&str>| {
        let mut message = Message::default();
        message.attachments.push(Attachment {
            filename: "a.png".to_string(),
            mime: "image/png".to_string(),
            size: 1,
            key: key.map(str::to_string),
            oversized: false,
        });
        Slot {
            folder: "INBOX".to_string(),
            media_missing: missing_media_count(&root, &message),
            cached: Some(message),
            complete: true,
        }
    };
    // Pruned, key-less, and present.
    let slots = [
        slot(Some("acc/INBOX/1/0.png")),
        slot(None),
        slot(Some("acc/INBOX/3/0.png")),
    ];
    let headers = [header(1, true), header(2, true), header(3, true)];

    let mut hold = super::MediaRecoveryHold::new();
    assert_eq!(
        messages_missing_media(&hold, "acc", &slots, &headers),
        vec![("INBOX".to_string(), 1), ("INBOX".to_string(), 2)]
    );

    // A refetch that did not bring uid 2's file back is not repeated.
    hold.insert(message_key("acc", "INBOX", 2), None);
    assert_eq!(
        messages_missing_media(&hold, "acc", &slots, &headers),
        vec![("INBOX".to_string(), 1)]
    );

    // A download that failed holds uid 1 for a while, not for good.
    let now = std::time::Instant::now();
    let key = message_key("acc", "INBOX", 1);
    hold.insert(key.clone(), Some(now + std::time::Duration::from_secs(60)));
    assert!(messages_missing_media(&hold, "acc", &slots, &headers).is_empty());
    hold.insert(key, now.checked_sub(std::time::Duration::from_secs(1)));
    assert_eq!(
        messages_missing_media(&hold, "acc", &slots, &headers),
        vec![("INBOX".to_string(), 1)]
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn shapes_ids_and_flags_from_header() {
    let value = thread_message_json(
        "acc",
        "tid",
        "INBOX",
        &header(10, false),
        None,
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(value["id"], "acc#INBOX#10");
    assert_eq!(value["thread_id"], "tid");
    assert_eq!(value["folder_id"], "INBOX");
    assert_eq!(value["unread"], true);
    assert_eq!(value["subject"], "Hi");
    // The reply rule travels with the message, even before its body caches.
    assert_eq!(value["reply"]["to"], "ann@x.com");
    assert_eq!(value["reply"]["all_adds_recipients"], false);
}

#[test]
fn missing_body_is_flagged_not_dropped() {
    let value = thread_message_json(
        "acc",
        "tid",
        "Sent",
        &header(11, true),
        None,
        &HashSet::new(),
        &HashSet::new(),
    );
    assert_eq!(value["body_missing"], true);
    assert_eq!(value["body"], "");
    assert_eq!(value["unread"], false);
}

#[test]
fn equal_uids_in_different_folders_have_distinct_ids() {
    let inbox = thread_message_json(
        "acc",
        "tid",
        "INBOX",
        &header(10, false),
        None,
        &HashSet::new(),
        &HashSet::new(),
    );
    let sent = thread_message_json(
        "acc",
        "tid",
        "Sent",
        &header(10, true),
        None,
        &HashSet::new(),
        &HashSet::new(),
    );

    assert_eq!(inbox["id"], "acc#INBOX#10");
    assert_eq!(sent["id"], "acc#Sent#10");
    assert_ne!(inbox["id"], sent["id"]);
    assert_eq!(inbox["thread_id"], sent["thread_id"]);
}

#[test]
fn cursor_distinguishes_equal_uids_in_different_folders() {
    let inbox = MessageHeader {
        uid: 7,
        folder: "INBOX".into(),
        ..Default::default()
    };
    let sent = MessageHeader {
        uid: 7,
        folder: "Sent".into(),
        ..Default::default()
    };

    assert_ne!(thread_cursor(&inbox), thread_cursor(&sent));
    assert_eq!(
        parse_thread_cursor(&thread_cursor(&sent)),
        Some((Some("Sent".into()), 7))
    );
    assert_eq!(parse_thread_cursor("uid:7"), Some((None, 7)));

    let headers = vec![
        inbox,
        MessageHeader {
            uid: 9,
            folder: "Archive".into(),
            ..Default::default()
        },
        sent,
    ];
    let cursor = parse_thread_cursor(&thread_cursor(&headers[2])).unwrap();
    assert_eq!(thread_cursor_position(&headers, &cursor), Some(2));
}
