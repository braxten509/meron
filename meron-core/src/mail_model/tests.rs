//! Regression tests for mail model.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;
use std::slice;

#[test]
fn allocated_message_ids_use_account_domain_and_lifecycle_prefix() {
    let draft = allocate_message_id("me@example.com", true);
    let outgoing = allocate_message_id("me@example.com", false);
    assert!(draft.starts_with("meron-draft-"));
    assert!(outgoing.starts_with("meron-"));
    assert!(!outgoing.starts_with("meron-draft-"));
    assert!(draft.ends_with("@example.com"));
    assert!(outgoing.ends_with("@example.com"));
    assert_ne!(draft, outgoing);
}

#[test]
fn starred_cards_stop_at_the_budget_even_within_one_root_thread() {
    // Expanding a starred hit re-reads its whole root thread, so a root that
    // branched by subject hands back branches the budget never admitted.
    // Those must not ride along: the budget is what bounds both the work
    // here and the list the caller then pages through.
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 1,
                subject: "Sprint planning".to_string(),
                date: 300,
                starred: true,
                thread_key: "root@example.com".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 2,
                subject: "Lunch orders".to_string(),
                date: 200,
                starred: true,
                thread_key: "root@example.com".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();

    let one = starred_thread_cards(&conn, 1).unwrap();
    assert_eq!(one.len(), 1, "{one:?}");
    // The newest starred branch is the one kept. Card titles carry the root
    // thread's subject, the same for every branch, so the date names which
    // branch this is.
    assert_eq!(one[0]["date"], 300);

    let both = starred_thread_cards(&conn, 2).unwrap();
    assert_eq!(both.len(), 2, "{both:?}");
}

// A search page is a slice of the merged mailbox+Sent results. When the
// thread's copy in the searched mailbox sits below the page — an old hit,
// pushed down by the reply that was just sent — the page alone would leave
// the card on Sent, minting a second id for a thread the list already shows
// under the mailbox's.
#[test]
fn a_search_card_keeps_the_searched_mailbox_even_when_its_copy_paged_out() {
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::ensure_folder(&conn, "me@example.com", "Sent").unwrap();
    let inbox_copy = MessageHeader {
        uid: 1,
        folder: "INBOX".to_string(),
        subject: "Topic".to_string(),
        date: 100,
        thread_key: "root@example.com".to_string(),
        ..Default::default()
    };
    let sent_reply = MessageHeader {
        uid: 9,
        folder: "Sent".to_string(),
        subject: "Re: Topic".to_string(),
        date: 300,
        thread_key: "root@example.com".to_string(),
        ..Default::default()
    };
    store::upsert_messages(&conn, "me@example.com", "INBOX", &[inbox_copy]).unwrap();
    store::upsert_messages(&conn, "me@example.com", "Sent", &[sent_reply.clone()]).unwrap();

    // Only the reply made the page.
    let cards = thread_cards_json(
        &conn,
        "me@example.com",
        "INBOX",
        vec![sent_reply],
        &HashSet::new(),
    )
    .unwrap();

    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["folder_id"], "INBOX");
    assert_eq!(
        cards[0]["thread_id"],
        format_thread_id("me@example.com", "INBOX", "root@example.com#Topic")
    );
    // Both copies are one thread, page or no page — and the title is the
    // cached Inbox root's, not the reply that was all the page carried.
    assert_eq!(cards[0]["message_count"], 2);
    assert_eq!(cards[0]["subject"], "Topic");
}

