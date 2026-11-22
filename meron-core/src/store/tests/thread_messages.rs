//! Selecting and ordering the messages in a conversation.
//! UIDs stay scoped to a folder or subject branch where required.
//! Cross-folder reads exercise deduplication and missing references.

use super::*;

#[test]
fn newest_thread_uids_names_only_the_latest_message() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, thread_key)
         VALUES('acct', 'INBOX', '1', 1, 'Topic', 'Them', 'them@example.com', 100, 1, 'topic-key'),
               ('acct', 'INBOX', '2', 2, 'Topic', 'Them', 'them@example.com', 300, 1, 'topic-key'),
               ('acct', 'INBOX', '3', 3, 'Topic', 'Them', 'them@example.com', 200, 1, 'topic-key')",
        params![],
    )
    .unwrap();

    // Marking a thread unread flags this one message, so the thread comes back
    // as a single unread message rather than an all-unread thread.
    let uids = newest_thread_uids(&conn, "acct", "INBOX", "topic-key", None).unwrap();

    assert_eq!(uids, vec![2]);
}

#[test]
fn newest_thread_uids_stays_within_a_subject_branch() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, thread_key)
         VALUES('acct', 'INBOX', '1', 1, 'Invoice', 'Them', 'them@example.com', 100, 1, 'root'),
               ('acct', 'INBOX', '2', 2, 'Re: Receipt', 'Them', 'them@example.com', 400, 1, 'root'),
               ('acct', 'INBOX', '3', 3, 'Re: Invoice', 'Them', 'them@example.com', 300, 1, 'root')",
        params![],
    )
    .unwrap();

    let uids = newest_thread_uids(&conn, "acct", "INBOX", "root", Some("Invoice")).unwrap();

    assert_eq!(uids, vec![3]);
}

#[test]
fn thread_uids_span_folders_and_can_skip_read_messages() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, thread_key)
         VALUES('acct', 'INBOX', '1', 1, 'Topic', 'Them', 'them@example.com', 100, 0, 'topic-key'),
               ('acct', 'INBOX', '2', 2, 'Topic', 'Them', 'them@example.com', 200, 1, 'topic-key'),
               ('acct', 'Archive', '3', 7, 'Re: Topic', 'Them', 'them@example.com', 300, 0, 'topic-key'),
               ('acct', 'Archive', '4', 8, 'Other', 'Them', 'them@example.com', 400, 0, 'other-key')",
        params![],
    )
    .unwrap();

    let by_folder = thread_uids_by_folder(&conn, "acct", "INBOX", "topic-key", None, true).unwrap();

    assert_eq!(
        by_folder.into_iter().collect::<Vec<_>>(),
        vec![
            ("Archive".to_string(), vec![7]),
            ("INBOX".to_string(), vec![1])
        ]
    );
}

#[test]
fn thread_uids_keep_uid_keys_folder_local() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, thread_key)
         VALUES('acct', 'INBOX', '1', 5, 'One', 'Them', 'them@example.com', 100, 0, ''),
               ('acct', 'Archive', '2', 5, 'Two', 'Them', 'them@example.com', 200, 0, '')",
        params![],
    )
    .unwrap();

    let by_folder = thread_uids_by_folder(&conn, "acct", "INBOX", "uid:5", None, false).unwrap();

    assert_eq!(
        by_folder.into_iter().collect::<Vec<_>>(),
        vec![("INBOX".to_string(), vec![5])]
    );
}

