//! Grouping mailbox rows into conversation cards.
//! Subject branches retain their relationship to the root message.
//! The fixtures exercise date ordering and ties across folders.

use super::*;

#[test]
fn resolve_message_uids_prefers_explicit_then_single_then_thread() {
    let conn = test_conn();
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[
            MessageHeader {
                uid: 10,
                subject: "Root".to_string(),
                from_addr: "root@example.com".to_string(),
                thread_key: "thread-a".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 11,
                subject: "Reply".to_string(),
                from_addr: "reply@example.com".to_string(),
                thread_key: "thread-a".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();

    assert_eq!(
        resolve_message_uids(
            &conn,
            "acct",
            "INBOX",
            "thread-a",
            None,
            Some(10),
            &[42, 43]
        )
        .unwrap(),
        vec![42, 43]
    );
    assert_eq!(
        resolve_message_uids(&conn, "acct", "INBOX", "", None, Some(10), &[]).unwrap(),
        vec![10]
    );
    assert_eq!(
        resolve_message_uids(&conn, "acct", "INBOX", "", None, None, &[]).unwrap(),
        Vec::<u32>::new()
    );
    assert_eq!(
        resolve_message_uids(&conn, "acct", "INBOX", "thread-a", None, None, &[]).unwrap(),
        vec![10, 11]
    );
}

#[test]
fn thread_subject_normalization_matches_desktop_prefix_semantics() {
    // Parity with the Go subjectPrefixRegex alternation in mailjson.go: every
    // listed prefix must strip when followed by a colon, including the ones
    // that share a shorter list entry ("fwd" behind "fw", "res" behind "re").
    let cases = [
        ("Re: Topic", "Topic"),
        ("Fwd: Topic", "Topic"),
        ("FWD: Topic", "Topic"),
        ("Res: Topic", "Topic"),
        ("Re[2]: Topic", "Topic"),
        ("FW(3): Topic", "Topic"),
        ("回复：主题", "主题"),
        ("Re: Fwd: Topic", "Topic"),
        // Not reply prefixes: the colon check must reject these.
        ("Ready: set", "Ready: set"),
        ("Fwdish: nope", "Fwdish: nope"),
        ("Topic", "Topic"),
    ];
    for (subject, expected) in cases {
        assert_eq!(thread_grouping_subject(subject), expected, "{subject}");
    }
    // Leading bracket tags drop too.
    assert_eq!(thread_grouping_subject("[EXTERNAL] Re: Topic"), "Topic");
    assert_eq!(thread_grouping_subject("[JIRA-1] Topic"), "Topic");
}

#[test]
fn branch_compound_key_round_trips_hash_and_percent_in_root() {
    let cases = ["abc#123@host", "plain@host", "50%25#done@host"];
    for root in cases {
        let compound = branch_compound_key(root, "Topic");
        assert_eq!(
            split_branch_compound_key(&compound),
            (root.to_string(), Some("Topic".to_string())),
            "{root}"
        );
    }
    // Unbranched legacy keys pass through verbatim.
    assert_eq!(
        split_branch_compound_key("plain@host"),
        ("plain@host".to_string(), None)
    );
}

#[test]
fn group_thread_cards_branches_subject_drift_and_links_to_root() {
    use crate::imap::MessageHeader;
    let messages = vec![
        MessageHeader {
            uid: 3,
            subject: "New topic".to_string(),
            from_name: "Newest".to_string(),
            from_addr: "new@example.com".to_string(),
            date: 300,
            seen: false,
            thread_key: "refs-root".to_string(),
            ..Default::default()
        },
        MessageHeader {
            uid: 2,
            subject: "Re: Old topic".to_string(),
            from_name: "Reply".to_string(),
            from_addr: "reply@example.com".to_string(),
            date: 200,
            seen: true,
            thread_key: "refs-root".to_string(),
            ..Default::default()
        },
        MessageHeader {
            uid: 1,
            subject: "Old topic".to_string(),
            from_name: "Root".to_string(),
            from_addr: "root@example.com".to_string(),
            date: 100,
            seen: false,
            thread_key: "refs-root".to_string(),
            ..Default::default()
        },
    ];

    let cards = group_thread_cards(messages, "INBOX");
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].thread_key, "refs-root#New topic");
    assert_eq!(
        cards[0].original_thread_key.as_deref(),
        Some("refs-root#Old topic")
    );
    assert_eq!(cards[0].header.subject, "Old topic");
    assert_eq!(cards[0].unread_count, 1);
    // Counts follow the branch, not the root thread key: the drifted subject
    // stands alone while the original keeps its reply.
    assert_eq!(cards[0].message_count, 1);
    assert!(!cards[0].header.seen);

    assert_eq!(cards[1].thread_key, "refs-root#Old topic");
    assert_eq!(cards[1].original_thread_key, None);
    assert_eq!(cards[1].header.subject, "Old topic");
    assert_eq!(cards[1].unread_count, 1);
    assert_eq!(cards[1].message_count, 2);
}

