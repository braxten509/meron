//! Exercises store behavior for search.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn search_messages_matches_subject_sender_and_body_case_insensitively() {
    let conn = test_conn();
    insert_message(&conn, 1, "Quarterly Plan", "Aki", "aki@example.com", None);
    insert_message(&conn, 2, "Hello", "Launch Team", "team@example.com", None);
    insert_message(
        &conn,
        3,
        "Notes",
        "Ops",
        "ops@example.com",
        Some("The deploy window is confirmed."),
    );
    insert_message(&conn, 4, "Other", "No Match", "other@example.com", None);

    let subject = search_messages(&conn, "acct", "INBOX", "quarterly", 10, None).unwrap();
    assert_eq!(subject.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![1]);

    let sender = search_messages(&conn, "acct", "INBOX", "launch", 10, None).unwrap();
    assert_eq!(sender.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![2]);

    let body = search_messages(&conn, "acct", "INBOX", "DEPLOY", 10, None).unwrap();
    assert_eq!(body.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![3]);
}

#[test]
fn cached_search_reads_carry_the_message_id() {
    use crate::imap::MessageHeader;
    // Search pages span folders, and the grouping that turns them into cards
    // folds a self-sent message's two copies by Message-ID. A cache-answered
    // search (offline, or the first page before the live results land) must
    // carry the id too, or its cards count that message twice.
    let conn = test_conn();
    let copy = |folder: &str, uid: u32| MessageHeader {
        uid,
        folder: folder.to_string(),
        subject: "Quarterly Plan".to_string(),
        date: 100,
        thread_key: "root@example.com".to_string(),
        message_id: "<self@example.com>".to_string(),
        ..Default::default()
    };
    ensure_folder(&conn, "acct", "INBOX").unwrap();
    ensure_folder(&conn, "acct", "Sent").unwrap();
    upsert_messages(&conn, "acct", "INBOX", &[copy("INBOX", 1)]).unwrap();
    upsert_messages(&conn, "acct", "Sent", &[copy("Sent", 9)]).unwrap();

    let folders = vec!["INBOX".to_string(), "Sent".to_string()];
    // Both reads: the trigram index answers queries of three codepoints or
    // more, the LIKE scan the shorter ones.
    for query in ["quarterly", "an"] {
        let hits = search_messages_in_folders(&conn, "acct", &folders, query, 10, None).unwrap();
        assert_eq!(hits.len(), 2, "{query}");
        assert!(
            hits.iter()
                .all(|hit| hit.message_id == "<self@example.com>"),
            "{query}"
        );
        let cards = group_thread_cards(hits, "INBOX");
        assert_eq!(cards.len(), 1, "{query}");
        assert_eq!(cards[0].message_count, 1, "{query}");
    }
}

#[test]
fn search_messages_has_attachment_reads_the_files_column() {
    let conn = test_conn();
    insert_message(&conn, 1, "Plan", "Aki", "aki@example.com", None);
    insert_message(&conn, 2, "Plan", "Aki", "aki@example.com", None);
    conn.execute(
        "UPDATE messages SET files = '[{\"filename\":\"a.pdf\",\"mime\":\"application/pdf\"}]'
         WHERE uid = 2",
        [],
    )
    .unwrap();

    let uids = |query: &str| {
        search_messages(&conn, "acct", "INBOX", query, 10, None)
            .unwrap()
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>()
    };
    assert_eq!(uids("has:attachment"), [2]);
    assert_eq!(uids("plan has:attachment"), [2]);
}

#[test]
fn cached_search_headers_returns_the_rows_it_holds() {
    let conn = test_conn();
    insert_message(&conn, 1, "Plain", "Aki", "aki@example.com", Some("read"));
    insert_message(&conn, 2, "Files", "Aki", "aki@example.com", None);
    conn.execute(
        "UPDATE messages SET files = '[{\"filename\":\"a.pdf\",\"mime\":\"application/pdf\"}]',
                             json = '{\"gmail_msg_id\":7,\"message_id\":\"m2\",\"attachment_metadata\":true}'
         WHERE uid = 2",
        [],
    )
    .unwrap();

    // Header-only, attachments never read: the fetch has to fill this one in.
    insert_message(&conn, 3, "Unknown", "Aki", "aki@example.com", None);

    let headers = cached_search_headers(&conn, "acct", "INBOX", &[1, 2, 3, 4]).unwrap();
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[0].folder, "INBOX");
    assert_eq!(headers[0].files.as_deref().map(<[_]>::len), Some(0));
    assert_eq!(headers[1].files.as_deref().map(<[_]>::len), Some(1));
    assert_eq!(headers[1].gmail_msg_id, Some(7));
    assert_eq!(headers[1].message_id, "m2");
}

