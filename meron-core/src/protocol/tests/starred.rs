//! Starred conversation lists returned to mobile clients.
//! Copies in multiple folders should appear as one conversation.
//! Unthreaded messages with equal folder-local UIDs remain distinct.

use super::*;

#[test]
fn mobile_protocol_lists_starred_items_from_store() {
    let data_dir = unique_data_dir("starred-items");
    seed_mobile_account(&data_dir, "a1@example.com");
    seed_mobile_account(&data_dir, "a2@example.com");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "a1@example.com", "INBOX").unwrap();
    store::ensure_folder(&conn, "a2@example.com", "Sent").unwrap();
    store::upsert_messages(
        &conn,
        "a1@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 1,
                subject: "Older starred".to_string(),
                from_name: "Ada".to_string(),
                from_addr: "ada@example.com".to_string(),
                date: 100,
                seen: false,
                starred: true,
                thread_key: "older".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 2,
                subject: "Not starred".to_string(),
                date: 300,
                seen: false,
                starred: false,
                thread_key: "skip".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    store::upsert_messages(
        &conn,
        "a2@example.com",
        "Sent",
        &[MessageHeader {
            uid: 7,
            subject: "Newest starred".to_string(),
            from_name: "Bea".to_string(),
            from_addr: "bea@example.com".to_string(),
            date: 200,
            seen: true,
            starred: true,
            thread_key: "newer".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":74,"method":"mail.starredItems","params":{"limit":2}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(value["id"], 74);
    let items = value["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["account_id"], "a2@example.com");
    assert_eq!(items[0]["folder_id"], "Sent");
    assert_eq!(items[0]["subject"], "Newest starred");
    assert_eq!(
        items[0]["thread_id"],
        "a2@example.com#Sent#t.bmV3ZXIjTmV3ZXN0IHN0YXJyZWQ"
    );
    // Starred rows are thread cards like every other mailbox: the row id *is*
    // the thread id, not a per-message "{thread_id}#{uid}".
    assert_eq!(items[0]["id"], items[0]["thread_id"]);
    assert_eq!(items[0]["unread"], false);
    assert_eq!(items[0]["starred"], true);
    assert_eq!(items[1]["account_id"], "a1@example.com");
    assert_eq!(items[1]["subject"], "Older starred");

    // Filtering happens before the limit is applied, so a newer read row cannot
    // hide an older unread row on the first page.
    let unread = invoke_mobile_protocol_json(
        r#"{"id":75,"method":"mail.starredItems","params":{"filter":"unread","limit":1}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    let unread_items = unread["result"]["items"].as_array().unwrap();
    assert_eq!(unread_items.len(), 1);
    assert_eq!(unread_items[0]["subject"], "Older starred");
    assert!(unread["result"].get("next_cursor").is_none());

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn starred_lists_a_thread_once_when_several_folders_cache_it() {
    // Gmail files every message in All Mail as well as its Inbox/label copy, so
    // the same starred thread is cached under several folders. It is one thread
    // and gets one row, in the most specific folder holding it.
    let data_dir = unique_data_dir("starred-cross-folder");
    seed_mobile_account(&data_dir, "me@example.com");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    store::upsert_folders(
        &conn,
        "me@example.com",
        &[
            Folder {
                name: "INBOX".to_string(),
                ..Default::default()
            },
            Folder {
                name: "Receipts".to_string(),
                ..Default::default()
            },
            Folder {
                name: "[Gmail]/All Mail".to_string(),
                special_use: Some("all".to_string()),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    let header = MessageHeader {
        uid: 5,
        subject: "Your update is live".to_string(),
        from_name: "Google Play Console".to_string(),
        from_addr: "noreply@google.com".to_string(),
        date: 400,
        seen: true,
        starred: true,
        thread_key: "play-update@google.com".to_string(),
        ..Default::default()
    };
    for folder in ["INBOX", "Receipts", "[Gmail]/All Mail"] {
        store::upsert_messages(&conn, "me@example.com", folder, &[header.clone()]).unwrap();
    }
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":76,"method":"mail.starredItems","params":{"limit":10}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    let items = value["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["folder_id"], "INBOX");

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn starred_keeps_unthreaded_messages_that_share_a_uid_apart() {
    // A message with no threading headers falls back to a "uid:<uid>" key, and
    // IMAP UIDs only mean anything inside their own mailbox. Two unrelated
    // starred messages that happen to be uid 1 in different folders are not one
    // conversation and neither may swallow the other.
    let data_dir = unique_data_dir("starred-uid-keys");
    seed_mobile_account(&data_dir, "me@example.com");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    for folder in ["INBOX", "Archive"] {
        store::ensure_folder(&conn, "me@example.com", folder).unwrap();
    }
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[MessageHeader {
            uid: 1,
            subject: "Inbox note".to_string(),
            from_addr: "ana@example.com".to_string(),
            date: 100,
            starred: true,
            ..Default::default()
        }],
    )
    .unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "Archive",
        &[MessageHeader {
            uid: 1,
            subject: "Archived note".to_string(),
            from_addr: "bob@example.com".to_string(),
            date: 200,
            starred: true,
            ..Default::default()
        }],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":77,"method":"mail.starredItems","params":{"limit":10}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    let items = value["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{value}");
    let subjects = items
        .iter()
        .map(|item| item["subject"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(subjects.contains(&"Inbox note"), "{value}");
    assert!(subjects.contains(&"Archived note"), "{value}");

    let _ = std::fs::remove_dir_all(data_dir);
}
