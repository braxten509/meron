//! Exercises protocol behavior for flags.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn mobile_protocol_flag_actions_leave_the_store_alone_when_the_server_write_fails() {
    let data_dir = unique_data_dir("thread-actions");
    seed_mobile_account(&data_dir, "me@example.com");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 10,
                subject: "First".to_string(),
                seen: false,
                starred: false,
                thread_key: "topic".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 11,
                subject: "Second".to_string(),
                seen: false,
                starred: false,
                thread_key: "topic".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    drop(conn);

    let read = invoke_mobile_protocol_json(
        r#"{"id":68,"method":"mail.markRead","params":{"thread_id":"me@example.com#INBOX#t.dG9waWM","seen":true}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(read["id"], 68);
    assert!(
        read["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let starred = invoke_mobile_protocol_json(
        r#"{"id":69,"method":"mail.markStarred","params":{"thread_id":"me@example.com#INBOX#t.dG9waWM","starred":true}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(starred["id"], 69);
    assert!(
        starred["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let mark_all = invoke_mobile_protocol_json(
        r#"{"id":72,"method":"mail.markAllRead","params":{"account_id":"me@example.com","folder_id":"INBOX"}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(mark_all["id"], 72);
    assert!(
        mark_all["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    // Emptying is gated on the folder role, before any account/network work:
    // INBOX is refused outright, while Trash gets as far as the creds check.
    let empty_inbox = invoke_mobile_protocol_json(
        r#"{"id":73,"method":"mail.emptyFolder","params":{"account_id":"me@example.com","folder_id":"INBOX"}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(empty_inbox["id"], 73);
    assert!(
        empty_inbox["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Only Trash and Junk folders can be emptied")
    );

    let empty_trash = invoke_mobile_protocol_json(
        r#"{"id":74,"method":"mail.emptyFolder","params":{"account_id":"me@example.com","folder_id":"Trash"}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(empty_trash["id"], 74);
    assert!(
        empty_trash["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let threads = invoke_mobile_protocol_json(
        r#"{"id":70,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"INBOX","filter":"all"}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    let first = &threads["result"]["threads"][0];
    assert_eq!(
        first["thread_id"],
        "me@example.com#INBOX#t.dG9waWMjU2Vjb25k"
    );
    // None of the writes reached the server, so none reached the store.
    assert_eq!(first["unread"], true);
    assert_eq!(first["unread_count"], 1);
    assert_eq!(first["starred"], false);

    let unread = invoke_mobile_protocol_json(
        r#"{"id":71,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"INBOX","filter":"unread"}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(unread["result"]["threads"].as_array().unwrap().len(), 2);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_keeps_uid_thread_unread_when_server_write_fails() {
    let data_dir = unique_data_dir("uid-thread-actions");
    seed_mobile_account(&data_dir, "me@example.com");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
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
        r#"{"id":72,"method":"mail.markRead","params":{"thread_id":"me@example.com#Drafts#4","seen":true}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let read = invoke_mobile_protocol_json(
        r#"{"id":73,"method":"mail.threadRead","params":{"thread_id":"me@example.com#Drafts#4"}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    // The server never took the change, so the store must not claim it did.
    assert_eq!(read["result"]["messages"][0]["unread"], true);

    let _ = std::fs::remove_dir_all(data_dir);
}
