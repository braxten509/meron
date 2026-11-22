//! Applying rules to new arrivals and to mail already in an account.
//! Fixtures exercise matching, overlapping rules and stopping a manual run.
//! Assertions cover the scope of the action queue and its recorded outcome.

use super::*;

#[test]
fn rules_tag_only_the_new_mail_that_matches() {
    let conn = db();
    let rule = Rule {
        query: "from:billing".into(),
        tags: vec!["Finance".into()],
        ..Default::default()
    };
    assert_eq!(set_rules(&conn, vec![rule]).unwrap()[0].tags, ["finance"]);

    let mut new_bill = message(2, "new@x", Some(&[]));
    new_bill.from_addr = "billing@shop.test".into();
    let mut old_bill = message(1, "old@x", Some(&[]));
    old_bill.from_addr = "billing@shop.test".into();
    let other = message(3, "other@x", Some(&[]));
    upsert(&conn, "INBOX", &[old_bill, new_bill, other]);

    apply_rules(&conn, "a", "INBOX", 1, &[2, 3]).unwrap();
    assert_eq!(tags(&conn, "new@x"), ["finance"]);
    assert!(tags(&conn, "old@x").is_empty(), "already here before");
    assert!(tags(&conn, "other@x").is_empty());
}

#[test]
fn mail_filed_from_another_folder_has_not_arrived() {
    let conn = db();
    let known = message(1, "m@x", Some(&[]));
    upsert(&conn, "INBOX", std::slice::from_ref(&known));
    let filed = message(7, "m@x", Some(&[]));
    let fresh = message(8, "new@x", Some(&[]));
    let first_seen =
        crate::store::first_seen_messages(&conn, "a", "Archive", 7, 9, &[filed, fresh]).unwrap();
    let uids: Vec<u32> = first_seen.iter().map(|message| message.uid).collect();
    assert_eq!(uids, [8]);
}

#[test]
fn rules_leave_sent_mail_alone_and_refuse_to_be_empty() {
    let conn = db();
    let rule = Rule {
        query: "subject:s".into(),
        tags: vec!["todo".into()],
        ..Default::default()
    };
    set_rules(&conn, vec![rule.clone()]).unwrap();
    upsert(&conn, "Sent", &[message(1, "m@x", Some(&[]))]);
    apply_rules(&conn, "a", "Sent", 1, &[1]).unwrap();
    assert!(tags(&conn, "m@x").is_empty());

    // No search is all new mail; no action is nothing at all.
    let no_search = Rule {
        query: " ".into(),
        ..rule.clone()
    };
    assert_eq!(set_rules(&conn, vec![no_search]).unwrap()[0].query, "");
    let no_tag = Rule {
        tags: vec![],
        ..rule
    };
    assert!(set_rules(&conn, vec![no_tag]).is_err());
}

#[test]
fn renaming_a_tag_moves_it_on_the_server_and_merges_into_an_existing_one() {
    let conn = db();
    upsert(
        &conn,
        "INBOX",
        &[
            message(1, "one@x", Some(&["old"])),
            message(2, "both@x", Some(&["new", "old"])),
            message(3, "other@x", Some(&["keep"])),
        ],
    );
    assert_eq!(replace_tag(&conn, "old", Some("new"), true).unwrap(), ["a"]);
    assert_eq!(tags(&conn, "one@x"), ["new"]);
    assert_eq!(tags(&conn, "both@x"), ["new"]);
    assert_eq!(tags(&conn, "other@x"), ["keep"]);
    assert_eq!(tag_counts(&conn, None).unwrap().get("old"), None);

    let pushes = pending_tag_pushes(&conn, "a").unwrap();
    assert_eq!(pushes.len(), 2);
    assert!(
        pushes
            .iter()
            .any(|push| push.tag == "new" && push.add && push.uids == [1])
    );
    assert!(
        pushes
            .iter()
            .any(|push| push.tag == "old" && !push.add && push.uids == [1, 2])
    );
}

#[test]
fn matching_rules_add_up_and_the_first_move_wins() {
    let conn = db();
    let rule = |query: &str| Rule {
        query: query.into(),
        ..Default::default()
    };
    set_rules(
        &conn,
        vec![
            Rule {
                mark_read: true,
                ..rule("subject:s")
            },
            Rule {
                move_to: "trash".into(),
                ..rule("subject:s")
            },
            Rule {
                star: true,
                move_to: "archive".into(),
                ..rule("subject:s")
            },
        ],
    )
    .unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);

    apply_rules(&conn, "a", "INBOX", 9, &[1]).unwrap();
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert_eq!(queued.len(), 1);
    let action = &queued[0];
    assert_eq!((action.uid, action.uidvalidity), (1, 9));
    assert!(action.mark_read && action.star && action.trash);
    assert_eq!(action.target, "");
}

