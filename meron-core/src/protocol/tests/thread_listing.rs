//! Single-account and unified conversation lists.
//! Participants, folder roles and account preferences determine eligible rows.
//! Cases cover Gmail thread identity and sent replies inheriting a parent.

use super::*;

#[test]
fn mobile_protocol_lists_threads_from_store() {
    let data_directory = unique_data_directory("threads");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 7,
                subject: "Newest".to_string(),
                from_name: "Ada".to_string(),
                from_addr: "ada@example.com".to_string(),
                date: fresh_date() + 200,
                seen: false,
                starred: true,
                thread_key: "topic".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 3,
                subject: "Older".to_string(),
                from_name: "Bea".to_string(),
                from_addr: "bea@example.com".to_string(),
                date: fresh_date() + 100,
                seen: true,
                starred: false,
                thread_key: "older-topic".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":64,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"inbox","filter":"all"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(value["id"], 64);
    assert_eq!(value["result"]["folder_unread"], 1);
    assert_eq!(value["result"]["folder_synced"], false);
    let first = &value["result"]["threads"][0];
    assert_eq!(first["account_id"], "me@example.com");
    assert_eq!(first["folder_id"], "INBOX");
    assert_eq!(first["subject"], "Newest");
    assert_eq!(first["from_name"], "Ada");
    assert_eq!(first["unread"], true);
    assert_eq!(first["unread_count"], 1);
    assert_eq!(first["starred"], true);
    assert_eq!(
        first["thread_id"],
        "me@example.com#INBOX#t.dG9waWMjTmV3ZXN0"
    );

    let unread = invoke_mobile_protocol_json(
        r#"{"id":65,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"inbox","filter":"unread"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(unread["result"]["threads"].as_array().unwrap().len(), 1);

    let starred = invoke_mobile_protocol_json(
        r#"{"id":66,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"inbox","filter":"starred"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let starred_threads = starred["result"]["threads"].as_array().unwrap();
    assert_eq!(starred_threads.len(), 1);
    assert_eq!(starred_threads[0]["subject"], "Newest");

    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::set_folder_state(&conn, "me@example.com", "INBOX", 1, 8).unwrap();
    drop(conn);
    let synced = invoke_mobile_protocol_json(
        r#"{"id":67,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"inbox","filter":"all"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(synced["result"]["folder_synced"], true);

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_contact_threads_pages_exact_matches_across_accounts() {
    let data_directory = unique_data_directory("contact-threads");
    seed_mobile_account(&data_directory, "me@example.com");
    seed_mobile_account(&data_directory, "other@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    for (account, folder, uid, addr, date) in [
        ("me@example.com", "INBOX", 1, "Alice@example.com", 300),
        ("other@example.com", "Archive", 2, "alice@example.com", 200),
        ("me@example.com", "INBOX", 3, "notalice@example.com", 400),
        ("me@example.com", "Spam", 4, "alice@example.com", 900),
        ("other@example.com", "Trash", 5, "alice@example.com", 1000),
    ] {
        store::upsert_messages(
            &conn,
            account,
            folder,
            &[MessageHeader {
                uid,
                subject: format!("Topic {uid}"),
                from_name: "Alice".to_string(),
                from_addr: addr.to_string(),
                date: fresh_date() + date,
                thread_key: format!("mid:{uid}"),
                starred: true,
                ..Default::default()
            }],
        )
        .unwrap();
    }
    drop(conn);
    let invoke = |method: &str, params: Value| {
        invoke_mobile_protocol_json(
            &json!({"id": 1, "method": method, "params": params}).to_string(),
            Some(data_directory.to_str().unwrap()),
        )
    };
    let first = invoke(
        "mail.contactThreads",
        json!({"addr": " alice@example.com ", "limit": 1}),
    );
    let cards = first["result"]["threads"].as_array().unwrap();
    assert_eq!(cards.len(), 1, "{first}");
    assert_eq!(cards[0]["account_id"], "me@example.com");
    assert_eq!(cards[0]["folder_id"], "INBOX");
    assert_eq!(cards[0]["id"], cards[0]["thread_id"]);
    assert_eq!(cards[0]["unread"], true);
    assert_eq!(cards[0]["starred"], true);
    let mailbox = invoke(
        "mail.threadList",
        json!({"account_id": "me@example.com", "folder_id": "INBOX"}),
    );
    assert!(
        mailbox["result"]["threads"]
            .as_array()
            .unwrap()
            .contains(&cards[0])
    );
    let second = invoke(
        "contacts.threads",
        json!({
            "addr": "alice@example.com", "limit": 1,
            "before_cursor": first["result"]["next_cursor"],
        }),
    );
    assert_eq!(
        second["result"]["threads"][0]["account_id"],
        "other@example.com"
    );
    assert_eq!(second["result"]["threads"][0]["folder_id"], "Archive");
    assert!(second["result"]["next_cursor"].is_null());
    assert!(
        invoke("contacts.threads", json!({"addr": "nobody@example.com"}))["result"]["threads"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for params in [
        json!({}),
        json!({"addr": "  "}),
        json!({"addr": "alice@example.com", "before_cursor": "invalid"}),
    ] {
        assert!(invoke("mail.contactThreads", params)["error"]["message"].is_string());
    }
    let clamped = invoke(
        "contacts.threads",
        json!({"addr": "alice@example.com", "limit": 0}),
    );
    assert_eq!(clamped["result"]["threads"].as_array().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_lists_unified_threads_and_excludes_opted_out_accounts() {
    let data_directory = unique_data_directory("unified-threads");
    for account in [
        "first@example.com",
        "second@example.com",
        "excluded@example.com",
    ] {
        seed_mobile_account(&data_directory, account);
    }
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::set_account_pref(&conn, "excluded@example.com", "included_in_unified", false).unwrap();
    for (account, uid, date, unread) in [
        ("first@example.com", 7, 100, true),
        ("second@example.com", 9, 200, true),
        ("excluded@example.com", 11, 300, true),
    ] {
        store::ensure_folder(&conn, account, "INBOX").unwrap();
        store::upsert_messages(
            &conn,
            account,
            "INBOX",
            &[MessageHeader {
                uid,
                subject: format!("Message from {account}"),
                from_addr: "sender@example.com".to_string(),
                date: fresh_date() + date,
                seen: !unread,
                thread_key: format!("topic-{uid}"),
                ..Default::default()
            }],
        )
        .unwrap();
    }
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":164,"method":"mail.threadList","params":{"account_id":"unified","folder_id":"inbox","filter":"all"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let threads = value["result"]["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 2, "{value}");
    assert_eq!(threads[0]["account_id"], "second@example.com", "{value}");
    assert_eq!(threads[1]["account_id"], "first@example.com", "{value}");
    assert_eq!(value["result"]["folder_unread"], 2, "{value}");
    assert_eq!(value["result"]["folder_unreads"]["first@example.com"], 1);
    assert_eq!(value["result"]["folder_unreads"]["second@example.com"], 1);
    assert_eq!(value["result"]["failures"], json!([]));

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn starred_rows_group_into_threads_across_folders() {
    let data_directory = unique_data_directory("starred-threads");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    for folder in ["INBOX", "Archive"] {
        store::ensure_folder(&conn, "me@example.com", folder).unwrap();
    }
    // Two starred messages of one thread in INBOX collapse to a single card...
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 1,
                subject: "Trip plans".to_string(),
                from_addr: "ana@example.com".to_string(),
                date: 100,
                seen: true,
                starred: true,
                thread_key: "trip".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 2,
                subject: "Re: Trip plans".to_string(),
                from_addr: "ana@example.com".to_string(),
                date: 150,
                seen: true,
                starred: true,
                thread_key: "trip".to_string(),
                ..Default::default()
            },
            // The card is admitted by the stars above, but its ordinary thread
            // metadata must still come from this newer unread reply.
            MessageHeader {
                uid: 3,
                subject: "Re: Trip plans".to_string(),
                from_addr: "bob@example.com".to_string(),
                date: 200,
                thread_key: "trip".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    // ...and so does the same thread starred in Archive: it is one conversation
    // to the reader however many folders the server files it under, and it is
    // listed in the more specific of them.
    store::upsert_messages(
        &conn,
        "me@example.com",
        "Archive",
        &[MessageHeader {
            uid: 5,
            subject: "Trip plans".to_string(),
            from_addr: "ana@example.com".to_string(),
            date: 120,
            starred: true,
            thread_key: "trip".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":75,"method":"mail.starredItems","params":{"limit":10}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let items = value["result"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{value}");
    let inbox = &items[0];
    assert_eq!(inbox["id"], inbox["thread_id"], "{value}");
    assert_eq!(inbox["folder_id"], "INBOX", "{value}");
    assert_eq!(inbox["date"], 200, "{value}");
    assert_eq!(inbox["unread"], true, "{value}");
    assert_eq!(inbox["unread_count"], 1, "{value}");

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn unified_folder_role_resolves_per_account_and_skips_accounts_without_it() {
    let data_directory = unique_data_directory("unified-role");
    for account in ["has-sent@example.com", "no-sent@example.com"] {
        seed_mobile_account(&data_directory, account);
    }
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    // One account files sent mail under a non-obvious name flagged \Sent; the
    // other has no sent mailbox at all and must drop out of the merge.
    store::upsert_folders(
        &conn,
        "has-sent@example.com",
        &[crate::imap::Folder {
            name: "Postausgang".to_string(),
            special_use: Some("sent".to_string()),
            ..Default::default()
        }],
    )
    .unwrap();
    store::ensure_folder(&conn, "has-sent@example.com", "Postausgang").unwrap();
    store::upsert_messages(
        &conn,
        "has-sent@example.com",
        "Postausgang",
        &[MessageHeader {
            uid: 3,
            subject: "Sent mail".to_string(),
            from_addr: "me@example.com".to_string(),
            date: 500,
            seen: false,
            thread_key: "sent-topic".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":165,"method":"mail.threadList","params":{"account_id":"unified","folder_id":"sent","folder_role":"sent","filter":"all"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let threads = value["result"]["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1, "{value}");
    assert_eq!(threads[0]["account_id"], "has-sent@example.com", "{value}");
    assert_eq!(threads[0]["folder_id"], "Postausgang", "{value}");
    // A missing role is not an error: an account whose server has no Sent
    // mailbox is simply absent, with no failure banner for the view to show.
    assert_eq!(value["result"]["failures"], json!([]), "{value}");

    // Folder-wide mutations use the same role resolution. The one account with
    // a Sent mailbox reaches authentication, while the account without one is
    // skipped; treating "sent" as a literal folder would silently update neither.
    let mark_all = invoke_mobile_protocol_json(
        r#"{"id":168,"method":"mail.markAllRead","params":{"account_id":"unified","folder_id":"sent"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let failures = mark_all["result"]["failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "{mark_all}");
    assert_eq!(
        failures[0]["account_id"], "has-sent@example.com",
        "{mark_all}"
    );

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_keeps_gmail_thread_id_atomic_across_subject_drift() {
    let data_directory = unique_data_directory("gmail-thread-id");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 10,
                subject: "[nonbili/Nora] Profiles bug (Issue #295)".to_string(),
                from_name: "NightStars".to_string(),
                date: fresh_date() + 100,
                seen: true,
                thread_key: "gmthrid:123".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 11,
                subject: "[nonbili/Nora] Profiles bug [Linux Flatpak] (Issue #295)".to_string(),
                from_name: "NightStars".to_string(),
                date: fresh_date() + 200,
                seen: false,
                thread_key: "gmthrid:123".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    drop(conn);

    let value = invoke_mobile_protocol_json(
        r#"{"id":166,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"inbox","filter":"all"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let threads = value["result"]["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1, "{value}");
    assert_eq!(
        threads[0]["thread_id"],
        "me@example.com#INBOX#t.Z210aHJpZDoxMjM"
    );

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_inherits_parent_thread_key_for_sent_reply_chain() {
    let data_directory = unique_data_directory("sent-reply-chain");
    seed_mobile_account(&data_directory, "me@example.com");
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::ensure_folder(&conn, "me@example.com", "Sent").unwrap();

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

    // Reproduces a fresh account add / Sent sync: newest rows can be processed
    // before older parents. The second sent reply has no usable References root,
    // so its computed key fell back to In-Reply-To (the first sent reply), even
    // though that parent appears later in the same batch.
    store::upsert_messages(
        &conn,
        "me@example.com",
        "Sent",
        &[
            MessageHeader {
                uid: 9,
                subject: "Re: test mailo2".to_string(),
                from_name: "Me".to_string(),
                from_addr: "me@example.com".to_string(),
                date: 300,
                seen: true,
                thread_key: "first-reply@mailo.com".to_string(),
                message_id: "second-reply@mailo.com".to_string(),
                in_reply_to: "first-reply@mailo.com".to_string(),
                ..Default::default()
            },
            MessageHeader {
                // Deliberately collides with the root's INBOX UID. IMAP UIDs
                // are mailbox-local, so both messages must keep distinct ids.
                uid: 7,
                subject: "Re: test mailo2".to_string(),
                from_name: "Me".to_string(),
                from_addr: "me@example.com".to_string(),
                date: 200,
                seen: true,
                thread_key: "root@mail.gmail.com".to_string(),
                message_id: "first-reply@mailo.com".to_string(),
                in_reply_to: "root@mail.gmail.com".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    store::save_cached_message(
        &conn,
        "me@example.com",
        "Sent",
        7,
        &Message {
            subject: "Re: test mailo2".to_string(),
            from_name: "Me".to_string(),
            from_addr: "me@example.com".to_string(),
            to: "sender@gmail.com".to_string(),
            message_id: "first-reply@mailo.com".to_string(),
            references: "root@mail.gmail.com".to_string(),
            date: 200,
            body: "first".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    store::save_cached_message(
        &conn,
        "me@example.com",
        "Sent",
        9,
        &Message {
            subject: "Re: test mailo2".to_string(),
            from_name: "Me".to_string(),
            from_addr: "me@example.com".to_string(),
            to: "sender@gmail.com".to_string(),
            message_id: "second-reply@mailo.com".to_string(),
            references: "root@mail.gmail.com first-reply@mailo.com".to_string(),
            date: 300,
            body: "second".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    drop(conn);

    let sent = invoke_mobile_protocol_json(
        r#"{"id":167,"method":"mail.threadList","params":{"account_id":"me@example.com","folder_id":"Sent","filter":"all"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let sent_threads = sent["result"]["threads"].as_array().unwrap();
    assert_eq!(sent_threads.len(), 1, "{sent}");
    assert_eq!(
        sent_threads[0]["thread_id"],
        "me@example.com#Sent#t.cm9vdEBtYWlsLmdtYWlsLmNvbSN0ZXN0IG1haWxvMg"
    );

    let read = invoke_mobile_protocol_json(
        r#"{"id":168,"method":"mail.threadRead","params":{"thread_id":"me@example.com#INBOX#t.cm9vdEBtYWlsLmdtYWlsLmNvbQ"}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    let messages = read["result"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3, "{read}");
    assert_eq!(messages[0]["id"], "me@example.com#INBOX#7");
    assert_eq!(messages[1]["id"], "me@example.com#Sent#7");
    assert_eq!(messages[0]["message_id"], "root@mail.gmail.com");
    assert_eq!(messages[1]["message_id"], "first-reply@mailo.com");
    assert_eq!(messages[2]["message_id"], "second-reply@mailo.com");

    let _ = std::fs::remove_dir_all(data_directory);
}
