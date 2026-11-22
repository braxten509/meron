//! Mailbox views grouped by correspondent.
//! Fixtures include inbox, drafts and sent copies for the same person.
//! The cases verify paging, unread filters and the scope of deletion.

use super::*;

#[test]
fn sender_pages_never_repeat_a_person() {
    let conn = sender_test_conn();
    // Five people, three messages each, interleaved in time; two share a
    // newest date so the address tiebreak is exercised at a page edge.
    let people = ["a@x.com", "b@x.com", "c@x.com", "d@x.com", "e@x.com"];
    let mut uid = 0;
    for round in 0..3 {
        for (idx, person) in people.iter().enumerate() {
            uid += 1;
            let date = if round == 2 && idx >= 3 {
                1000
            } else {
                (round * 10 + idx) as i64
            };
            // Recent enough for the Inbox to list: only the order matters.
            let date = fresh_date() - 2000 + date;
            insert_sender_row(
                &conn,
                "INBOX",
                uid,
                person,
                "me@example.com",
                date,
                true,
                "",
            );
        }
    }
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let (groups, next) =
            get_sender_page(&conn, "acct", "INBOX", 2, cursor, RecentFilter::default()).unwrap();
        for group in &groups {
            assert_eq!(group.message_count, 3, "{}", group.who);
            seen.push(group.who.clone());
        }
        match next {
            Some(next) => cursor = Some(parse_sender_cursor(&next).unwrap()),
            None => break,
        }
    }
    assert_eq!(
        seen,
        ["d@x.com", "e@x.com", "c@x.com", "b@x.com", "a@x.com"]
    );
}

#[test]
fn sender_delete_resolves_this_folders_mail_and_keeps_sent_copies() {
    let conn = sender_test_conn();
    insert_sender_row(
        &conn,
        "INBOX",
        1,
        "Bob@X.com",
        "me@example.com",
        10,
        true,
        "b1@h",
    );
    insert_sender_row(
        &conn,
        "INBOX",
        2,
        "bob@x.com",
        "me@example.com",
        30,
        false,
        "b2@h",
    );
    insert_sender_row(
        &conn,
        "INBOX",
        3,
        "carol@x.com",
        "me@example.com",
        20,
        true,
        "c1@h",
    );
    insert_sender_row(
        &conn,
        "Sent",
        4,
        "me@example.com",
        "bob@x.com",
        20,
        true,
        "s1@h",
    );
    insert_sender_row(
        &conn,
        "Archive",
        5,
        "bob@x.com",
        "me@example.com",
        5,
        true,
        "a1@h",
    );
    let key = sender_thread_key("bob@x.com");
    let uids = resolve_message_uids(&conn, "acct", "INBOX", &key, None, None, &[]).unwrap();
    assert_eq!(uids, vec![1, 2]);
    // Mark read on open reaches the same messages, not the Sent copy.
    let by_folder = thread_uids_by_folder(&conn, "acct", "INBOX", &key, None, false).unwrap();
    assert_eq!(
        by_folder,
        BTreeMap::from([("INBOX".to_string(), vec![1, 2])])
    );
    // In Sent, the row is the recipient's, and only Sent mail is acted on.
    let sent = resolve_message_uids(&conn, "acct", "Sent", &key, None, None, &[]).unwrap();
    assert_eq!(sent, vec![4]);
}

#[test]
fn sender_unread_filter_keeps_people_with_unread_mail() {
    let conn = sender_test_conn();
    // Recent enough for the Inbox to list.
    let day = fresh_date();
    insert_sender_row(
        &conn,
        "INBOX",
        1,
        "read@x.com",
        "me@example.com",
        day + 10,
        true,
        "",
    );
    insert_sender_row(
        &conn,
        "INBOX",
        2,
        "mixed@x.com",
        "me@example.com",
        day + 20,
        false,
        "",
    );
    insert_sender_row(
        &conn,
        "INBOX",
        3,
        "mixed@x.com",
        "me@example.com",
        day + 30,
        true,
        "",
    );
    let unread = RecentFilter {
        unread_only: true,
        attachments_only: false,
    };
    let (groups, next) = get_sender_page(&conn, "acct", "INBOX", 50, None, unread).unwrap();
    assert!(next.is_none());
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].who, "mixed@x.com");
    assert_eq!(groups[0].unread_count, 1);
    assert_eq!(groups[0].message_count, 2);
    assert_eq!(
        groups[0].newest.uid, 3,
        "the row shows their newest message"
    );
}

#[test]
fn sender_chat_lists_this_folder_sent_and_drafts_oldest_first() {
    let conn = sender_test_conn();
    insert_sender_row(
        &conn,
        "INBOX",
        1,
        "bob@x.com",
        "me@example.com",
        10,
        true,
        "b1@h",
    );
    // Mail to Bob that copied me: an Inbox copy and a Sent copy, one message.
    insert_sender_row(
        &conn,
        "INBOX",
        2,
        "me@example.com",
        "bob@x.com",
        20,
        true,
        "self@h",
    );
    insert_sender_row(
        &conn,
        "Sent",
        7,
        "me@example.com",
        "bob@x.com",
        20,
        true,
        "self@h",
    );
    insert_sender_row(
        &conn,
        "Sent",
        8,
        "me@example.com",
        "bob@x.com",
        30,
        true,
        "s2@h",
    );
    insert_sender_row(
        &conn,
        "Drafts",
        9,
        "me@example.com",
        "bob@x.com",
        40,
        true,
        "d1@h",
    );
    // Not in the chat: another folder, and other people.
    insert_sender_row(
        &conn,
        "Archive",
        3,
        "bob@x.com",
        "me@example.com",
        15,
        true,
        "a1@h",
    );
    insert_sender_row(
        &conn,
        "INBOX",
        4,
        "carol@x.com",
        "me@example.com",
        25,
        true,
        "c1@h",
    );
    insert_sender_row(
        &conn,
        "Sent",
        5,
        "me@example.com",
        "carol@x.com",
        26,
        true,
        "s3@h",
    );
    let chat = sender_chat_headers(&conn, "acct", "INBOX", "bob@x.com").unwrap();
    let got: Vec<(&str, u32)> = chat.iter().map(|h| (h.folder.as_str(), h.uid)).collect();
    assert_eq!(
        got,
        [("INBOX", 1), ("INBOX", 2), ("Sent", 8), ("Drafts", 9)]
    );
}