#[test]
fn a_rule_without_a_search_takes_all_new_mail() {
    let conn = db();
    set_rules(
        &conn,
        vec![Rule {
            star: true,
            ..Default::default()
        }],
    )
    .unwrap();
    upsert(
        &conn,
        "INBOX",
        &[message(1, "m@x", Some(&[])), message(2, "n@x", Some(&[]))],
    );

    apply_rules(&conn, "a", "INBOX", 1, &[1, 2]).unwrap();
    assert_eq!(pending_rule_actions(&conn, "a", "INBOX").unwrap().len(), 2);
}

#[test]
fn running_a_rule_by_hand_takes_inbox_mail_and_is_safe_to_repeat() {
    let conn = db();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, email, display_name, avatar_url, config, created_at, updated_at)
         VALUES('a', 'imap', 'imap', 'a@x.test', '', '', '{}', 0, 0)",
        [],
    )
    .unwrap();
    crate::store::set_folder_state(&conn, "a", "INBOX", 7, 10).unwrap();
    let rules = set_rules(
        &conn,
        vec![Rule {
            id: "r".into(),
            query: "subject:s".into(),
            star: true,
            copy_account: "a".into(),
            copy_folder: "Saved".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    let id = &rules[0].id;
    upsert(
        &conn,
        "INBOX",
        &[message(1, "m@x", Some(&[])), message(2, "n@x", Some(&[]))],
    );
    upsert(&conn, "Saved", &[message(9, "n@x", Some(&[]))]);
    upsert(&conn, "Other", &[message(5, "o@x", Some(&[]))]);

    assert_eq!(count_existing_rule_matches(&conn, id).unwrap(), 2);
    assert_eq!(
        queue_existing_rule(&conn, id).unwrap(),
        [("a".to_string(), 2)]
    );
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert_eq!(queued.len(), 2);
    assert_eq!(queued[0].uidvalidity, 7);
    assert_eq!(queued[0].copies.len(), 1);
    assert!(
        pending_rule_actions(&conn, "a", "Other")
            .unwrap()
            .is_empty()
    );
    assert!(queue_existing_rule(&conn, id).unwrap().is_empty());
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert_eq!(queued[0].copies.len(), 1);
}

#[test]
fn a_rule_turned_off_does_nothing() {
    let conn = db();
    account(&conn, "a");
    crate::store::set_folder_state(&conn, "a", "INBOX", 7, 10).unwrap();
    let rules = set_rules(
        &conn,
        vec![Rule {
            star: true,
            disabled: true,
            ..Default::default()
        }],
    )
    .unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);

    apply_rules(&conn, "a", "INBOX", 7, &[1]).unwrap();
    assert!(
        pending_rule_actions(&conn, "a", "INBOX")
            .unwrap()
            .is_empty()
    );
    assert!(queue_existing_rule(&conn, &rules[0].id).is_err());
}

#[test]
fn mail_without_a_message_id_is_not_copied_or_moved_across_accounts() {
    let conn = db();
    set_rules(
        &conn,
        vec![Rule {
            star: true,
            move_to: "folder".into(),
            folder: "Saved".into(),
            folder_account: "b".into(),
            copy_account: "b".into(),
            copy_folder: "Other".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    upsert(&conn, "INBOX", &[message(1, "", Some(&[]))]);

    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert!(queued[0].star);
    assert!(queued[0].copies.is_empty() && !queued[0].trash);
    let log = crate::store::rule_log(&conn).unwrap();
    assert!(log.iter().any(|entry| entry.failed));
}

#[test]
fn removing_an_account_narrows_a_rule_or_turns_it_off() {
    let conn = db();
    let rule = |accounts: &[&str], copy: &str| Rule {
        star: true,
        accounts: accounts.iter().map(|id| id.to_string()).collect(),
        copy_account: copy.into(),
        copy_folder: if copy.is_empty() { "" } else { "Saved" }.into(),
        ..Default::default()
    };
    set_rules(
        &conn,
        vec![
            rule(&["gone", "kept"], ""),
            rule(&["gone"], ""),
            rule(&[], "gone"),
        ],
    )
    .unwrap();

    crate::store::forget_rule_account(&conn, "gone").unwrap();
    let rules = crate::store::rules(&conn).unwrap();
    assert_eq!(rules[0].accounts, ["kept"]);
    assert!(!rules[0].disabled);
    assert_eq!(rules[1].accounts, ["gone"]);
    assert!(rules[1].disabled && rules[2].disabled);
}

#[test]
fn what_rules_did_is_logged_and_a_given_up_action_counts_against_its_rule() {
    let conn = db();
    let rules = set_rules(
        &conn,
        vec![Rule {
            star: true,
            ..Default::default()
        }],
    )
    .unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();

    for _ in 0..5 {
        retry_rule_actions(&conn, "a", "INBOX", &[1], 0, "no such folder").unwrap();
    }
    let log = crate::store::rule_log(&conn).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].rule_id, rules[0].id);
    assert!(log[0].failed);
    assert_eq!(log[0].detail, "no such folder");
    assert_eq!(log[0].subject, "s");
    assert_eq!(log[1].subject, "s");
    assert!(!log[1].failed);
}

#[test]
fn a_run_by_hand_reports_what_is_left_and_can_be_stopped() {
    let conn = db();
    account(&conn, "a");
    crate::store::set_folder_state(&conn, "a", "INBOX", 7, 10).unwrap();
    let rules = set_rules(
        &conn,
        vec![Rule {
            query: "subject:s".into(),
            star: true,
            ..Default::default()
        }],
    )
    .unwrap();
    let id = rules[0].id.clone();
    upsert(
        &conn,
        "INBOX",
        &[message(1, "m@x", Some(&[])), message(2, "n@x", Some(&[]))],
    );

    queue_existing_rule(&conn, &id).unwrap();
    assert_eq!(crate::store::running_rules(&conn).unwrap()[&id], 2);
    crate::store::stop_rule(&conn, &id).unwrap();
    assert!(crate::store::running_rules(&conn).unwrap().is_empty());
    assert!(
        pending_rule_actions(&conn, "a", "INBOX")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_run_by_hand_doesnt_skip_mail_another_rule_has_work_queued_for() {
    let conn = db();
    account(&conn, "a");
    crate::store::set_folder_state(&conn, "a", "INBOX", 7, 10).unwrap();
    let rule = |star: bool| Rule {
        query: "subject:s".into(),
        star,
        mark_read: !star,
        ..Default::default()
    };
    let rules = set_rules(&conn, vec![rule(true), rule(false)]).unwrap();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);

    queue_existing_rule(&conn, &rules[0].id).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    assert_eq!(
        queue_existing_rule(&conn, &rules[1].id).unwrap(),
        [("a".to_string(), 1)]
    );
    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert_eq!(queued.len(), 2);
    assert!(queued.iter().any(|action| action.star));
    assert!(queued.iter().any(|action| action.mark_read));

    // The same rule again adds nothing.
    assert!(queue_existing_rule(&conn, &rules[1].id).unwrap().is_empty());
}

#[test]
fn what_a_stopped_run_had_in_flight_leaves_a_later_run_alone() {
    let conn = db();
    account(&conn, "a");
    crate::store::set_folder_state(&conn, "a", "INBOX", 7, 10).unwrap();
    let rules = set_rules(
        &conn,
        vec![Rule {
            query: "subject:s".into(),
            copy_account: "a".into(),
            copy_folder: "Saved".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    let id = rules[0].id.clone();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);

    queue_existing_rule(&conn, &id).unwrap();
    let old = pending_rule_actions(&conn, "a", "INBOX").unwrap().remove(0);
    crate::store::stop_rule(&conn, &id).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(2));
    queue_existing_rule(&conn, &id).unwrap();
    for _ in 0..5 {
        retry_rule_actions(&conn, "a", "INBOX", &[1], old.run, "x").unwrap();
    }
    finish_rule_actions(&conn, "a", "INBOX", &[1], old.run).unwrap();

    let queued = pending_rule_actions(&conn, "a", "INBOX").unwrap();
    assert_eq!(queued.len(), 1);
    assert_ne!(queued[0].run, old.run);
    assert_eq!(queued[0].copies, old.copies);
}

#[test]
fn a_rule_runs_only_on_its_accounts() {
    let conn = db();
    let rule = |accounts: &[&str]| Rule {
        query: "subject:s".into(),
        star: true,
        accounts: accounts.iter().map(|id| id.to_string()).collect(),
        ..Default::default()
    };
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);

    set_rules(&conn, vec![rule(&["other"])]).unwrap();
    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();
    assert!(
        pending_rule_actions(&conn, "a", "INBOX")
            .unwrap()
            .is_empty()
    );

    set_rules(&conn, vec![rule(&["a", "other"])]).unwrap();
    apply_rules(&conn, "a", "INBOX", 1, &[1]).unwrap();
    assert_eq!(pending_rule_actions(&conn, "a", "INBOX").unwrap().len(), 1);
}