#[test]
fn group_thread_cards_keep_reply_prefix_when_the_thread_start_is_missing() {
    use crate::imap::MessageHeader;
    // Only replies reached this mailbox, so the oldest message here is itself a
    // reply. The card must say so rather than pass the reply off as the thread's
    // opening message.
    let cards = group_thread_cards(
        vec![
            MessageHeader {
                uid: 2,
                subject: "Re: abdullah <> ping".to_string(),
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 1,
                subject: "Re: abdullah <> ping".to_string(),
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].header.subject, "Re: abdullah <> ping");
    // Grouping still ignores the prefix, so the branch key is the bare subject.
    assert_eq!(cards[0].thread_key, "refs-root#abdullah <> ping");
    assert_eq!(cards[0].message_count, 2);
}

#[test]
fn group_thread_cards_pick_the_root_by_date_across_folders() {
    use crate::imap::MessageHeader;
    // A folder-spanning page (search, or any thread the user replied to) merges
    // Inbox and Sent, whose UID spaces are unrelated: the Sent reply's UID 1 is
    // not older than the Inbox root's UID 100. Send time is what orders them.
    let cards = group_thread_cards(
        vec![
            MessageHeader {
                uid: 1,
                folder: "Sent".to_string(),
                subject: "Re: Topic".to_string(),
                date: 200,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 100,
                folder: "INBOX".to_string(),
                subject: "Topic".to_string(),
                date: 100,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].header.subject, "Topic");
}

#[test]
fn group_thread_cards_break_a_cross_folder_tie_with_the_read_mailbox() {
    use crate::imap::MessageHeader;
    // Neither copy carries a date, so send time cannot separate them. UID must
    // not step in — it is folder-local, and the Sent reply's UID 1 would win
    // against the Inbox root's UID 100. The mailbox being read owns the card.
    let cards = group_thread_cards(
        vec![
            MessageHeader {
                uid: 1,
                folder: "Sent".to_string(),
                subject: "Re: Topic".to_string(),
                date: 0,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 100,
                folder: "INBOX".to_string(),
                subject: "Topic".to_string(),
                date: 0,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].header.subject, "Topic");
}

#[test]
fn group_thread_cards_keep_uid_order_within_a_folder_when_a_date_is_missing() {
    use crate::imap::MessageHeader;
    // `date == 0` means unknown, not "epoch". Inside one folder UIDs already
    // order by arrival, so the undated root stays the root.
    let cards = group_thread_cards(
        vec![
            MessageHeader {
                uid: 2,
                subject: "Re: Topic".to_string(),
                date: 100,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 1,
                subject: "Topic".to_string(),
                date: 0,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].header.subject, "Topic");
}

#[test]
fn card_root_subjects_read_the_thread_start_the_page_left_out() {
    let conn = test_conn();
    let insert = |uid: u32, subject: &str, seen: i64| {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr,
                                  date, seen, thread_key)
             VALUES('acct', 'INBOX', ?1, ?2, ?3, 'Aki', 'aki@example.com', ?4, ?5,
                    'root@example.com')",
            params![uid.to_string(), uid, subject, 100 * uid as i64, seen],
        )
        .unwrap();
    };
    insert(1, "Topic", 1);
    insert(2, "Re: Topic", 0);

    // The unread view's page holds only the reply, so grouping alone would title
    // the card "Re: Topic". The cache still has the seen thread start.
    let subjects = card_root_subjects(
        &conn,
        "acct",
        "INBOX",
        &["root@example.com#Topic".to_string()],
    )
    .unwrap();
    assert_eq!(
        subjects.get("root@example.com").map(String::as_str),
        Some("Topic")
    );

    // A thread the cache has never stored answers nothing, leaving the caller
    // its own page-derived title.
    assert!(
        card_root_subjects(
            &conn,
            "acct",
            "INBOX",
            &["missing@example.com#Topic".to_string()]
        )
        .unwrap()
        .is_empty()
    );
}

#[test]
fn group_thread_cards_keeps_uid_and_gmail_threads_atomic() {
    use crate::imap::MessageHeader;
    let uid_cards = group_thread_cards(
        vec![
            MessageHeader {
                uid: 2,
                subject: "Other".to_string(),
                thread_key: "uid:2".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 1,
                subject: "Root".to_string(),
                thread_key: "uid:2".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );
    assert_eq!(uid_cards.len(), 1);
    assert_eq!(uid_cards[0].thread_key, "uid:2");
    // Read messages count too; the badge is thread size, not unread size.
    assert_eq!(uid_cards[0].message_count, 2);

    let gmail_cards = group_thread_cards(
        vec![
            MessageHeader {
                uid: 2,
                subject: "Other".to_string(),
                thread_key: "gmthrid:abc".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 1,
                subject: "Root".to_string(),
                thread_key: "gmthrid:abc".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );
    assert_eq!(gmail_cards.len(), 1);
    assert_eq!(gmail_cards[0].thread_key, "gmthrid:abc");
    assert_eq!(gmail_cards[0].message_count, 2);
}

#[test]
fn group_thread_cards_accumulates_unread_and_starred_across_group() {
    use crate::imap::MessageHeader;
    let cards = group_thread_cards(
        vec![
            MessageHeader {
                uid: 3,
                subject: "Re: [EXTERNAL] Topic".to_string(),
                seen: false,
                starred: false,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 2,
                subject: "[EXTERNAL] Topic".to_string(),
                seen: false,
                starred: true,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 1,
                subject: "Topic".to_string(),
                seen: true,
                starred: false,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].thread_key, "refs-root#Topic");
    assert_eq!(cards[0].header.subject, "Topic");
    assert_eq!(cards[0].unread_count, 2);
    assert!(cards[0].header.starred);
}

#[test]
fn group_thread_cards_marks_groups_with_cached_drafts() {
    use crate::imap::MessageHeader;
    let draft_keys = ["refs-root".to_string()].into_iter().collect();
    let cards = group_thread_cards_with_drafts(
        vec![
            MessageHeader {
                uid: 2,
                subject: "Re: Topic".to_string(),
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 1,
                subject: "Other".to_string(),
                thread_key: "other-root".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
        &draft_keys,
    );

    let marked = cards
        .iter()
        .find(|card| card.thread_key == "refs-root#Topic")
        .unwrap();
    let unmarked = cards
        .iter()
        .find(|card| card.thread_key == "other-root#Other")
        .unwrap();
    assert!(marked.has_draft);
    assert!(!unmarked.has_draft);
}

#[test]
fn group_thread_cards_count_a_self_sent_copy_once() {
    use crate::imap::MessageHeader;
    // A message the user sent to themselves is cached in both Sent and Inbox,
    // and a search page reads both. It is one message in the thread: the tally
    // the card carries — a floor under the cached count, which folds copies by
    // Message-ID — must not count it twice, or the search list shows a longer
    // thread than the mailbox's own list does.
    let copy = |folder: &str, uid: u32| MessageHeader {
        uid,
        folder: folder.to_string(),
        subject: "Re: Topic".to_string(),
        date: 300,
        seen: false,
        thread_key: "refs-root".to_string(),
        message_id: "<reply@example.com>".to_string(),
        ..Default::default()
    };
    let cards = group_thread_cards(
        vec![
            copy("Sent", 7),
            copy("INBOX", 9),
            MessageHeader {
                uid: 2,
                folder: "INBOX".to_string(),
                subject: "Topic".to_string(),
                date: 100,
                seen: true,
                thread_key: "refs-root".to_string(),
                message_id: "<root@example.com>".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].message_count, 2);
    assert_eq!(cards[0].unread_count, 1);
    assert!(!cards[0].header.seen);

    // The copies can disagree on the read flag: Sent's is read, the mailbox's is
    // not. The message is unread whichever copy the page lists first.
    for order in [
        [("Sent", 7, true), ("INBOX", 9, false)],
        [("INBOX", 9, false), ("Sent", 7, true)],
    ] {
        let page = order
            .into_iter()
            .map(|(folder, uid, seen)| MessageHeader {
                seen,
                ..copy(folder, uid)
            })
            .collect::<Vec<_>>();
        let cards = group_thread_cards(page, "INBOX");
        assert_eq!(cards[0].message_count, 1);
        assert_eq!(cards[0].unread_count, 1);
        assert!(!cards[0].header.seen);
    }

    // Without a Message-ID there is nothing to match a copy on, so each row
    // still counts for itself.
    let anonymous = |folder: &str, uid: u32| MessageHeader {
        uid,
        folder: folder.to_string(),
        subject: "Topic".to_string(),
        thread_key: "refs-root".to_string(),
        ..Default::default()
    };
    let cards = group_thread_cards(vec![anonymous("Sent", 7), anonymous("INBOX", 9)], "INBOX");
    assert_eq!(cards[0].message_count, 2);
}

#[test]
fn group_thread_cards_keep_the_searched_folder_when_a_page_spans_folders() {
    use crate::imap::MessageHeader;
    // A search page merges the mailbox with Sent, newest first. Replying puts
    // the Sent copy on top, and the card must still be the searched mailbox's
    // one: its thread id is what the list already has the thread under.
    let cards = group_thread_cards(
        vec![
            MessageHeader {
                uid: 7,
                folder: "Sent".to_string(),
                subject: "Re: Topic".to_string(),
                date: 300,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 2,
                folder: "INBOX".to_string(),
                subject: "Topic".to_string(),
                date: 100,
                thread_key: "refs-root".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].header.folder, "INBOX");
    // Only the identity moves: the card still shows the newest message.
    assert_eq!(cards[0].header.date, 300);
    assert_eq!(cards[0].message_count, 2);

    // A thread the searched mailbox has no copy of stays where it was found.
    let sent_only = group_thread_cards(
        vec![MessageHeader {
            uid: 7,
            folder: "Sent".to_string(),
            subject: "Topic".to_string(),
            thread_key: "refs-root".to_string(),
            ..Default::default()
        }],
        "INBOX",
    );
    assert_eq!(sent_only[0].header.folder, "Sent");

    // `uid:` keys name one message by (folder, uid). Two mailboxes reuse UIDs, so
    // such a key is only unique within its own folder: unrelated messages that
    // share one stay separate cards, and neither card's folder may move.
    let uid_keyed = group_thread_cards(
        vec![
            MessageHeader {
                uid: 7,
                folder: "Sent".to_string(),
                subject: "Reply to somebody".to_string(),
                thread_key: "uid:7".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 7,
                folder: "INBOX".to_string(),
                subject: "Unrelated mail".to_string(),
                seen: false,
                thread_key: "uid:7".to_string(),
                ..Default::default()
            },
        ],
        "INBOX",
    );
    assert_eq!(uid_keyed.len(), 2);
    assert_eq!(uid_keyed[0].header.folder, "Sent");
    assert_eq!(uid_keyed[0].message_count, 1);
    assert_eq!(uid_keyed[1].header.folder, "INBOX");
    assert_eq!(uid_keyed[1].unread_count, 1);
}
