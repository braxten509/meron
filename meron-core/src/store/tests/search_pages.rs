//! Search pagination across accounts and folders.
//! Equal folder-local UIDs can refer to different messages.
//! Cases check date order, pending hits and stable continuation cursors.

use super::*;

#[test]
fn search_messages_pages_with_a_keyset_cursor() {
    let conn = test_conn();
    for uid in 1..=3u32 {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date)
             VALUES('acct', 'INBOX', ?1, ?2, 'deploy notes', 'Ops', 'ops@example.com', ?3)",
            params![uid.to_string(), uid, 100 * uid as i64],
        )
        .unwrap();
    }

    let first = search_messages(&conn, "acct", "INBOX", "deploy", 2, None).unwrap();
    assert_eq!(
        first.iter().map(|message| message.uid).collect::<Vec<_>>(),
        vec![3, 2]
    );
    let cursor = crate::thread_list::SearchCursor {
        date: 200,
        uid: 2,
        folder: "INBOX".to_string(),
        scanned: 2,
        snapshot: None,
        offset: 0,
    };

    let second = search_messages(&conn, "acct", "INBOX", "deploy", 2, Some(&cursor)).unwrap();
    assert_eq!(
        second.iter().map(|message| message.uid).collect::<Vec<_>>(),
        vec![1]
    );
    // A short page means the result set is exhausted, so paging stops.
    assert!(search_next_cursor(&second, 2, 4).is_none());
}

#[test]
fn search_messages_in_folders_merges_newest_first_and_tags_the_folder() {
    let conn = test_conn();
    let insert = |folder: &str, uid: u32, date: i64| {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date)
             VALUES('acct', ?1, ?2, ?3, 'deploy notes', 'Ops', 'ops@example.com', ?4)",
            params![folder, uid.to_string(), uid, date],
        )
        .unwrap();
    };
    insert("INBOX", 1, 100);
    insert("Sent", 1, 200);
    insert("Archive", 9, 300);

    let hits = search_messages_in_folders(
        &conn,
        "acct",
        &["INBOX".to_string(), "Sent".to_string()],
        "deploy",
        10,
        None,
    )
    .unwrap();
    // Folder-scoped UIDs collide across mailboxes, so both uid-1 rows survive;
    // the un-searched folder does not.
    assert_eq!(
        hits.iter()
            .map(|message| (message.folder.as_str(), message.uid))
            .collect::<Vec<_>>(),
        vec![("Sent", 1), ("INBOX", 1)]
    );
}

#[test]
fn search_messages_in_folders_pages_across_equal_folder_scoped_uids() {
    let conn = test_conn();
    for folder in ["INBOX", "Sent"] {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date)
             VALUES('acct', ?1, '1', 1, 'deploy notes', 'Ops', 'ops@example.com', 100)",
            params![folder],
        )
        .unwrap();
    }

    let folders = ["INBOX".to_string(), "Sent".to_string()];
    let first = search_messages_in_folders(&conn, "acct", &folders, "deploy", 1, None).unwrap();
    assert_eq!(first[0].folder, "Sent");

    let cursor = crate::thread_list::parse_search_cursor(
        &search_next_cursor(&first, 1, 1).expect("full page has a cursor"),
    )
    .unwrap();
    let second =
        search_messages_in_folders(&conn, "acct", &folders, "deploy", 1, Some(&cursor)).unwrap();
    assert_eq!(second[0].folder, "INBOX");
}

#[test]
fn search_snapshot_pages_in_resolved_date_order_when_uids_diverge() {
    let conn = test_conn();
    for (uid, date) in [(3u32, 100i64), (2, 300), (1, 200)] {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date)
             VALUES('acct', 'INBOX', ?1, ?2, 'deploy notes', 'Ops', 'ops@example.com', ?3)",
            params![uid.to_string(), uid, date],
        )
        .unwrap();
    }
    let folders = ["INBOX".to_string()];
    let mut hits =
        search_messages_in_folders(&conn, "acct", &folders, "deploy", u32::MAX, None).unwrap();
    sort_search_hits_all(&mut hits);
    assert_eq!(
        hits.iter().map(|message| message.uid).collect::<Vec<_>>(),
        vec![2, 1, 3]
    );

    let token = create_search_snapshot(&conn, "acct", "deploy", &folders, &[]).unwrap();
    finish_search_batch(&conn, &token, "acct", &[], &hits, None).unwrap();
    let first = get_search_snapshot_page(&conn, "acct", "deploy", &folders, &token, 0, 1)
        .unwrap()
        .unwrap();
    assert_eq!(first.messages[0].uid, 2);
    assert!(first.has_more);

    let second = get_search_snapshot_page(
        &conn,
        "acct",
        "deploy",
        &folders,
        &token,
        first.next_offset,
        1,
    )
    .unwrap()
    .unwrap();
    assert_eq!(second.messages[0].uid, 1);
    assert!(
        get_search_snapshot_page(&conn, "acct", "different", &folders, &token, 0, 1)
            .unwrap()
            .is_none()
    );
}

