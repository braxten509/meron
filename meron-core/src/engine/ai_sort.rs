//! Coordinates classification of incoming mail and the manual Inbox sorting job.
//!
//! Queued moves use internal rule IDs with the ai-sort prefix so the normal retry
//! and UIDVALIDITY checks apply. Mail delivered to watched folders can be filed
//! locally while remaining in its original server folder.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::ai_sort::{self as ai, AiError, ErrorKind, Progress, PromptEmail, PromptFolder};
use crate::store::{AiSortCandidate, AiSortConfig, AiSortDecision};

/// How long new mail waits for the AI before it shows in the Inbox anyway.
pub const AI_SORT_BUDGET: Duration = Duration::from_secs(20);
/// How long a sync waits for the keys the first time.
const KEY_LOAD_WAIT: Duration = Duration::from_secs(2);
/// How long new mail's bodies may take when the AI reads the start of emails.
const BODY_WAIT: Duration = Duration::from_secs(6);
/// How long writing the `$MeronSorted` mark may take.
const MARK_WAIT: Duration = Duration::from_secs(5);
/// How long claiming mail for this device may take (see [`claim_mail`]).
const CLAIM_WAIT: Duration = Duration::from_secs(8);
/// How often a device that didn't get the claim looks what the other did.
const CLAIM_POLL: Duration = Duration::from_secs(1);
/// How long mail the other device marked decided may take to move away
/// before it counts as left in the Inbox.
const CLAIM_MOVE_WAIT: Duration = Duration::from_secs(3);
/// How long another device's claim stands before this one takes the mail
/// over: that device never finished (closed, or lost its connection).
const CLAIM_STALE: Duration = Duration::from_secs(120);
/// One batch of the "sort Inbox now" job, retry included.
const JOB_BATCH_BUDGET: Duration = Duration::from_secs(40);
/// Bodies for one batch of that job.
const JOB_BODY_WAIT: Duration = Duration::from_secs(15);

fn warn(what: std::fmt::Arguments<'_>) {
    crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "{what}");
}

mod credentials;
pub use credentials::{loaded_keys, store_key};

mod providers;
use providers::{
    decide_batch_by_decision, model_decision, model_structured, proxy_blocks_tinfoil,
    request_with_retry,
};
pub(crate) use providers::{tinfoil_client, warm_tinfoil};

mod classification;
pub use classification::push_lesson_tags;
use classification::{
    decide_batch, expand_decisions, group_conversations, representatives, sender_of, spam_folder,
};

mod outcomes;
use outcomes::{give_up, mark_filed, mark_sorted, record, refresh_after, report};

mod claims;
pub use claims::take_over_stuck;
use claims::{
    OtherDevice, await_other_device, claim_id, claim_mail, confirm_claims, unclaim_mail,
    watch_claims,
};

mod incoming;
pub(crate) use incoming::meron_arrivals;
pub use incoming::{AiSortPlan, MeronArrivals, SortedNewMail, plan_for, sort_new_mail};

mod watched_folders;
pub use watched_folders::check_watched_folders;
pub(crate) use watched_folders::settle_arrivals;

mod existing_inbox;
pub(crate) use existing_inbox::models;
pub use existing_inbox::{estimate, run_existing, stop_existing};

mod restore;
pub use restore::move_back;

#[cfg(test)]
mod tests;
