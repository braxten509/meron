//! Message reads from the mobile thread view.
//! Tests cover cached drafts, UID-based threads and unread notifications.
//! Print requests retain uncached entries when the connection is unavailable.

use super::*;

#[test]
fn print_thread_read_keeps_uncached_messages_when_connection_is_unavailable() {
    let data_directory = unique_data_directory("print-thread-offline");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "offline@example.com", "INBOX").unwrap();
    let headers: Vec<_> = (1..=60)
        .map(|uid| MessageHeader {
            uid,
            subject: "Print topic".to_string(),
            thread_key: "topic".to_string(),
            date: uid as i64,
            ..Default::default()
        })
        .collect();
    store::upsert_messages(&conn, "offline@example.com", "INBOX", &headers).unwrap();
    drop(conn);
    // No credentials: every body fetch fails, including pages with no cache.
    let mut cursor = serde_json::Value::Null;
    let mut count = 0;
    for expected in [50, 10] {
        let request = serde_json::json!({
            "id": 1, "method": "mail.threadRead",
            "params": { "thread_id": "offline@example.com#INBOX#t.dG9waWM",
                "limit": 50, "for_print": true, "before_cursor": cursor }
        });
        let response = invoke_mobile_protocol_json(
            &request.to_string(),
            Some(data_directory.to_str().unwrap()),
        );
        assert!(response.get("error").is_none(), "{response}");
        let messages = response["result"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), expected);
        assert!(
            messages
                .iter()
                .all(|message| message["body_missing"] == true)
        );
        count += messages.len();
        cursor = response["result"]["next_cursor"].clone();
    }
    assert_eq!(count, 60);
    assert!(cursor.is_null());
    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_reads_cached_thread_messages_from_store() {
    let data_directory = unique_data_directory("thread-read");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[MessageHeader {
            uid: 9,
            subject: "Cached subject".to_string(),
            from_name: "Ada".to_string(),
            from_addr: "ada@example.com".to_string(),
            date: 300,
            seen: false,
            starred: true,
            thread_key: "topic".to_string(),
            to: vec![Recipient {
                name: "Me".to_string(),
                addr: "me@example.com".to_string(),
            }],
            ..Default::default()
        }],
    )
    .unwrap();
    store::save_cached_message(
        &conn,
        "me@example.com",
        "INBOX",
        9,
        &Message {
            subject: "Cached subject".to_string(),
            from_name: "Ada".to_string(),
            from_addr: "ada@example.com".to_string(),
            to: "Me <me@example.com>".to_string(),
            message_id: "topic".to_string(),
            references: "root".to_string(),
            date: 300,
            body: "Hello from cache".to_string(),
            body_html: Some("<p>Hello from cache</p>".to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":66,"method":"mail.threadRead","params":{"thread_id":"me@example.com#INBOX#t.dG9waWM"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(value["id"], 66);
    let first = &value["result"]["messages"][0];
    assert_eq!(first["id"], "me@example.com#INBOX#9");
    assert_eq!(first["account_id"], "me@example.com");
    assert_eq!(first["folder_id"], "INBOX");
    assert_eq!(first["thread_id"], "me@example.com#INBOX#t.dG9waWM");
    assert_eq!(first["from_name"], "Ada");
    assert_eq!(first["to"], "Me <me@example.com>");
    assert_eq!(first["message_id"], "topic");
    assert_eq!(first["references"], "root");
    assert_eq!(first["subject"], "Cached subject");
    assert_eq!(first["body"], "Hello from cache");
    // Prepared by the core, like the desktop's: sanitized, quote-marked, CSP baked in.
    let body_html = first["body_html"].as_str().unwrap();
    assert!(
        body_html.contains("<body><p>Hello from cache</p></body>"),
        "{body_html}"
    );
    assert!(body_html.contains("Content-Security-Policy"), "{body_html}");
    assert_eq!(first["unread"], true);
    assert_eq!(first["starred"], true);

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_reads_uid_thread_message_from_store() {
    let data_directory = unique_data_directory("uid-thread-read");
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
        r#"{"id":67,"method":"mail.threadRead","params":{"thread_id":"me@example.com#Drafts#4"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(value["id"], 67);
    assert_eq!(value["result"]["messages"][0]["subject"], "Draft");
    assert_eq!(value["result"]["messages"][0]["body"], "Draft body");

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_reports_which_notified_messages_are_still_unread() {
    let data_directory = unique_data_directory("still-unread");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    let header = |uid: u32, seen: bool| MessageHeader {
        uid,
        subject: format!("Mail {uid}"),
        from_addr: "aki@example.com".to_string(),
        date: 100 + i64::from(uid),
        seen,
        ..Default::default()
    };
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[header(1, false), header(2, true), header(3, false)],
    )
    .unwrap();
    store::delete_messages_by_uid(&conn, "me@example.com", "INBOX", &[3]).unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":5,"method":"mail.stillUnread","params":{"messages":[
            {"account_id":"me@example.com","folder_id":"INBOX","message_id":"1"},
            {"account_id":"me@example.com","folder_id":"inbox","message_id":"1"},
            {"account_id":"me@example.com","folder_id":"INBOX","message_id":"2"},
            {"account_id":"me@example.com","folder_id":"INBOX","message_id":"3"},
            {"account_id":"other@example.com","folder_id":"INBOX","message_id":"1"},
            {"account_id":"me@example.com","folder_id":"","message_id":"1"}
        ]}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(
        value["result"]["unread"],
        // The inbox as a `mail.newMessages` detail names it ("inbox") finds the
        // stored "INBOX" row, and comes back named as it was asked.
        json!([
            { "account_id": "me@example.com", "folder_id": "INBOX", "message_id": "1" },
            { "account_id": "me@example.com", "folder_id": "inbox", "message_id": "1" }
        ])
    );

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_thread_read_includes_cached_draft_reply() {
    let data_directory = unique_data_directory("thread-read-draft-reply");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::ensure_folder(&conn, "me@example.com", "Drafts").unwrap();

    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[MessageHeader {
            uid: 7,
            subject: "test mailo2".to_string(),
            from_name: "Gmail".to_string(),
            from_addr: "sender@gmail.com".to_string(),
            date: 100,
            seen: true,
            thread_key: "root@mail.gmail.com".to_string(),
            message_id: "root@mail.gmail.com".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();
    store::save_cached_message(
        &conn,
        "me@example.com",
        "INBOX",
        7,
        &Message {
            subject: "test mailo2".to_string(),
            from_name: "Gmail".to_string(),
            from_addr: "sender@gmail.com".to_string(),
            to: "Me <me@example.com>".to_string(),
            message_id: "root@mail.gmail.com".to_string(),
            date: 100,
            body: "root".to_string(),
            ..Default::default()
        },
    )
    .unwrap();

    store::upsert_messages(
        &conn,
        "me@example.com",
        "Drafts",
        &[MessageHeader {
            uid: 3,
            subject: "Re: test mailo2".to_string(),
            from_name: "Me".to_string(),
            from_addr: "me@example.com".to_string(),
            date: 200,
            seen: true,
            thread_key: "root@mail.gmail.com".to_string(),
            message_id: "draft-reply@mailo.com".to_string(),
            in_reply_to: "root@mail.gmail.com".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();
    store::save_cached_message(
        &conn,
        "me@example.com",
        "Drafts",
        3,
        &Message {
            subject: "Re: test mailo2".to_string(),
            from_name: "Me".to_string(),
            from_addr: "me@example.com".to_string(),
            to: "sender@gmail.com".to_string(),
            message_id: "draft-reply@mailo.com".to_string(),
            references: "root@mail.gmail.com".to_string(),
            date: 200,
            body: "draft reply".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    drop(conn);

    let read = invoke_mobile_protocol_json(
        r#"{"id":69,"method":"mail.threadRead","params":{"thread_id":"me@example.com#INBOX#t.cm9vdEBtYWlsLmdtYWlsLmNvbQ"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let messages = read["result"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2, "{read}");
    assert_eq!(messages[0]["message_id"], "root@mail.gmail.com");
    assert_eq!(messages[1]["message_id"], "draft-reply@mailo.com");
    assert_eq!(messages[1]["in_reply_to"], "root@mail.gmail.com");
    assert_eq!(messages[1]["folder_id"], "Drafts");
    assert_eq!(messages[1]["body"], "draft reply");

    let _ = std::fs::remove_dir_all(data_directory);
}