// A thread several people replied to lists each of them once, oldest first,
// with the account's own messages as one "me" — not just the newest sender.
#[test]
fn a_card_lists_every_distinct_sender_oldest_first() {
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::ensure_folder(&conn, "me@example.com", "Sent").unwrap();
    let header = |uid: u32, date: i64, name: &str, addr: &str| MessageHeader {
        uid,
        folder: "INBOX".to_string(),
        subject: "Topic".to_string(),
        date,
        thread_key: "root@example.com".to_string(),
        from_name: name.to_string(),
        from_addr: addr.to_string(),
        message_id: format!("<{uid}@example.com>"),
        ..Default::default()
    };
    let inbox = vec![
        header(1, 100, "Alice Smith", "alice@example.com"),
        header(3, 300, "", "bob@example.com"),
        header(4, 400, "Alice Smith", "Alice@Example.com"),
    ];
    let mut sent = header(2, 200, "Me", "me@example.com");
    sent.folder = "Sent".to_string();
    store::upsert_messages(&conn, "me@example.com", "INBOX", &inbox).unwrap();
    store::upsert_messages(&conn, "me@example.com", "Sent", &[sent]).unwrap();

    let cards = thread_cards_json(
        &conn,
        "me@example.com",
        "INBOX",
        vec![inbox[2].clone()],
        &HashSet::new(),
    )
    .unwrap();

    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0]["senders"],
        json!([
            { "name": "Alice", "me": false },
            { "name": "", "me": true },
            { "name": "bob", "me": false },
        ])
    );
    assert_eq!(cards[0]["senders_truncated"], false);

    // Two more voices: the opener stays, the middle collapses into the gap.
    let more = vec![
        header(5, 500, "Carol King", "carol@example.com"),
        header(6, 600, "Dave", "dave@example.com"),
    ];
    store::upsert_messages(&conn, "me@example.com", "INBOX", &more).unwrap();
    let cards = thread_cards_json(
        &conn,
        "me@example.com",
        "INBOX",
        vec![more[1].clone()],
        &HashSet::new(),
    )
    .unwrap();
    assert_eq!(
        cards[0]["senders"],
        json!([
            { "name": "Alice", "me": false },
            { "name": "Carol", "me": false },
            { "name": "Dave", "me": false },
        ])
    );
    assert_eq!(cards[0]["senders_truncated"], true);

    // Bob, an early voice, sends the latest reply: he is one of the two most
    // recent, not elided for having joined before Carol.
    let bob_again = header(7, 700, "", "bob@example.com");
    store::upsert_messages(&conn, "me@example.com", "INBOX", &[bob_again.clone()]).unwrap();
    let cards = thread_cards_json(
        &conn,
        "me@example.com",
        "INBOX",
        vec![bob_again],
        &HashSet::new(),
    )
    .unwrap();
    assert_eq!(
        cards[0]["senders"],
        json!([
            { "name": "Alice", "me": false },
            { "name": "Dave", "me": false },
            { "name": "bob", "me": false },
        ])
    );
    assert_eq!(cards[0]["senders_truncated"], true);
}

// Tracker notifications all come from one address, each under the commenter's
// name. Those are different people, not one sender.
#[test]
fn senders_sharing_an_address_are_told_apart_by_name() {
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    let header = |uid: u32, name: &str| MessageHeader {
        uid,
        folder: "INBOX".to_string(),
        subject: "Re: [org/repo] Issue".to_string(),
        date: i64::from(uid) * 100,
        thread_key: "issue@tracker.example".to_string(),
        from_name: name.to_string(),
        from_addr: "notifications@tracker.example".to_string(),
        message_id: format!("<{uid}@tracker.example>"),
        ..Default::default()
    };
    let inbox = vec![
        header(1, "Carol Diaz"),
        header(2, "devuser42"),
        header(3, "Dana Evans"),
    ];
    store::upsert_messages(&conn, "me@example.com", "INBOX", &inbox).unwrap();

    let cards = thread_cards_json(
        &conn,
        "me@example.com",
        "INBOX",
        vec![inbox[2].clone()],
        &HashSet::new(),
    )
    .unwrap();

    assert_eq!(
        cards[0]["senders"],
        json!([
            { "name": "Carol", "me": false },
            { "name": "devuser42", "me": false },
            { "name": "Dana", "me": false },
        ])
    );
}

