//! Regression tests for ai sort.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;

fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    super::super::run_migrations(&conn).unwrap();
    conn
}

#[test]
fn a_device_keeps_one_claim_id_of_its_own() {
    let conn = test_conn();
    let id = ai_sort_claim_id(&conn).unwrap();
    assert_eq!(id.len(), 8);
    assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    // Made once: the same on every later ask.
    assert_eq!(ai_sort_claim_id(&conn).unwrap(), id);
    // Another device makes its own.
    assert_ne!(ai_sort_claim_id(&test_conn()).unwrap(), id);
    // Not a setting: settings travel with backups and setup copies.
    let settings: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM settings WHERE value LIKE ?1",
            [format!("%{id}%")],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(settings, 0);
}

fn insert_unread(conn: &Connection, uid: u32) {
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen)
         VALUES('acct', 'INBOX', ?1, ?1, 'Your order', 'Shop', 'orders@shop.example', ?2, 0)",
        // Recent: the Inbox leaves out mail past its age limit.
        params![uid, super::super::now_unix() - 3600],
    )
    .unwrap();
}

fn header(uid: u32, message_id: &str) -> MessageHeader {
    MessageHeader {
        uid,
        subject: "Your order".into(),
        from_addr: "orders@shop.example".into(),
        message_id: message_id.into(),
        ..Default::default()
    }
}

#[test]
fn held_mail_is_out_of_the_list_and_counts_until_the_hold_runs_out() {
    let conn = test_conn();
    insert_unread(&conn, 1);
    let now = super::super::now_unix();
    let hide = |until: i64| {
        conn.execute(
            "INSERT OR REPLACE INTO ai_sort_hidden(account, folder, uid, uidvalidity, hide_until)
             VALUES('acct', 'INBOX', 1, 7, ?1)",
            params![until],
        )
        .unwrap();
    };
    let shown = || {
        let filter = super::super::RecentFilter::default();
        let (page, _) =
            super::super::get_recent_page(&conn, "acct", "INBOX", 50, None, filter).unwrap();
        let (senders, _) =
            super::super::get_sender_page(&conn, "acct", "INBOX", 50, None, filter).unwrap();
        let unread = super::super::get_folder_unread(&conn, "acct", "INBOX").unwrap();
        (page.len(), senders.len(), unread)
    };

    hide(now + 1_000);
    assert_eq!(shown(), (0, 0, 0));
    hide(now - 1);
    assert_eq!(shown(), (1, 1, 1));
}

#[test]
fn only_new_unsorted_inbox_mail_without_rule_work_is_sorted() {
    let conn = test_conn();
    let marked = MessageHeader {
        ai_sorted: true,
        ..header(2, "b@shop.example")
    };
    // Moved back once: decided on this device already.
    conn.execute(
        "INSERT INTO ai_sort_done(account, identity, at)
         VALUES('acct', 'message-id:c@shop.example', 1)",
        [],
    )
    .unwrap();
    let headers = [
        header(1, "a@shop.example"),
        marked,
        header(3, "c@shop.example"),
        header(4, "d@shop.example"),
    ];

    // The first sync of a folder has no new mail to give.
    let first = super::super::first_seen_messages(&conn, "acct", "INBOX", 0, 10, &headers).unwrap();
    assert!(first.is_empty());
    assert!(
        ai_sort_hold(&conn, "acct", "Archive", 7, &headers, 100)
            .unwrap()
            .is_empty()
    );

    let held = ai_sort_hold(&conn, "acct", "INBOX", 7, &headers, 100).unwrap();
    let uids: Vec<u32> = held.iter().map(|candidate| candidate.uid).collect();
    assert_eq!(uids, [1, 4]);

    // A rule queued work for 4: it is the rule's, not the AI's, and shows again.
    let action = RuleAction {
        folder: "INBOX".into(),
        uid: 4,
        uidvalidity: 7,
        mark_read: true,
        rule_ids: vec!["rule".into()],
        ..RuleAction::default()
    };
    super::super::rules::queue_rule_actions(&conn, "acct", &BTreeMap::from([(4, action)]), 0)
        .unwrap();
    let kept = ai_sort_skip_rule_actions(&conn, "acct", "INBOX", held).unwrap();
    assert_eq!(
        kept.iter()
            .map(|candidate| candidate.uid)
            .collect::<Vec<_>>(),
        [1]
    );
    let hidden: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM ai_sort_hidden WHERE uid = 4",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hidden, 0);
}
