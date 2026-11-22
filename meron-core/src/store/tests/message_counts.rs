//! Exercises store behavior for message counts.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn card_message_counts_span_the_folder_not_the_page() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    let insert = |uid: u32, subject: &str, seen: i64| {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr,
                                  date, seen, thread_key)
             VALUES('acct', 'INBOX', ?1, ?2, ?3, 'Aki', 'aki@example.com', ?5, ?4,
                    'root@example.com')",
            params![uid.to_string(), uid, subject, seen, fresh_date()],
        )
        .unwrap();
    };
    insert(1, "Old topic", 1);
    insert(2, "Re: Old topic", 1);
    insert(3, "Re: Old topic", 0);
    insert(4, "New topic", 0);

    let card_key = |uid: u32, subject: &str| {
        card_thread_key(&MessageHeader {
            uid,
            subject: subject.to_string(),
            thread_key: "root@example.com".to_string(),
            ..Default::default()
        })
    };
    let old_key = card_key(1, "Old topic");
    let new_key = card_key(4, "New topic");

    // The unread view hands grouping only the two unread messages, so the cards
    // it produces tally one message each.
    let (page, _) = get_recent_page(&conn, "acct", "INBOX", 50, None, UNREAD).unwrap();
    let cards = group_thread_cards(page, "INBOX");
    let keys = cards
        .iter()
        .map(|card| card.thread_key.clone())
        .collect::<Vec<_>>();
    assert!(cards.iter().all(|card| card.message_count == 1));

    // The cache knows better: the old branch holds three messages (one unread),
    // and the drifted subject is counted as its own branch, not folded in.
    let counts = card_message_counts(&conn, "acct", "INBOX", &keys).unwrap();
    assert_eq!(counts.get(&old_key).copied(), Some(3));
    assert_eq!(counts.get(&new_key).copied(), Some(1));
}

#[test]
fn card_message_counts_span_folders_and_dedupe_self_sent_copies() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    let insert = |folder: &str, uid: u32, subject: &str, thread_key: &str, message_id: &str| {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr,
                                  date, seen, thread_key, json)
             VALUES('acct', ?1, ?2, ?3, ?4, 'Aki', 'aki@example.com', 1779580800, 1, ?5,
                    json_object('message_id', ?6))",
            params![
                folder,
                format!("{folder}-{uid}"),
                uid,
                subject,
                thread_key,
                message_id
            ],
        )
        .unwrap();
    };
    // A received message and the reply the user sent back: one thread, two
    // folders. The Sent reply is also cached in All Mail, as Gmail serves it.
    insert(
        "INBOX",
        1,
        "Old topic",
        "root@example.com",
        "<a@example.com>",
    );
    insert(
        "Sent",
        7,
        "Re: Old topic",
        "root@example.com",
        "<b@example.com>",
    );
    insert(
        "All Mail",
        9,
        "Re: Old topic",
        "root@example.com",
        "<b@example.com>",
    );
    insert(
        "INBOX",
        10,
        "Gmail topic",
        "gmthrid:123",
        "<gmail-a@example.com>",
    );
    insert(
        "Sent",
        11,
        "Different Gmail subject",
        "gmthrid:123",
        "<gmail-b@example.com>",
    );
    // Same UID, different folder, no threading headers of its own.
    insert("INBOX", 4, "Standalone", "", "<c@example.com>");
    insert("Archive", 4, "Unrelated", "", "<d@example.com>");

    let threaded = card_thread_key(&MessageHeader {
        uid: 1,
        subject: "Old topic".to_string(),
        thread_key: "root@example.com".to_string(),
        ..Default::default()
    });
    let gmail_threaded = "gmthrid:123".to_string();
    let counts = card_message_counts(
        &conn,
        "acct",
        "INBOX",
        &[
            threaded.clone(),
            gmail_threaded.clone(),
            "uid:4".to_string(),
        ],
    )
    .unwrap();

    // Two bubbles open in the reader, so the row says two: the All Mail copy of
    // the Sent reply is folded by Message-ID, not counted twice.
    assert_eq!(counts.get(&threaded).copied(), Some(2));
    // Gmail's atomic thread key also spans folders even though it does not
    // branch by subject: only synthetic uid: keys are folder-local.
    assert_eq!(counts.get(&gmail_threaded).copied(), Some(2));
    // A synthetic uid: key stays folder-local; the unrelated Archive row that
    // happens to share UID 4 is not part of this thread.
    assert_eq!(counts.get("uid:4").copied(), Some(1));
}