// One person writing once with a display name and once without is still one
// sender — but on a shared notification address, a nameless message cannot
// be credited to any one of several named participants.
#[test]
fn a_nameless_message_joins_its_address_only_when_one_name_is_known() {
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    // Each thread's key doubles as its subject, to find its card by.
    let header = |uid: u32, key: &str, name: &str, addr: &str| MessageHeader {
        uid,
        folder: "INBOX".to_string(),
        subject: key.to_string(),
        date: i64::from(uid) * 100,
        thread_key: key.to_string(),
        from_name: name.to_string(),
        from_addr: addr.to_string(),
        message_id: format!("<{uid}@example.com>"),
        ..Default::default()
    };
    let messages = vec![
        // Alice's nameless message comes first; the card still says "Alice".
        header(1, "a@example.com", "", "alice@example.com"),
        header(2, "a@example.com", "Bob", "bob@example.com"),
        header(3, "a@example.com", "Alice Smith", "alice@example.com"),
        header(
            4,
            "gh@example.com",
            "Carol Diaz",
            "notifications@tracker.example",
        ),
        header(5, "gh@example.com", "", "notifications@tracker.example"),
        header(
            6,
            "gh@example.com",
            "devuser42",
            "notifications@tracker.example",
        ),
    ];
    store::upsert_messages(&conn, "me@example.com", "INBOX", &messages).unwrap();

    let cards = thread_cards_json(
        &conn,
        "me@example.com",
        "INBOX",
        vec![messages[2].clone(), messages[5].clone()],
        &HashSet::new(),
    )
    .unwrap();

    let senders = |subject: &str| {
        cards
            .iter()
            .find(|card| card["subject"] == subject)
            .unwrap()["senders"]
            .clone()
    };
    assert_eq!(
        senders("a@example.com"),
        json!([{ "name": "Alice", "me": false }, { "name": "Bob", "me": false }])
    );
    assert_eq!(
        senders("gh@example.com"),
        json!([
            { "name": "Carol", "me": false },
            { "name": "notifications", "me": false },
            { "name": "devuser42", "me": false },
        ])
    );
}

// A `uid:` key names a message by (folder, uid) — the fallback for one with
// no threading headers — so a page that spans folders can carry two
// unrelated cards under `uid:7`. Neither may answer for the other.
#[test]
fn same_uid_cards_in_two_folders_keep_their_own_titles_and_counts() {
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::ensure_folder(&conn, "me@example.com", "Sent").unwrap();
    let inbox = MessageHeader {
        uid: 7,
        folder: "INBOX".to_string(),
        subject: "Inbox topic".to_string(),
        date: 100,
        ..Default::default()
    };
    let sent = MessageHeader {
        uid: 7,
        folder: "Sent".to_string(),
        subject: "Sent topic".to_string(),
        date: 300,
        ..Default::default()
    };
    store::upsert_messages(&conn, "me@example.com", "INBOX", slice::from_ref(&inbox)).unwrap();
    store::upsert_messages(&conn, "me@example.com", "Sent", slice::from_ref(&sent)).unwrap();

    let cards = thread_cards_json(
        &conn,
        "me@example.com",
        "INBOX",
        vec![sent, inbox],
        &HashSet::new(),
    )
    .unwrap();

    assert_eq!(cards.len(), 2, "{cards:?}");
    let title = |folder: &str| {
        cards
            .iter()
            .find(|card| card["folder_id"] == folder)
            .unwrap_or_else(|| panic!("no card in {folder}: {cards:?}"))["subject"]
            .clone()
    };
    assert_eq!(title("INBOX"), "Inbox topic");
    assert_eq!(title("Sent"), "Sent topic");
}

#[test]
fn folder_delete_gate_requires_a_synced_ordinary_folder() {
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::upsert_folders(
        &conn,
        "me@example.com",
        &[
            crate::imap::Folder {
                name: "Projects".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Projects/2026".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Projects/2026/Q1".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Localized Sent".to_string(),
                delimiter: Some("/".to_string()),
                special_use: Some("sent".to_string()),
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Mail".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Mail/Archive".to_string(),
                delimiter: Some("/".to_string()),
                special_use: Some("archive".to_string()),
                ..Default::default()
            },
        ],
    )
    .unwrap();

    // A parent goes with its subfolders, so nesting is no bar on its own.
    assert!(check_folder_deletable(&conn, "me@example.com", "Projects").is_ok());
    assert!(
        check_folder_deletable(&conn, "me@example.com", "Localized Sent")
            .unwrap_err()
            .contains("special folder")
    );
    // ...but a special-use folder buried in the subtree still blocks it.
    assert!(
        check_folder_deletable(&conn, "me@example.com", "Mail")
            .unwrap_err()
            .contains("Mail/Archive is a special folder")
    );
    assert!(
        check_folder_deletable(&conn, "me@example.com", "Uncached")
            .unwrap_err()
            .contains("synchronized folder list")
    );
}

