//! Regression tests for ai sort.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;
use crate::ai_sort::SortTarget;

fn arrival(uid: u32) -> imap::MessageHeader {
    imap::MessageHeader {
        uid,
        ..Default::default()
    }
}

fn decision(uid: u32, target: Option<&str>) -> AiSortDecision {
    AiSortDecision {
        candidate: AiSortCandidate {
            uid,
            identity: format!("message-id:{uid}@shop.example"),
            ..Default::default()
        },
        target: target.map(|folder| SortTarget::Server(folder.to_string())),
        why: String::new(),
    }
}

#[test]
fn ai_moved_mail_does_not_alert_until_its_move_fails_for_good() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    store::set_folder_state(&conn, "acct", "INBOX", 7, 10).unwrap();
    for uid in [1, 2] {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen)
             VALUES('acct', 'INBOX', ?1, ?1, 'Hi', 'Shop', 'orders@shop.test', 1779580800, 0)",
            rusqlite::params![uid],
        )
        .unwrap();
    }
    let recorded = store::ai_sort_record_decisions(
        &conn,
        "acct",
        "INBOX",
        7,
        &[decision(1, Some("Receipts")), decision(2, None)],
        0,
        &|| true,
    )
    .unwrap()
    .unwrap();
    assert_eq!(recorded.moving, [1]);

    // The move is still queued, and the mail it carries doesn't alert.
    let silenced = recorded.moving.iter().copied().collect();
    let mut arrivals = vec![arrival(1), arrival(2)];
    settle_arrivals(&mut arrivals, &silenced, Vec::new());
    assert_eq!(arrivals.iter().map(|it| it.uid).collect::<Vec<_>>(), [2]);

    // Every try used up: the queue gives the move up, and the mail alerts once.
    for _ in 0..5 {
        store::retry_rule_actions(&conn, "acct", "INBOX", &[1], 0, "gone").unwrap();
    }
    let late = store::ai_sort_take_alerts(&conn, "acct", "INBOX").unwrap();
    let mut arrivals = Vec::new();
    settle_arrivals(&mut arrivals, &BTreeSet::new(), late);
    assert_eq!(arrivals.iter().map(|it| it.uid).collect::<Vec<_>>(), [1]);
    assert!(store::ai_sort_take_alerts(&conn, "acct", "INBOX").unwrap().is_empty());
}