#[test]
fn newest_thread_uids_is_empty_for_an_unknown_thread() {
    let conn = test_conn();

    assert!(
        newest_thread_uids(&conn, "acct", "INBOX", "missing", None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn get_thread_headers_all_folders_sorts_chronologically_by_parsed_date() {
    let conn = test_conn();
    // Insert via the real RFC 2822 parser (the ingestion path), so this also
    // covers parse_date_to_epoch; ordering is then done by the stored epoch.
    let d = |s: &str| crate::parse::parse_date_to_epoch(s);
    conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, thread_key)
             VALUES('acct', 'Sent', '2', 2, 'Re: Topic', 'Me', 'me@example.com', ?1, 1, 'topic-key'),
                   ('acct', 'INBOX', '3', 3, 'Re: Topic', 'Them', 'them@example.com', ?2, 1, 'topic-key'),
                   ('acct', 'INBOX', '1', 1, 'Topic', 'Them', 'them@example.com', ?3, 1, 'topic-key')",
            params![
                d("Fri, 22 May 2026 21:05:20 -0700"),
                d("Fri, 22 May 2026 21:13:27 -0700"),
                d("Thu, 21 May 2026 22:02:02 -0700"),
            ],
        )
        .unwrap();

    let headers = get_thread_headers_all_folders(&conn, "acct", "topic-key").unwrap();
    let uids: Vec<u32> = headers.iter().map(|h| h.uid).collect();
    assert_eq!(uids, vec![1, 2, 3]);
}

#[test]
fn get_thread_headers_all_folders_dedupes_self_sent_by_message_id() {
    let conn = test_conn();
    // A message sent to yourself lands in both Sent and the Inbox with the
    // same RFC Message-ID but distinct per-folder UIDs. The thread view must
    // collapse the pair into one bubble.
    conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, thread_key, json)
             VALUES('acct', '[Gmail]/Sent Mail', '462', 462, 'test', 'Me', 'me@example.com', 100, 1, 'self-key',
                      '{\"message_id\":\"mid@host\"}'),
                   ('acct', 'INBOX', '1440', 1440, 'test', 'Me', 'me@example.com', 100, 0, 'self-key',
                      '{\"message_id\":\"mid@host\"}'),
                   ('acct', 'INBOX', '1441', 1441, 'reply', 'Them', 'them@example.com', 200, 1, 'self-key',
                      '{\"message_id\":\"other@host\"}')",
            [],
        )
        .unwrap();

    let headers = get_thread_headers_all_folders(&conn, "acct", "self-key").unwrap();
    let uids: Vec<u32> = headers.iter().map(|h| h.uid).collect();
    // The self-sent pair collapses to the unread copy when one exists, so the
    // visible bubble can be marked read and clear the thread badge.
    assert_eq!(uids, vec![1440, 1441]);
}

#[test]
fn get_thread_headers_all_folders_keeps_rows_without_message_id() {
    let conn = test_conn();
    // Rows lacking a Message-ID (e.g. drafts) must never be collapsed into
    // each other — a NULL Message-ID never equates in SQL.
    conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, thread_key, json)
             VALUES('acct', 'Drafts', '10', 10, 'draft a', 'Me', 'me@example.com', 100, 'k', '{}'),
                   ('acct', 'Drafts', '11', 11, 'draft b', 'Me', 'me@example.com', 200, 'k', '{}')",
            [],
        )
        .unwrap();

    let headers = get_thread_headers_all_folders(&conn, "acct", "k").unwrap();
    let uids: Vec<u32> = headers.iter().map(|h| h.uid).collect();
    assert_eq!(uids, vec![10, 11]);
}

#[test]
fn thread_reference_gaps_are_referenced_ids_not_yet_cached() {
    let conn = test_conn();
    // A draft replying into a thread: thread_key is the (lowercased) root id;
    // its json carries its own Message-ID and the full References chain. The
    // referenced ancestors aren't cached, so they're all gaps — except the
    // one ancestor we also store as a cached row below.
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, date, thread_key, json)
             VALUES('acct', 'Drafts', '99', 99, 'Re: Hi', 0, 'root@h',
               '{\"message_id\":\"leaf@h\",\"references\":\"root@h MID-ONE@h mid-two@h\"}')",
        [],
    )
    .unwrap();
    // An already-cached ancestor (its own Message-ID present) must be excluded.
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, date, thread_key, json)
             VALUES('acct', 'INBOX', '50', 50, 'Hi', 0, 'root@h',
               '{\"message_id\":\"mid-two@h\",\"references\":\"root@h\"}')",
        [],
    )
    .unwrap();

    let mut gaps = get_thread_reference_gaps(&conn, "acct", "root@h").unwrap();
    gaps.sort();
    // root@h and mid-one@h are referenced but uncached; mid-two@h is present;
    // leaf@h is a Message-ID, never a gap. Comparison is case-insensitive.
    assert_eq!(gaps, vec!["mid-one@h".to_string(), "root@h".to_string()]);
}
