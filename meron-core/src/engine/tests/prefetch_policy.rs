//! Checks how the offline reader spends its background download budget.
//! These cases cover failed fetch attribution and choosing the newest UIDs first.
//! No account, network connection or cached message bodies are needed.

use crate::engine::prefetch::prefetch_failure_is_the_messages;
use crate::engine::{TransferTimedOut, limit_prefetch_uids};
use std::time::Duration;

#[test]
fn prefetch_blames_a_message_only_for_failures_of_its_own() {
    // Went silent or outlasted its allowance, or refused by the server:
    // potentially the message's.
    assert!(prefetch_failure_is_the_messages(
        &TransferTimedOut(Duration::from_secs(120)).into()
    ));
    assert!(prefetch_failure_is_the_messages(&anyhow::anyhow!(
        "UID FETCH: no such message"
    )));
    // The connection went away: says nothing about the message.
    assert!(!prefetch_failure_is_the_messages(&anyhow::Error::new(
        async_imap::error::Error::ConnectionLost
    )));
}

#[test]
fn prefetch_limit_keeps_all_uids_when_uncapped() {
    // Uncapped runs still warm newest mail first; nothing is dropped.
    assert_eq!(
        limit_prefetch_uids(vec![1, 2, 3, 4], None),
        vec![4, 3, 2, 1]
    );
}

#[test]
fn prefetch_limit_spends_mobile_budget_on_newest_uids() {
    assert_eq!(limit_prefetch_uids(vec![1, 2, 3, 4], Some(2)), vec![4, 3]);
}

#[test]
fn prefetch_limit_zero_fetches_nothing() {
    assert_eq!(
        limit_prefetch_uids(vec![1, 2, 3, 4], Some(0)),
        Vec::<u32>::new()
    );
}
