//! Regression tests for rules.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;

#[test]
fn mail_bound_for_different_places_fails_separately() {
    let action = |uid, target: &str| RuleAction {
        uid,
        target: target.into(),
        ..RuleAction::default()
    };
    let batches = rule_action_batches(vec![
        action(1, "Gone"),
        action(2, "Work"),
        action(3, "Gone"),
    ]);
    let uids: Vec<Vec<u32>> = batches
        .iter()
        .map(|batch| batch.iter().map(|action| action.uid).collect())
        .collect();
    assert_eq!(uids, [vec![1, 3], vec![2]]);
}

#[test]
fn moves_come_after_what_other_runs_want_done_to_the_same_mail() {
    let batches = rule_action_batches(vec![
        RuleAction {
            uid: 1,
            target: "Archive".into(),
            run: 1,
            ..RuleAction::default()
        },
        RuleAction {
            uid: 1,
            trash: true,
            mark_read: true,
            star: true,
            run: 2,
            ..RuleAction::default()
        },
        RuleAction {
            uid: 1,
            star: true,
            run: 3,
            ..RuleAction::default()
        },
    ]);
    assert_eq!(batches[0][0].run, 3);
    assert_eq!(batches.len(), 3);

    let order = in_queue_order(batches);
    assert_eq!(
        order.iter().map(|batch| batch[0].run).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[test]
fn a_message_to_copy_takes_a_turn_of_its_own() {
    let plain = |uid| RuleAction {
        uid,
        ..RuleAction::default()
    };
    let copying = |uid| RuleAction {
        uid,
        copies: vec![RuleCopy::default()],
        ..RuleAction::default()
    };
    let uids = |actions: Vec<RuleAction>| {
        one_turn(actions)
            .iter()
            .map(|it| it.uid)
            .collect::<Vec<_>>()
    };
    assert_eq!(uids(vec![copying(1), copying(2)]), [1]);
    assert_eq!(uids(vec![plain(1), plain(2), copying(3)]), [1, 2]);
    assert_eq!(uids((1..40).map(plain).collect()).len(), RULE_RUN_TURN);
    // Other runs' work on the message stays with it.
    assert_eq!(uids(vec![copying(1), plain(1), plain(2)]), [1, 1]);
}

#[test]
fn a_move_waits_for_failed_work_on_the_same_message() {
    let action = |uid, target: &str| RuleAction {
        uid,
        target: target.into(),
        ..RuleAction::default()
    };
    let unfinished = BTreeSet::from([1]);
    let ready = ready_actions(
        vec![action(1, "Archive"), action(2, "Archive")],
        &unfinished,
    );
    assert_eq!(ready.iter().map(|it| it.uid).collect::<Vec<_>>(), [2]);
    assert_eq!(ready_actions(vec![action(1, "")], &unfinished).len(), 1);
}

#[test]
fn mail_copied_to_different_places_fails_separately() {
    let action = |uid, folder: &str| RuleAction {
        uid,
        copies: vec![RuleCopy {
            account: "b".into(),
            folder: folder.into(),
        }],
        ..RuleAction::default()
    };
    let batches = rule_action_batches(vec![action(1, "Gone"), action(2, "Work")]);
    assert_eq!(batches.len(), 2);
}
