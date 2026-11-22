//! Exercises store behavior for starred.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn get_starred_all_accounts_spans_accounts_and_skips_rss_rows() {
    let conn = test_conn();
    let insert = |account: &str, folder: &str, uid: u32, subject: &str, date: i64, starred: i64| {
        conn.execute(
                "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, starred)
                 VALUES(?1, ?2, ?3, ?4, ?5, 'Aki', 'aki@example.com', ?6, 1, ?7)",
                params![account, folder, uid.to_string(), uid, subject, date, starred],
            )
            .unwrap();
    };
    insert("a1", "INBOX", 1, "Starred newer", 1_767_312_000, 1); // 2026-01-02
    insert("a1", "INBOX", 2, "Not starred", 1_767_398_400, 0); // 2026-01-03
    insert("a2", "Sent", 5, "Starred older", 1_767_225_600, 1); // 2026-01-01
    // RSS row: uid = 0, must be excluded even when starred.
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, date, seen, starred)
             VALUES('rss-acct', 'feed-1', 'item-1', 0, 'Feed item', 0, 1, 1)",
        [],
    )
    .unwrap();

    let starred = get_starred_all_accounts(&conn, 10).unwrap();
    assert_eq!(
        starred
            .iter()
            .map(|(account, m)| (account.as_str(), m.folder.as_str(), m.uid))
            .collect::<Vec<_>>(),
        vec![("a1", "INBOX", 1), ("a2", "Sent", 5)]
    );
    assert!(starred.iter().all(|(_, m)| m.starred));
}

#[test]
fn get_starred_all_accounts_budgets_threads_not_rows() {
    let conn = test_conn();
    let insert = |folder: &str, uid: u32, thread_key: &str, date: i64| {
        conn.execute(
                "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, starred, thread_key)
                 VALUES('a1', ?1, ?2, ?3, 'Subject', 'Aki', 'aki@example.com', ?4, 1, 1, ?5)",
                params![folder, format!("{folder}-{uid}"), uid, date, thread_key],
            )
            .unwrap();
    };
    // Two conversations, each cached in Inbox and All Mail the way Gmail files
    // them, plus an older third conversation.
    for (folder, uid_base) in [("INBOX", 10), ("All Mail", 20)] {
        insert(folder, uid_base + 1, "newest@example.com", 300);
        insert(folder, uid_base + 2, "middle@example.com", 200);
        insert(folder, uid_base + 3, "oldest@example.com", 100);
    }

    // A budget of two threads spends nothing on the duplicate folder copies: it
    // reaches back through both copies of the two newest conversations.
    let starred = get_starred_all_accounts(&conn, 2).unwrap();
    let mut keys = starred
        .iter()
        .map(|(_, m)| m.thread_key.as_str())
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys, vec!["middle@example.com", "newest@example.com"]);
    assert_eq!(starred.len(), 4);

    // Room for every conversation still returns every copy of each.
    assert_eq!(get_starred_all_accounts(&conn, 3).unwrap().len(), 6);
}

#[test]
fn get_starred_all_accounts_counts_unthreaded_messages_per_folder() {
    let conn = test_conn();
    // No thread_key at all: these fall back to "uid:<uid>", and a UID only means
    // something inside its own mailbox, so these are two conversations.
    for (folder, date) in [("INBOX", 200), ("Archive", 100)] {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, starred)
             VALUES('a1', ?1, ?2, 1, 'Subject', 'Aki', 'aki@example.com', ?3, 1, 1)",
            params![folder, format!("{folder}-1"), date],
        )
        .unwrap();
    }

    let one = get_starred_all_accounts(&conn, 1).unwrap();
    assert_eq!(
        one.iter()
            .map(|(_, m)| m.folder.as_str())
            .collect::<Vec<_>>(),
        vec!["INBOX"]
    );
    let both = get_starred_all_accounts(&conn, 2).unwrap();
    assert_eq!(both.len(), 2);
}

#[test]
fn get_starred_all_accounts_counts_subject_branches_as_separate_conversations() {
    let conn = test_conn();
    // One root thread key that branched by subject: the list shows a card per
    // branch, so the budget has to count them the same way.
    for (uid, subject, date) in [(1, "Sprint planning", 300), (2, "Lunch orders", 200)] {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, starred, thread_key)
             VALUES('a1', 'INBOX', ?1, ?2, ?3, 'Aki', 'aki@example.com', ?4, 1, 1, 'root@example.com')",
            params![format!("m-{uid}"), uid, subject, date],
        )
        .unwrap();
    }

    let one = get_starred_all_accounts(&conn, 1).unwrap();
    assert_eq!(
        one.iter()
            .map(|(_, m)| m.subject.as_str())
            .collect::<Vec<_>>(),
        vec!["Sprint planning"]
    );
    assert_eq!(get_starred_all_accounts(&conn, 2).unwrap().len(), 2);
}

#[test]
fn get_starred_all_accounts_keeps_copies_of_admitted_threads_past_the_budget() {
    let conn = test_conn();
    let insert = |folder: &str, uid: u32, thread_key: &str, date: i64| {
        conn.execute(
                "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, starred, thread_key)
                 VALUES('a1', ?1, ?2, ?3, 'Subject', 'Aki', 'aki@example.com', ?4, 1, 1, ?5)",
                params![folder, format!("{folder}-{uid}"), uid, date, thread_key],
            )
            .unwrap();
    };
    // Two folders' copies of one message can hold different cached dates, which
    // puts an over-budget conversation between them in this ordering.
    insert("All Mail", 1, "kept@example.com", 300);
    insert("INBOX", 2, "over-budget@example.com", 200);
    insert("INBOX", 3, "kept@example.com", 100);

    let starred = get_starred_all_accounts(&conn, 1).unwrap();
    assert!(
        starred
            .iter()
            .all(|(_, m)| m.thread_key == "kept@example.com"),
        "over-budget conversation must not be admitted"
    );
    // Both copies of the admitted conversation come back: the caller chooses
    // which folder's card to show from this set, and dropping the Inbox copy
    // here would file the thread under All Mail instead.
    let mut folders = starred
        .iter()
        .map(|(_, m)| m.folder.as_str())
        .collect::<Vec<_>>();
    folders.sort_unstable();
    assert_eq!(folders, vec!["All Mail", "INBOX"]);
}
