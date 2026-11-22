//! Move, copy, delete and archive commands on mobile.
//! The cases cover missing credentials and archive-folder discovery.
//! An explicit message selection must survive the move request.

use super::*;

#[test]
fn mobile_protocol_move_requires_usable_account_secret() {
    let data_directory = unique_data_directory("move-needs-secret");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[MessageHeader {
            uid: 7,
            subject: "Move me".to_string(),
            thread_key: "topic".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":67,"method":"mail.move","params":{"thread_id":"me@example.com#INBOX#t.dG9waWM","target_folder_id":"Work"}}"#,
        Some(data_directory.to_str().unwrap()),
    );

    assert_eq!(value["id"], 67);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    // INBOX in another account must not be mistaken for the source INBOX.
    let cross_account = invoke_mobile_protocol_json(
        r#"{"id":69,"method":"mail.move","params":{"thread_id":"me@example.com#INBOX#t.dG9waWM","target_account_id":"other@example.com","target_folder_id":"INBOX"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert!(
        cross_account["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_copy_requires_usable_account_secret() {
    let data_directory = unique_data_directory("copy-needs-secret");
    seed_mobile_account(&data_directory, "me@example.com");
    seed_mobile_account(&data_directory, "target@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::ensure_folder(&conn, "target@example.com", "Archive").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[MessageHeader {
            uid: 7,
            subject: "Copy me".to_string(),
            thread_key: "topic".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":68,"method":"mail.copy","params":{"thread_id":"me@example.com#INBOX#t.dG9waWM","target_account_id":"target@example.com","target_folder_id":"Archive"}}"#,
        Some(data_directory.to_str().unwrap()),
    );

    assert_eq!(value["id"], 68);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_delete_requires_server_credentials() {
    let data_directory = unique_data_directory("thread-delete");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 20,
                subject: "Delete me".to_string(),
                date: fresh_date() + 300,
                thread_key: "topic".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 21,
                subject: "Delete me too".to_string(),
                date: fresh_date() + 200,
                thread_key: "topic".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 22,
                subject: "Keep me".to_string(),
                date: fresh_date() + 100,
                thread_key: "other".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":75,"method":"mail.delete","params":{"thread_id":"me@example.com#INBOX#t.dG9waWM"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(value["id"], 75);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let threads = invoke_mobile_protocol_json(
        r#"{"id":76,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"INBOX","filter":"all"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let rows = threads["result"]["threads"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["subject"], "Delete me");
    assert_eq!(rows[1]["subject"], "Delete me");
    assert_eq!(rows[2]["subject"], "Keep me");

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_delete_uid_thread_requires_server_credentials() {
    let data_directory = unique_data_directory("uid-thread-delete");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::save_cached_message(
        &conn,
        "me@example.com",
        "Drafts",
        4,
        &Message {
            subject: "Draft".to_string(),
            from_name: "Me".to_string(),
            from_addr: "me@example.com".to_string(),
            date: 400,
            body: "Draft body".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":77,"method":"mail.delete","params":{"thread_id":"me@example.com#Drafts#4"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let read = invoke_mobile_protocol_json(
        r#"{"id":78,"method":"mail.threadRead","params":{"thread_id":"me@example.com#Drafts#4"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(read["result"]["messages"].as_array().unwrap().len(), 1);

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_archive_requires_server_credentials() {
    let data_directory = unique_data_directory("thread-archive");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::upsert_folders(
        &conn,
        "me@example.com",
        &[
            Folder {
                name: "INBOX".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            Folder {
                name: "Archive".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 31,
                subject: "Archive me".to_string(),
                date: fresh_date() + 300,
                thread_key: "topic".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 32,
                subject: "Archive me too".to_string(),
                date: fresh_date() + 200,
                thread_key: "topic".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 33,
                subject: "Keep me".to_string(),
                date: fresh_date() + 100,
                thread_key: "other".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":79,"method":"mail.archive","params":{"thread_id":"me@example.com#INBOX#t.dG9waWM"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(value["id"], 79);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let inbox = invoke_mobile_protocol_json(
        r#"{"id":80,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"INBOX","filter":"all"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let inbox_rows = inbox["result"]["threads"].as_array().unwrap();
    assert_eq!(inbox_rows.len(), 3);
    assert_eq!(inbox_rows[0]["subject"], "Archive me");
    assert_eq!(inbox_rows[1]["subject"], "Archive me");
    assert_eq!(inbox_rows[2]["subject"], "Keep me");

    let archive = invoke_mobile_protocol_json(
        r#"{"id":81,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"Archive","filter":"all"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let archive_rows = archive["result"]["threads"].as_array().unwrap();
    assert_eq!(archive_rows.len(), 0);

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_archive_requires_cached_archive_folder() {
    let data_directory = unique_data_directory("thread-archive-missing-folder");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[MessageHeader {
            uid: 41,
            subject: "Archive me".to_string(),
            thread_key: "topic".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();
    drop(conn);

    // A single email: a whole conversation is filed into the built-in Archive
    // instead when the account has no archive mailbox (see
    // `meron_folders::archive_meron_only`).
    let value = invoke_mobile_protocol_json(
        r#"{"id":82,"method":"mail.archive","params":{"thread_id":"me@example.com#INBOX#41"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(value["id"], 82);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Archive folder not found")
    );

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_move_honors_explicit_message_ids() {
    // An undo moves back the copies an archive reported, by UID. Those rows
    // need not be cached in the archive folder, and a uid: thread key names
    // nothing there at all — so the explicit ids must be used as given rather
    // than resolved through the cached thread.
    let data_directory = unique_data_directory("move-explicit-ids");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "Archive").unwrap();
    drop(conn);

    let without_ids = invoke_mobile_protocol_json(
        r#"{"id":68,"method":"mail.move","params":{"thread_id":"me@example.com#Archive#t.dG9waWM","target_folder_id":"INBOX"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(without_ids["result"]["moved"], 0);

    // Past UID resolution to the server write, which this account can't make.
    let with_ids = invoke_mobile_protocol_json(
        r#"{"id":69,"method":"mail.move","params":{"thread_id":"me@example.com#Archive#t.dG9waWM","target_folder_id":"INBOX","message_ids":["42"]}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert!(
        with_ids["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect"),
        "{with_ids}"
    );

    let _ = std::fs::remove_dir_all(data_directory);
}
