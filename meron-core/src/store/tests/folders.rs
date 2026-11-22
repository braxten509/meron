//! Exercises store behavior for folders.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn folders_round_trip_special_use_and_ensure_folder_keeps_it() {
    let conn = test_conn();
    upsert_folders(
        &conn,
        "acct",
        &[
            Folder {
                name: "INBOX".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            Folder {
                name: "Mail/Entwürfe".to_string(),
                delimiter: Some("/".to_string()),
                special_use: Some("drafts".to_string()),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    // Message syncs call ensure_folder on every pass; it must not clobber the
    // role recorded by the folder LIST sync.
    ensure_folder(&conn, "acct", "Mail/Entwürfe").unwrap();

    let folders = get_folders(&conn, "acct").unwrap();
    let drafts = folders.iter().find(|f| f.name == "Mail/Entwürfe").unwrap();
    assert_eq!(drafts.special_use.as_deref(), Some("drafts"));
    let inbox = folders.iter().find(|f| f.name == "INBOX").unwrap();
    assert_eq!(inbox.special_use, None);
}

#[test]
fn folder_unread_counts_messages_without_requiring_a_folder_row() {
    let conn = test_conn();
    insert_message(&conn, 1, "Unread", "Ada", "ada@example.com", None);
    conn.execute(
        "UPDATE messages SET seen = 1 WHERE account = 'acct' AND folder = 'INBOX' AND uid = 1",
        [],
    )
    .unwrap();
    insert_message(&conn, 2, "Unread", "Bea", "bea@example.com", None);

    assert_eq!(get_folder_unread(&conn, "acct", "INBOX").unwrap(), 1);
    assert_eq!(get_folder_unread(&conn, "acct", "Archive").unwrap(), 0);
}

#[test]
fn folder_unread_includes_unseen_server_mail_outside_the_cache() {
    let conn = test_conn();
    // On an ordinary folder: a real Inbox counts only the mail it lists, and
    // an unseen UID the cache doesn't hold was never listed.
    insert_message_in(&conn, "Work", 10, "Cached", "Ada", "ada@example.com", None);
    // The server reports 10 (cached) and 3..=5 (older, not cached) unseen.
    set_uncached_unseen(&conn, "acct", "Work", &[3, 4, 5, 10].into_iter().collect()).unwrap();
    assert_eq!(get_folder_unread(&conn, "acct", "Work").unwrap(), 4);
    ensure_folder(&conn, "acct", "Work").unwrap();
    let folders = get_folders(&conn, "acct").unwrap();
    assert_eq!(folders.iter().find(|f| f.name == "Work").unwrap().unread, 4);

    // Paging an older message into the cache doesn't count it twice, and
    // reading it there is reflected.
    insert_message_in(&conn, "Work", 4, "Paged in", "Bea", "bea@example.com", None);
    assert_eq!(get_folder_unread(&conn, "acct", "Work").unwrap(), 4);
    update_message_seen(&conn, "acct", "Work", 4, true).unwrap();
    assert_eq!(get_folder_unread(&conn, "acct", "Work").unwrap(), 3);

    let mut unseen = get_unseen_uids(&conn, "acct", "Work").unwrap();
    unseen.sort_unstable();
    assert_eq!(unseen, vec![3, 5, 10]);
    mark_folder_seen(&conn, "acct", "Work", true).unwrap();
    assert_eq!(get_folder_unread(&conn, "acct", "Work").unwrap(), 0);
}

#[test]
fn a_stale_unseen_snapshot_cannot_restore_an_archived_uid_to_unread_counts() {
    let conn = test_conn();
    // On an ordinary folder: a real Inbox does not count unseen mail the cache
    // doesn't hold.
    insert_message_in(
        &conn,
        "Work",
        7,
        "Archive me",
        "Ada",
        "ada@example.com",
        None,
    );
    let (sync, fetched_at) = begin_message_sync(&conn).unwrap();
    let stale_unseen = [3, 7].into_iter().collect();
    delete_messages_by_uid(&conn, "acct", "Work", &[7]).unwrap();
    release_removed_messages(&conn, "acct", "Work", &[7], fetched_at).unwrap();
    set_uncached_unseen(&conn, "acct", "Work", &stale_unseen).unwrap();
    assert_eq!(get_folder_unread(&conn, "acct", "Work").unwrap(), 1);
    assert_eq!(get_unseen_uids(&conn, "acct", "Work").unwrap(), vec![3]);
    // Removal markers are scoped to their source mailbox and account.
    set_uncached_unseen(&conn, "acct", "Archive", &stale_unseen).unwrap();
    assert_eq!(get_folder_unread(&conn, "acct", "Archive").unwrap(), 2);
    set_uncached_unseen(&conn, "other", "Work", &stale_unseen).unwrap();
    assert_eq!(get_folder_unread(&conn, "other", "Work").unwrap(), 2);
    end_message_sync(&conn, sync).unwrap();
    // A later valid unseen read still counts ordinary uncached mail.
    set_uncached_unseen(&conn, "acct", "Work", &[3, 8].into_iter().collect()).unwrap();
    assert_eq!(get_folder_unread(&conn, "acct", "Work").unwrap(), 2);
}

#[test]
fn removing_a_paged_in_message_does_not_revive_its_uncached_unread() {
    let conn = test_conn();
    set_uncached_unseen(&conn, "acct", "INBOX", &[4].into_iter().collect()).unwrap();
    insert_message(&conn, 4, "Paged in", "Bea", "bea@example.com", None);
    update_message_seen(&conn, "acct", "INBOX", 4, true).unwrap();
    conn.execute(
        "DELETE FROM messages WHERE account = 'acct' AND folder = 'INBOX' AND uid = 4",
        [],
    )
    .unwrap();
    assert_eq!(get_folder_unread(&conn, "acct", "INBOX").unwrap(), 0);
}

#[test]
fn moving_a_cached_message_onto_an_uncached_unseen_uid_counts_it_once() {
    let conn = test_conn();
    insert_message(&conn, 4, "Moved", "Bea", "bea@example.com", None);
    set_uncached_unseen(&conn, "acct", "Archive", &[4].into_iter().collect()).unwrap();
    move_messages_by_uid(&conn, "acct", "INBOX", "Archive", &[4]).unwrap();
    assert_eq!(get_folder_unread(&conn, "acct", "Archive").unwrap(), 1);
}

#[test]
fn emptying_a_folder_clears_its_uncached_unread() {
    let conn = test_conn();
    set_uncached_unseen(&conn, "acct", "INBOX", &[3, 5].into_iter().collect()).unwrap();
    delete_folder_messages(&conn, "acct", "INBOX").unwrap();
    assert_eq!(get_folder_unread(&conn, "acct", "INBOX").unwrap(), 0);
}

#[test]
fn delete_folder_drops_the_row_its_messages_and_its_sync_state() {
    let conn = test_conn();
    upsert_folders(
        &conn,
        "acct",
        &[
            Folder {
                name: "INBOX".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            Folder {
                name: "Work".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            Folder {
                name: "Work/Reports".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    insert_message(&conn, 1, "Kept", "Ada", "ada@example.com", None);
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, date, seen)
         VALUES('acct', 'Work/Reports', '7', 7, 'Q3', 1779580800, 0)",
        [],
    )
    .unwrap();
    set_folder_state(&conn, "acct", "Work/Reports", 1, 8).unwrap();

    // Nesting is read off each row's own delimiter, so the parent knows it has
    // children while the leaf has none.
    assert_eq!(
        child_folders(&conn, "acct", "Work").unwrap(),
        vec!["Work/Reports".to_string()]
    );
    assert!(
        child_folders(&conn, "acct", "Work/Reports")
            .unwrap()
            .is_empty()
    );

    assert_eq!(delete_folder(&conn, "acct", "Work/Reports").unwrap(), 1);
    let names: Vec<String> = get_folders(&conn, "acct")
        .unwrap()
        .into_iter()
        .map(|folder| folder.name)
        .collect();
    assert_eq!(names, vec!["INBOX".to_string(), "Work".to_string()]);
    assert_eq!(
        get_folder_state(&conn, "acct", "Work/Reports").unwrap(),
        None
    );
    // Untouched folders keep their mail.
    assert_eq!(get_folder_unread(&conn, "acct", "INBOX").unwrap(), 1);
}

#[test]
fn delimiterless_folder_names_do_not_form_a_delete_subtree() {
    let conn = test_conn();
    upsert_folders(
        &conn,
        "acct",
        &[
            Folder {
                name: "Work".to_string(),
                delimiter: None,
                ..Default::default()
            },
            Folder {
                name: "Work/Reports".to_string(),
                delimiter: None,
                ..Default::default()
            },
        ],
    )
    .unwrap();

    assert!(child_folders(&conn, "acct", "Work").unwrap().is_empty());
}

#[test]
fn folder_role_assignment_uses_special_use_then_name_fallback() {
    let cases = [
        ("Mail/Entwürfe", Some("drafts"), "drafts"),
        ("Archive", Some("sent"), "sent"),
        ("INBOX", None, "inbox"),
        ("Sent Mail", None, "sent"),
        ("Drafts", None, "drafts"),
        ("Deleted Items", None, "trash"),
        ("Spam", None, "junk"),
        ("All Mail", None, "archive"),
        ("Projects", None, "folder"),
    ];
    for (name, special_use, role) in cases {
        assert_eq!(classify_folder_role(name, special_use), role, "{name}");
    }
}

#[test]
fn draft_thread_keys_detects_special_use_and_name_fallback() {
    let conn = test_conn();
    upsert_folders(
        &conn,
        "acct",
        &[
            Folder {
                name: "Mail/Entwürfe".to_string(),
                special_use: Some("drafts".to_string()),
                ..Default::default()
            },
            Folder {
                name: "[Gmail]/Drafts".to_string(),
                ..Default::default()
            },
            Folder {
                name: "INBOX".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, thread_key)
         VALUES('acct', 'Mail/Entwürfe', '1', 1, 'draft', 'root@h'),
               ('acct', '[Gmail]/Drafts', '2', 2, 'draft', 'other@h'),
               ('acct', 'INBOX', '3', 3, 'not draft', 'inbox@h'),
               ('acct', 'Mail/Entwürfe', '4', 7, 'unthreaded draft', '')",
        [],
    )
    .unwrap();

    let keys = draft_thread_keys(&conn, "acct").unwrap();
    assert!(keys.contains("root@h"));
    assert!(keys.contains("other@h"));
    assert!(!keys.contains("inbox@h"));
    // A draft with no threading headers falls back to its own UID, which is only
    // unique within its mailbox — badging every `uid:7` card in the account with
    // it would mark unrelated mail as having a draft.
    assert!(!keys.contains("uid:7"));
    assert!(!keys.contains(""));
}

#[test]
fn rename_folder_keeps_the_rows_and_drops_the_cache() {
    let conn = test_conn();
    upsert_folders(
        &conn,
        "acct",
        &[
            Folder {
                name: "Work".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            Folder {
                name: "Work/Reports".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            Folder {
                name: "Workshop".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    set_folder_notify(&conn, "acct", "Work/Reports", true).unwrap();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, date, seen)
         VALUES('acct', 'Work/Reports', '7', 7, 'Q3', 1779580800, 0)",
        [],
    )
    .unwrap();
    set_folder_state(&conn, "acct", "Work/Reports", 1, 8).unwrap();

    let pairs = folder_rename_pairs(&conn, "acct", "Work", "Jobs").unwrap();
    assert_eq!(
        pairs,
        vec![
            ("Work".to_string(), "Jobs".to_string()),
            ("Work/Reports".to_string(), "Jobs/Reports".to_string()),
        ]
    );
    rename_folder(&conn, "acct", &pairs).unwrap();
    let folders = get_folders(&conn, "acct").unwrap();
    assert_eq!(
        folders
            .iter()
            .map(|folder| folder.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Jobs", "Jobs/Reports", "Workshop"]
    );
    let reports = folders
        .iter()
        .find(|folder| folder.name == "Jobs/Reports")
        .unwrap();
    assert!(reports.notify);
    assert_eq!(reports.delimiter.as_deref(), Some("/"));
    assert_eq!(
        get_folder_state(&conn, "acct", "Work/Reports").unwrap(),
        None
    );
    assert_eq!(get_folder_unread(&conn, "acct", "Work/Reports").unwrap(), 0);
}
