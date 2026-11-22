//! Exercises store behavior for drafts.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn delete_draft_copies_removes_stale_autosaves_keeping_live_uid() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    // Two autosaves of the same draft (stable Message-ID, fresh UID per APPEND)
    // plus an unrelated draft that must survive.
    let draft = |uid: u32, message_id: &str| MessageHeader {
        uid,
        subject: "Re: test".into(),
        date: 100 + uid as i64,
        thread_key: "root@mail.example".into(),
        message_id: message_id.into(),
        ..Default::default()
    };
    upsert_messages(
        &conn,
        "acct",
        "Drafts",
        &[
            draft(10, "Meron-Draft-1@meron"),
            draft(11, "meron-draft-1@meron"),
            draft(12, "meron-draft-2@meron"),
        ],
    )
    .unwrap();

    // Replace path: keep the newest copy, drop the stale one (id compared
    // case-insensitively, matching the thread-key inheritance lookup).
    let deleted =
        delete_draft_copies(&conn, "acct", "Drafts", "meron-draft-1@meron", Some(11)).unwrap();
    assert_eq!(deleted, 1);
    let uids: Vec<u32> = conn
        .prepare("SELECT uid FROM messages WHERE folder = 'Drafts' ORDER BY uid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(uids, vec![11, 12]);

    // Discard path: no survivor, every copy goes.
    let deleted =
        delete_draft_copies(&conn, "acct", "Drafts", "meron-draft-1@meron", None).unwrap();
    assert_eq!(deleted, 1);
    // Blank id is a no-op, never a mass delete.
    assert_eq!(
        delete_draft_copies(&conn, "acct", "Drafts", " ", None).unwrap(),
        0
    );
}

#[test]
fn thread_draft_cleanup_requires_real_thread_key_and_generated_id() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    let draft = |uid: u32, thread_key: &str, message_id: &str| MessageHeader {
        uid,
        thread_key: thread_key.into(),
        message_id: message_id.into(),
        ..Default::default()
    };
    upsert_messages(
        &conn,
        "acct",
        "Drafts",
        &[
            draft(10, "root@example.com", "meron-draft-old@gmail.com"),
            draft(11, "root@example.com", "manual@example.com"),
            draft(12, "uid:12", "meron-draft-uid@gmail.com"),
        ],
    )
    .unwrap();

    assert_eq!(
        delete_quick_reply_drafts_in_thread(&conn, "acct", "Drafts", "uid:12").unwrap(),
        0
    );
    assert_eq!(
        delete_quick_reply_drafts_in_thread(&conn, "acct", "Drafts", "root@example.com").unwrap(),
        1
    );
    let uids: Vec<u32> = conn
        .prepare("SELECT uid FROM messages WHERE folder = 'Drafts' ORDER BY uid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(uids, vec![11, 12]);
}

#[test]
fn delete_draft_sibling_copies_drops_hidden_same_id_rows() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    let draft = |uid: u32, message_id: &str| MessageHeader {
        uid,
        subject: "Re: test".into(),
        date: 100 + uid as i64,
        thread_key: "root@mail.example".into(),
        message_id: message_id.into(),
        ..Default::default()
    };
    // The pane dedupes uid 10/11 (same Message-ID) into one bubble; discarding
    // it deletes only the visible uid, so the sibling cleanup must catch the
    // hidden copy — while leaving the unrelated draft alone.
    upsert_messages(
        &conn,
        "acct",
        "Drafts",
        &[
            draft(10, "meron-draft-1@meron"),
            draft(11, "meron-draft-1@meron"),
            draft(12, "meron-draft-2@meron"),
        ],
    )
    .unwrap();

    let deleted = delete_draft_sibling_copies(&conn, "acct", "Drafts", &[11]).unwrap();
    assert_eq!(deleted, 1);
    delete_messages_by_uid(&conn, "acct", "Drafts", &[11]).unwrap();
    let uids: Vec<u32> = conn
        .prepare("SELECT uid FROM messages WHERE folder = 'Drafts' ORDER BY uid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(uids, vec![12]);
}

#[test]
fn collapse_thread_draft_headers_keeps_newest_draft_only() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    upsert_folders(
        &conn,
        "acct",
        &[crate::imap::Folder {
            name: "Drafts".into(),
            display_name: "Drafts".into(),
            delimiter: None,
            special_use: Some("drafts".into()),
            role: "drafts".into(),
            unread: 0,
            notify: false,
        }],
    )
    .unwrap();
    let header = |uid: u32, folder: &str, date: i64| MessageHeader {
        uid,
        folder: folder.into(),
        subject: "Re: test".into(),
        date,
        thread_key: "root@example.com".into(),
        message_id: format!("draft-{uid}@example.com"),
        ..Default::default()
    };

    let collapsed = collapse_thread_draft_headers(
        &conn,
        "acct",
        "INBOX",
        vec![
            header(1, "INBOX", 100),
            header(10, "Drafts", 110),
            header(11, "Drafts", 120),
        ],
    )
    .unwrap();

    let uids = collapsed
        .into_iter()
        .map(|header| header.uid)
        .collect::<Vec<_>>();
    assert_eq!(uids, vec![1, 11]);
}