#[test]
fn search_pending_is_taken_newest_first_by_date_across_folders() {
    let conn = test_conn();
    let hit = |folder: &str, uid: u32, date: i64| PendingSearchHit {
        folder: folder.to_string(),
        uid,
        date,
    };
    let folders = ["INBOX".to_string(), "Sent".to_string()];
    // UID 1 was imported late: the lowest UID carries the newest date.
    let token = create_search_snapshot(
        &conn,
        "acct",
        "deploy",
        &folders,
        &[
            hit("INBOX", 3, 300),
            hit("INBOX", 2, 200),
            hit("INBOX", 1, 1000),
            hit("Sent", 9, 250),
        ],
    )
    .unwrap();

    let (batch, next) = take_search_pending(&conn, &token, 2).unwrap();
    assert_eq!(batch, vec![hit("INBOX", 1, 1000), hit("INBOX", 3, 300)]);
    assert_eq!(next, Some(hit("Sent", 9, 250)));
    let (batch, next) = take_search_pending(&conn, &token, 4).unwrap();
    assert_eq!(batch.len(), 4);
    assert_eq!(next, None);
}

#[test]
fn search_batches_consume_pending_hits_and_append_in_order() {
    let conn = test_conn();
    for (uid, date) in [(5u32, 500i64), (4, 400), (3, 300)] {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date)
             VALUES('acct', 'INBOX', ?1, ?2, 'deploy notes', 'Ops', 'ops@example.com', ?3)",
            params![uid.to_string(), uid, date],
        )
        .unwrap();
    }
    let folders = ["INBOX".to_string()];
    let pending = [(5u32, 500i64), (4, 400), (3, 300)].map(|(uid, date)| PendingSearchHit {
        folder: "INBOX".to_string(),
        uid,
        date,
    });
    let token = create_search_snapshot(&conn, "acct", "deploy", &folders, &pending).unwrap();
    let header = |uid: u32, date: i64| MessageHeader {
        uid,
        date,
        folder: "INBOX".to_string(),
        ..Default::default()
    };

    let (batch, next) = take_search_pending(&conn, &token, 2).unwrap();
    assert_eq!(
        batch.iter().map(|hit| hit.uid).collect::<Vec<_>>(),
        vec![5, 4]
    );
    assert_eq!(next.map(|hit| hit.uid), Some(3));
    finish_search_batch(
        &conn,
        &token,
        "acct",
        &[("INBOX".to_string(), 5), ("INBOX".to_string(), 4)],
        &[header(5, 500), header(4, 400)],
        None,
    )
    .unwrap();
    let page = get_search_snapshot_page(&conn, "acct", "deploy", &folders, &token, 0, 10)
        .unwrap()
        .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>(),
        vec![5, 4]
    );
    assert!(page.pending);

    // The last batch appends after the first and leaves nothing pending; a hit
    // already listed is not appended twice.
    finish_search_batch(
        &conn,
        &token,
        "acct",
        &[("INBOX".to_string(), 3)],
        &[header(4, 400), header(3, 300)],
        None,
    )
    .unwrap();
    let page = get_search_snapshot_page(&conn, "acct", "deploy", &folders, &token, 0, 10)
        .unwrap()
        .unwrap();
    assert_eq!(
        page.messages
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>(),
        vec![5, 4, 3]
    );
    assert!(!page.pending);
    assert_eq!(
        take_search_pending(&conn, &token, 2).unwrap(),
        (vec![], None)
    );
}