#[test]
fn folder_delete_targets_list_the_subtree_deepest_first() {
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::upsert_folders(
        &conn,
        "me@example.com",
        &[
            crate::imap::Folder {
                name: "Projects".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Projects/2026".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Projects/2026/Q1".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Projects2".to_string(),
                delimiter: Some("/".to_string()),
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Flat".to_string(),
                delimiter: None,
                ..Default::default()
            },
            crate::imap::Folder {
                name: "Flat/Independent".to_string(),
                delimiter: None,
                ..Default::default()
            },
        ],
    )
    .unwrap();

    // Every child comes before its own parent, and the lookalike sibling
    // that merely shares a name prefix stays out of it.
    assert_eq!(
        folder_delete_targets(&conn, "me@example.com", "Projects").unwrap(),
        vec![
            "Projects/2026/Q1".to_string(),
            "Projects/2026".to_string(),
            "Projects".to_string(),
        ]
    );
    assert_eq!(
        folder_delete_targets(&conn, "me@example.com", "Projects/2026/Q1").unwrap(),
        vec!["Projects/2026/Q1".to_string()]
    );
    // A NIL hierarchy delimiter makes slash ordinary name punctuation.
    assert_eq!(
        folder_delete_targets(&conn, "me@example.com", "Flat").unwrap(),
        vec!["Flat".to_string()]
    );
}

#[test]
fn starred_page_filters_and_pages_with_an_opaque_cursor() {
    let items = vec![
        json!({"id":"three","date":300,"subject":"Other"}),
        json!({"id":"two","date":200,"subject":"Design review"}),
        json!({"id":"one","date":100,"subject":"Design notes"}),
    ];
    let first = starred_page(items.clone(), "design", "all", false, 1, None);
    assert_eq!(first["items"][0]["id"], "two");
    let cursor = first["next_cursor"].as_str().unwrap();
    assert!(cursor.starts_with("starred:"));
    let second = starred_page(items, "design", "all", false, 1, Some(cursor));
    assert_eq!(second["items"][0]["id"], "one");
    assert!(second.get("next_cursor").is_none());
}

#[test]
fn starred_page_filters_unread_before_paging() {
    let items = vec![
        json!({"id":"read","date":300,"unread":false}),
        json!({"id":"unread-two","date":200,"unread":true}),
        json!({"id":"unread-one","date":100,"unread":true}),
    ];
    let first = starred_page(items.clone(), "", "unread", false, 1, None);
    assert_eq!(first["items"][0]["id"], "unread-two");
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = starred_page(items, "", "unread", false, 1, Some(cursor));
    assert_eq!(second["items"][0]["id"], "unread-one");
    assert!(second.get("next_cursor").is_none());
}

#[test]
fn starred_page_combines_unread_with_attachments() {
    let items = vec![
        json!({"id":"plain","date":300,"unread":true,"has_attachments":false}),
        json!({"id":"files","date":200,"unread":true,"has_attachments":true}),
        json!({"id":"read-files","date":100,"unread":false,"has_attachments":true}),
    ];
    let page = starred_page(items, "", "unread", true, 10, None);
    let ids: Vec<&str> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert_eq!(ids, vec!["files"]);
}

#[test]
fn mutation_result_reports_authoritative_counts_and_folder_roles() {
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::ensure_folder(&conn, "me@example.com", "Archive").unwrap();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, date, seen)
         VALUES('me@example.com', 'INBOX', 'one', 1, 'One', ?1 + 1, 0),
               ('me@example.com', 'Archive', 'two', 2, 'Two', ?1, 0)",
        // Recent: the Inbox leaves out mail past its age limit.
        [store::now_unix() - 3600],
    )
    .unwrap();

    let result = mutation_result(
        json!({ "ok": true }),
        &conn,
        "me@example.com",
        "me@example.com#INBOX#thread",
        "INBOX",
        Some("Archive"),
        Some(false),
        None,
        true,
    )
    .unwrap();

    assert_eq!(result["folder_unreads"]["me@example.com"]["INBOX"], 1);
    assert_eq!(result["folder_unreads"]["me@example.com"]["Archive"], 1);
    assert_eq!(result["change"]["source_role"], "inbox");
    assert_eq!(result["change"]["target_role"], "archive");
    assert_eq!(result["change"]["removed"], true);
    assert_eq!(result["folder_counts"].as_array().unwrap().len(), 2);
}