#[test]
fn get_recent_page_can_return_only_unread_messages() {
    let conn = test_conn();
    insert_message(&conn, 2, "Unread older", "Aki", "aki@example.com", None);
    insert_message(&conn, 3, "Unread middle", "Aki", "aki@example.com", None);
    insert_message(&conn, 4, "Read middle", "Aki", "aki@example.com", None);
    insert_message(&conn, 5, "Unread newest", "Aki", "aki@example.com", None);
    conn.execute(
        "UPDATE messages SET seen = 1 WHERE account = 'acct' AND folder = 'INBOX' AND uid = 4",
        [],
    )
    .unwrap();

    // insert_message stamps every row with the same date, so the date-ordered
    // list ties break on uid DESC and the cursor carries that shared date.
    let date = fresh_date();
    let (all, all_cursor) =
        get_recent_page(&conn, "acct", "INBOX", 2, None, RecentFilter::default()).unwrap();
    assert_eq!(all.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![5, 4]);
    assert_eq!(
        all_cursor.as_deref(),
        Some(format!("date:{date}:4").as_str())
    );

    let (unread, unread_cursor) = get_recent_page(&conn, "acct", "INBOX", 2, None, UNREAD).unwrap();
    assert_eq!(unread.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![5, 3]);
    assert_eq!(
        unread_cursor.as_deref(),
        Some(format!("date:{date}:3").as_str())
    );
    assert!(unread.iter().all(|m| !m.seen));

    let (next_unread, next_cursor) =
        get_recent_page(&conn, "acct", "INBOX", 2, Some((date, 3)), UNREAD).unwrap();
    assert_eq!(
        next_unread.iter().map(|m| m.uid).collect::<Vec<_>>(),
        vec![2]
    );
    assert_eq!(next_cursor, None);
}

#[test]
fn new_unread_inbox_messages_counts_uid_window_and_latest_unread() {
    let conn = test_conn();
    let inbox_messages = vec![
        MessageHeader {
            uid: 1,
            subject: "Before window".to_string(),
            from_addr: "old@example.com".to_string(),
            thread_key: "old".to_string(),
            ..Default::default()
        },
        MessageHeader {
            uid: 2,
            subject: "Lower bound".to_string(),
            from_addr: "lower@example.com".to_string(),
            thread_key: String::new(),
            ..Default::default()
        },
        MessageHeader {
            uid: 3,
            subject: "Already read".to_string(),
            from_addr: "read@example.com".to_string(),
            seen: true,
            thread_key: "read".to_string(),
            ..Default::default()
        },
        MessageHeader {
            uid: 4,
            subject: "Latest unread".to_string(),
            from_name: "Aki".to_string(),
            from_addr: "aki@example.com".to_string(),
            thread_key: "fresh".to_string(),
            ..Default::default()
        },
        MessageHeader {
            uid: 5,
            subject: "Upper bound excluded".to_string(),
            from_addr: "upper@example.com".to_string(),
            thread_key: "upper".to_string(),
            ..Default::default()
        },
    ];
    let arrivals = classify_inbox_arrivals(&conn, "acct", 2, 5, &inbox_messages).unwrap();
    upsert_messages(&conn, "acct", "INBOX", &inbox_messages).unwrap();
    upsert_messages(
        &conn,
        "acct",
        "Archive",
        &[MessageHeader {
            uid: 4,
            subject: "Wrong folder".to_string(),
            from_addr: "archive@example.com".to_string(),
            thread_key: "archive".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();

    assert_eq!(arrivals.len(), 2);
    let latest = &arrivals[0];
    assert_eq!(latest.uid, 4);
    assert_eq!(latest.subject, "Latest unread");
    assert_eq!(latest.thread_key, "fresh");

    let conn = test_conn();
    let lower = vec![MessageHeader {
        uid: 2,
        subject: "Lower bound".to_string(),
        from_addr: "lower@example.com".to_string(),
        thread_key: String::new(),
        ..Default::default()
    }];
    let lower_bound = classify_inbox_arrivals(&conn, "acct", 2, 3, &lower)
        .unwrap()
        .remove(0);
    assert_eq!(lower_bound.uid, 2);
    assert_eq!(lower_bound.thread_key, "");
}

#[test]
fn cached_body_preview_collapses_body_and_misses_without_a_cached_body() {
    let conn = test_conn();
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[MessageHeader {
            uid: 7,
            subject: "Lunch".to_string(),
            from_addr: "aki@example.com".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();

    // Header only: no snippet to show yet.
    assert!(cached_body_preview(&conn, "acct", "INBOX", 7).is_none());

    save_cached_message(
        &conn,
        "acct",
        "INBOX",
        7,
        &Message {
            subject: "Lunch".into(),
            body: "  Are we still on\n\n  for lunch?  ".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        cached_body_preview(&conn, "acct", "INBOX", 7).unwrap(),
        "Are we still on for lunch?"
    );

    // A whitespace-only body is a miss, not an empty snippet line.
    save_cached_message(
        &conn,
        "acct",
        "INBOX",
        7,
        &Message {
            subject: "Lunch".into(),
            body: "   \n  ".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(cached_body_preview(&conn, "acct", "INBOX", 7).is_none());
}
