//! Actions queued by rules for one or more accounts.
//! Fixtures exercise move, copy, star and folder resolution for an incoming message.
//! Retry and failure cases inspect the resulting queue and destination.

use super::*;

#[test]
fn a_copy_is_queued_unless_the_mail_is_already_there() {
    let conn = db();
    set_rules(
        &conn,
        vec![
            Rule {
                query: "subject:s".into(),
                copy_account: "b".into(),
                copy_folder: "Saved".into(),
                ..Default::default()
            },
            Rule {
                query: "subject:s".into(),
                copy_account: "a".into(),
                copy_folder: "INBOX".into(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);

    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    // Not to where the message already is.
    let copy = RuleCopy {
        account: "b".into(),
        folder: "Saved".into(),
    };
    assert_eq!(queued[0].copies, [copy.clone()]);
    assert!(!queued[0].trash && queued[0].target.is_empty());
}

#[test]
fn a_copy_to_where_the_mail_is_moving_is_left_to_the_move() {
    let conn = db();
    crate::store::ensure_folder(&conn, "a", "Saved").unwrap();
    set_rules(
        &conn,
        vec![
            Rule {
                query: "subject:s".into(),
                copy_account: "a".into(),
                copy_folder: "Saved".into(),
                ..Default::default()
            },
            Rule {
                query: "subject:s".into(),
                move_to: "folder".into(),
                folder: "Saved".into(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);

    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert_eq!(queued[0].target, "Saved");
    assert!(queued[0].copies.is_empty());
}

#[test]
fn a_copy_to_trash_is_left_to_the_move_there() {
    let conn = db();
    crate::store::upsert_folders(
        &conn,
        "a",
        &[crate::imap::Folder {
            name: "Bin".into(),
            role: "trash".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    set_rules(
        &conn,
        vec![Rule {
            query: "subject:s".into(),
            move_to: "trash".into(),
            copy_account: "a".into(),
            copy_folder: "Bin".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);

    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert!(queued[0].trash && queued[0].copies.is_empty());
}

#[test]
fn a_move_that_lost_to_another_rule_counts_as_a_failure() {
    let conn = db();
    let lost = crate::store::RuleAction {
        rule_ids: vec!["r".into()],
        ..Default::default()
    };
    crate::store::log_rule_lost_moves(&conn, "a", &[lost]).unwrap();
    let log = crate::store::rule_log(&conn).unwrap();
    assert_eq!(log[0].rule_id, "r");
    assert!(log[0].failed);
}

#[test]
fn a_move_to_another_account_is_a_copy_and_then_trash() {
    let conn = db();
    set_rules(
        &conn,
        vec![Rule {
            query: "subject:s".into(),
            move_to: "folder".into(),
            folder: "Saved".into(),
            folder_account: "b".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);

    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert!(queued[0].trash);
    assert_eq!(queued[0].target, "");
    assert_eq!(queued[0].copies.len(), 1);
    assert_eq!(queued[0].copies[0].account, "b");
}

#[test]
fn a_copy_needs_both_its_account_and_folder() {
    let conn = db();
    let refused = set_rules(
        &conn,
        vec![Rule {
            query: "subject:s".into(),
            copy_folder: "Saved".into(),
            ..Default::default()
        }],
    );
    assert!(refused.is_err());
}

#[test]
fn a_star_is_queued_and_a_tag_is_not() {
    let conn = db();
    let starred = Rule {
        query: "from:x".into(),
        star: true,
        ..Default::default()
    };
    let tagged = Rule {
        query: "subject:s".into(),
        tags: vec!["todo".into()],
        ..Default::default()
    };
    set_rules(&conn, vec![starred, tagged]).unwrap();
    let mut from_x = message(1, "m@x", Some(&[]));
    from_x.from_addr = "x@x.test".into();
    upsert(&conn, "INBOX", &[from_x, message(2, "n@x", Some(&[]))]);

    apply_rules(&conn, "a", "INBOX", 1, &[1, 2]).unwrap();
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert_eq!(queued.iter().map(|a| a.uid).collect::<Vec<_>>(), [1]);
    assert_eq!(tags(&conn, "n@x"), ["todo"]);
}

#[test]
fn a_move_to_a_folder_the_account_lacks_is_skipped() {
    let conn = db();
    let rule = Rule {
        query: "subject:s".into(),
        mark_read: true,
        move_to: "folder".into(),
        folder: "Nowhere".into(),
        ..Default::default()
    };
    set_rules(&conn, vec![rule]).unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert!(queued[0].mark_read);
    assert_eq!(queued[0].target, "");
    assert!(!queued[0].trash);
}

#[test]
fn a_rule_finds_a_folder_by_the_name_people_see() {
    let conn = db();
    let wire = crate::utf7::encode("日本語");
    conn.execute(
        "INSERT INTO folders(account, name) VALUES ('a', ?1)",
        [&wire],
    )
    .unwrap();
    let rule = Rule {
        query: "subject:s".into(),
        move_to: "folder".into(),
        folder: "日本語".into(),
        ..Default::default()
    };
    set_rules(&conn, vec![rule]).unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();
    assert_eq!(
        pending_rule_actions(&conn, "a", "INBOX").unwrap()[0].target,
        wire
    );
}

#[test]
fn queued_actions_retry_a_few_times_then_give_up() {
    let conn = db();
    let rule = Rule {
        query: "subject:s".into(),
        move_to: "trash".into(),
        ..Default::default()
    };
    set_rules(&conn, vec![rule]).unwrap();
    upsert(
        &conn,
        "INBOX",
        &[message(1, "m@x", Some(&[])), message(2, "n@x", Some(&[]))],
    );
    apply_rules(&conn, "a", "INBOX", 1, &[1, 2]).unwrap();

    finish_rule_actions(&conn, "a", "INBOX", &[2], 0).unwrap();
    for _ in 0..4 {
        assert_eq!(
            retry_rule_actions(&conn, "a", "INBOX", &[1], 0, "x").unwrap(),
            0
        );
    }
    assert_eq!(pending_rule_actions(&conn, "a", "INBOX").unwrap().len(), 1);
    assert_eq!(
        retry_rule_actions(&conn, "a", "INBOX", &[1], 0, "x").unwrap(),
        1
    );
    assert!(
        pending_rule_actions(&conn, "a", "INBOX")
            .unwrap()
            .is_empty()
    );
}