#[test]
fn search_messages_matches_substrings_including_cjk() {
    let conn = test_conn();
    insert_message(
        &conn,
        1,
        "Quarterly planning",
        "Aki",
        "aki@example.com",
        None,
    );
    insert_message(&conn, 2, "上海会议通知", "Aki", "aki@example.com", None);

    // Mid-word Latin substring (trigram, >= 3 chars).
    let latin = search_messages(&conn, "acct", "INBOX", "arter", 10, None).unwrap();
    assert_eq!(latin.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![1]);

    // CJK substring with no surrounding word breaks (>= 3 chars -> FTS).
    let cjk = search_messages(&conn, "acct", "INBOX", "会议通", 10, None).unwrap();
    assert_eq!(cjk.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![2]);

    let miss = search_messages(&conn, "acct", "INBOX", "zzz", 10, None).unwrap();
    assert!(miss.is_empty());
}

#[test]
fn search_messages_short_query_falls_back_to_like() {
    // Queries below the trigram minimum (3 codepoints) must still match via
    // the LIKE fallback — important for 2-character CJK words.
    let conn = test_conn();
    insert_message(&conn, 1, "上海会议通知", "Aki", "aki@example.com", None);
    insert_message(&conn, 2, "Other", "Aki", "aki@example.com", None);

    let hit = search_messages(&conn, "acct", "INBOX", "上海", 10, None).unwrap();
    assert_eq!(hit.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![1]);
}

#[test]
fn search_messages_short_query_folds_non_ascii_case() {
    // SQLite's lower() is ASCII-only; the short-query path must fold Cyrillic
    // and accented capitals the way the trigram index does for longer queries.
    let conn = test_conn();
    insert_message(&conn, 1, "Ок, созвон", "Aki", "aki@example.com", None);
    insert_message(&conn, 2, "ÉT report", "Aki", "aki@example.com", None);
    insert_message(&conn, 3, "Other", "Aki", "aki@example.com", None);

    let hit = search_messages(&conn, "acct", "INBOX", "ок", 10, None).unwrap();
    assert_eq!(hit.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![1]);
    let hit = search_messages(&conn, "acct", "INBOX", "ét", 10, None).unwrap();
    assert_eq!(hit.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![2]);
}

#[test]
fn search_messages_reindexes_on_body_update() {
    // The FTS triggers must follow a later body write, not just the initial insert.
    let conn = test_conn();
    insert_message(&conn, 1, "Notes", "Ops", "ops@example.com", None);
    assert!(
        search_messages(&conn, "acct", "INBOX", "deploy", 10, None)
            .unwrap()
            .is_empty()
    );

    conn.execute(
        "UPDATE messages SET body = 'deploy window confirmed' WHERE uid = 1",
        [],
    )
    .unwrap();

    let hit = search_messages(&conn, "acct", "INBOX", "deploy", 10, None).unwrap();
    assert_eq!(hit.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![1]);
}

#[test]
fn search_messages_matches_to_and_cc_recipients() {
    // Recipients live in the JSON catch-all, so they are searchable only through
    // the mirrored `recipients` column and its index — both the trigram path and
    // the short-query LIKE fallback must see them.
    let conn = test_conn();
    upsert_messages(
        &conn,
        "acct",
        "Sent",
        &[MessageHeader {
            uid: 1,
            subject: "Lunch".to_string(),
            from_addr: "me@example.com".to_string(),
            to: vec![crate::imap::Recipient {
                name: "Ann Baker".to_string(),
                addr: "ann@example.com".to_string(),
            }],
            cc: vec![crate::imap::Recipient {
                name: String::new(),
                addr: "ops@example.com".to_string(),
            }],
            ..Default::default()
        }],
    )
    .unwrap();

    for query in ["Ann Baker", "ann@example", "ops@example", "an"] {
        let hits = search_messages(&conn, "acct", "Sent", query, 10, None).unwrap();
        assert_eq!(
            hits.iter().map(|m| m.uid).collect::<Vec<_>>(),
            vec![1],
            "query: {query}"
        );
    }
    assert!(
        search_messages(&conn, "acct", "Sent", "zoe@example", 10, None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn search_messages_applies_gmail_style_operators() {
    let conn = test_conn();
    let message = |uid: u32, from: &str, to: &str, subject: &str, date: i64| MessageHeader {
        uid,
        subject: subject.to_string(),
        from_name: String::new(),
        from_addr: from.to_string(),
        date,
        seen: uid == 1,
        starred: uid == 2,
        to: vec![crate::imap::Recipient {
            name: String::new(),
            addr: to.to_string(),
        }],
        ..Default::default()
    };
    let day = |y, m, d| {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .unwrap()
            .timestamp()
    };
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[
            message(1, "a@b.c", "d@e.f", "Budget plan", day(2026, 1, 1)),
            message(2, "a@b.c", "x@y.z", "Budget", day(2026, 2, 1)),
            message(3, "zoe@b.c", "d@e.f", "Lunch plan", day(2026, 3, 1)),
        ],
    )
    .unwrap();

    let uids = |query: &str| {
        search_messages(&conn, "acct", "INBOX", query, 10, None)
            .unwrap()
            .iter()
            .map(|m| m.uid)
            .collect::<Vec<_>>()
    };
    assert_eq!(uids("from:a@b.c"), [2, 1]);
    assert_eq!(uids("from:a@b.c to:d@e.f"), [1]);
    // Free text still searches every column, ANDed with the operators.
    assert_eq!(uids("to:d@e.f plan"), [3, 1]);
    assert_eq!(uids("subject:budget is:unread"), [2]);
    assert_eq!(uids("is:starred"), [2]);
    assert_eq!(uids("after:2026/01/15 before:2026/03/01"), [2]);
    assert_eq!(uids("after:2026/03/01"), [3]);
    // Short values take the scan path and still respect their column.
    assert_eq!(uids("from:zo"), [3]);
    assert_eq!(uids("subject:lu"), [3]);
    assert_eq!(uids("from:lu"), Vec::<u32>::new());
    // A bare operator is plain text, not an empty filter.
    assert_eq!(uids("from:"), Vec::<u32>::new());
    // Alternatives: any of them matches, indexed or scanned.
    assert_eq!(uids("from:{zoe@b.c x@y.z}"), [3]);
    assert_eq!(uids("to:(x@y.z OR d@e.f) subject:plan"), [3, 1]);
    assert_eq!(uids("subject:lunch OR subject:budget"), [3, 2, 1]);
    assert_eq!(uids("from:zo,nobody@x.y"), [3]);
}

#[test]
fn search_messages_keeps_indexed_recipients_through_a_flag_only_resync() {
    // A flag-only resync carries no envelope; it must not blank the recipients
    // the envelope fetch already indexed.
    let conn = test_conn();
    let envelope = MessageHeader {
        uid: 1,
        subject: "Lunch".to_string(),
        to: vec![crate::imap::Recipient {
            name: "Ann".to_string(),
            addr: "ann@example.com".to_string(),
        }],
        ..Default::default()
    };
    upsert_messages(&conn, "acct", "INBOX", &[envelope]).unwrap();
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[MessageHeader {
            uid: 1,
            subject: "Lunch".to_string(),
            seen: true,
            ..Default::default()
        }],
    )
    .unwrap();

    let hits = search_messages(&conn, "acct", "INBOX", "ann@example", 10, None).unwrap();
    assert_eq!(hits.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![1]);
}

#[test]
fn migration_indexes_recipients_already_cached_before_v6() {
    // An existing install carries its recipients only in `json`; the upgrade has
    // to backfill and index them, or search stays blind to everything synced
    // before it.
    let conn = Connection::open_in_memory().unwrap();
    db::migrate_to_v5(&conn).unwrap();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, json)
         VALUES('acct', 'Sent', '1', 1, 'Lunch', 'Me', 'me@example.com', 100,
                '{\"to\":[{\"name\":\"Ann Baker\",\"addr\":\"ann@example.com\"}]}')",
        [],
    )
    .unwrap();

    db::run_migrations(&conn).unwrap();

    let hits = search_messages(&conn, "acct", "Sent", "ann@example", 10, None).unwrap();
    assert_eq!(hits.iter().map(|m| m.uid).collect::<Vec<_>>(), vec![1]);
}

#[test]
fn search_messages_returns_empty_for_blank_query() {
    let conn = test_conn();
    insert_message(&conn, 1, "Hello", "Aki", "aki@example.com", None);

    let results = search_messages(&conn, "acct", "INBOX", "  ", 10, None).unwrap();
    assert!(results.is_empty());
}

#[test]
fn matching_uids_reads_headers_and_never_a_cached_body() {
    let conn = test_conn();
    insert_message(
        &conn,
        1,
        "Plan",
        "Aki",
        "aki@example.com",
        Some("the secretword"),
    );
    insert_message(&conn, 2, "secretword", "Aki", "aki@example.com", None);
    insert_message(&conn, 3, "Plan", "Aki", "aki@example.com", Some("ab here"));
    insert_message(&conn, 4, "ab", "Aki", "aki@example.com", None);

    let hits = |query: &str| matching_uids(&conn, "acct", "INBOX", query, &[1, 2, 3, 4]).unwrap();
    assert_eq!(hits("secretword"), [2]);
    // Short enough to be scanned, not looked up in the index.
    assert_eq!(hits("ab"), [4]);
    let search = search_messages(&conn, "acct", "INBOX", "secretword", 10, None).unwrap();
    assert_eq!(search.len(), 2);
}
