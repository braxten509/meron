//! Coordinates ownership of sorting across devices sharing an IMAP account.
//! A device claims messages and checks what its peers have already handled.
//! Stale claims can be taken over so a lost connection does not strand mail.

use super::*;

// ---- Claims: one device sorts each email --------------------------------------
//
// Two of the reader's devices see the same new email at the same moment.
// Before a device asks the AI, it claims the email on the server (see
// `imap::claim_for_ai_sort`); only the device that got the claim asks and
// moves. The other waits to see what became of the email, so it neither
// shows nor announces mail that is about to be moved away, and takes the
// email over when the claim just stands there (that device never finished).

/// [`CLAIM_STALE`]; `MERON_AI_CLAIM_STALE_SECS` shortens it for checks.
fn claim_stale() -> Duration {
    std::env::var("MERON_AI_CLAIM_STALE_SECS")
        .ok()
        .and_then(|secs| secs.trim().parse().ok())
        .map(Duration::from_secs)
        .unwrap_or(CLAIM_STALE)
}

/// This device's claim id; `None` (no claiming) when the store can't say.
pub(super) fn claim_id(engine: &Engine) -> Option<String> {
    match store::ai_sort_claim_id(&engine.db.lock().unwrap()) {
        Ok(id) => Some(id),
        Err(error) => {
            warn(format_args!(
                "no claim id, sorting without claims: {error:#}"
            ));
            None
        }
    }
}

/// The mail this device is to sort, after [`claim_mail`].
pub(super) struct Claimed {
    pub(super) mine: BTreeSet<u32>,
    /// No other device can think the same (see `imap::AiClaims::exact`).
    /// Otherwise the claim is asked for once more before the mail is acted
    /// on ([`confirm_claims`]).
    pub(super) exact: bool,
}

/// Claim messages in this folder and return the UIDs this device should sort.
///
/// Exclude messages already claimed or decided by a peer. When the mailbox lacks
/// keywords or the claim request times out, return the candidates for ordinary sorting.
pub(super) async fn claim_mail(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: &[u32],
    id: &str,
    wait: Duration,
) -> Claimed {
    let all = || Claimed {
        mine: uids.iter().copied().collect(),
        exact: true,
    };
    if uids.is_empty() {
        return all();
    }
    let claimed = tokio::time::timeout(
        wait,
        engine.with_write_session(account, |session| {
            let folder = folder.to_string();
            let uids = uids.to_vec();
            let id = id.to_string();
            Box::pin(async move { imap::claim_for_ai_sort(session, &folder, &uids, &id).await })
        }),
    )
    .await;
    match claimed {
        Ok(Ok(Some(claims))) => Claimed {
            mine: claims
                .states
                .iter()
                .filter(|state| state.won_by(id))
                .map(|state| state.uid)
                .collect(),
            exact: claims.exact,
        },
        Ok(Ok(None)) => all(),
        Ok(Err(error)) => {
            warn(format_args!(
                "account={account}: claiming mail failed: {error:#}"
            ));
            all()
        }
        Err(_) => {
            warn(format_args!("account={account}: claiming mail timed out"));
            all()
        }
    }
}

/// Keep decisions for messages that this device still owns.
///
/// With inexact claims, a peer with a smaller device ID can take precedence.
/// If the server cannot be queried, keep the decisions already made.
pub(super) async fn confirm_claims(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    decisions: Vec<AiSortDecision>,
    id: &str,
) -> (Vec<AiSortDecision>, Vec<u32>) {
    let uids: Vec<u32> = decisions
        .iter()
        .map(|decision| decision.candidate.uid)
        .collect();
    let read = tokio::time::timeout(
        MARK_WAIT,
        engine.with_read_session(account, |session| {
            let folder = folder.to_string();
            let uids = uids.clone();
            Box::pin(async move { imap::ai_claim_states(session, &folder, &uids).await })
        }),
    )
    .await;
    let Ok(Ok(states)) = read else {
        warn(format_args!(
            "account={account}: claims not confirmed, sorting on"
        ));
        return (decisions, Vec::new());
    };
    let held: BTreeSet<u32> = states
        .iter()
        .filter(|state| state.won_by(id))
        .map(|state| state.uid)
        .collect();
    let (kept, lost): (Vec<_>, Vec<_>) = decisions
        .into_iter()
        .partition(|decision| held.contains(&decision.candidate.uid));
    (
        kept,
        lost.into_iter()
            .map(|decision| decision.candidate.uid)
            .collect(),
    )
}

/// Take the claims of the devices `ids` off `uids`. Best effort.
pub(super) async fn unclaim_mail(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: Vec<u32>,
    ids: Vec<String>,
) {
    if uids.is_empty() || ids.is_empty() {
        return;
    }
    let done = tokio::time::timeout(
        MARK_WAIT,
        engine.with_write_session(account, |session| {
            let folder = folder.to_string();
            let uids = uids.clone();
            let ids = ids.clone();
            Box::pin(async move { imap::unclaim_for_ai_sort(session, &folder, &uids, &ids).await })
        }),
    )
    .await;
    match done {
        Ok(Ok(())) => {}
        Ok(Err(error)) => warn(format_args!(
            "account={account}: taking a claim off failed: {error:#}"
        )),
        Err(_) => warn(format_args!(
            "account={account}: taking a claim off timed out"
        )),
    }
}

/// What became of mail another device has the claim on.
#[derive(Default)]
pub(super) struct OtherDevice {
    /// Moved away: no longer in the folder.
    pub(super) gone: Vec<u32>,
    /// Neither decided nor gone by the time limit, or given up on by that
    /// device: this one takes it over once the claim is stale. With the
    /// claims last seen on it.
    pub(super) open: Vec<(u32, Vec<String>)>,
}

/// Watch `uids`, which another device sorts, until each is moved away,
/// decided and left in the folder, or given up on; at most until `deadline`.
pub(super) async fn await_other_device(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: &[u32],
    deadline: Instant,
) -> OtherDevice {
    let mut waiting: BTreeSet<u32> = uids.iter().copied().collect();
    let mut decided: HashMap<u32, Instant> = HashMap::new();
    let mut claims: HashMap<u32, Vec<String>> = HashMap::new();
    let mut out = OtherDevice::default();
    while !waiting.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        let read = tokio::time::timeout(
            left,
            engine.with_read_session(account, |session| {
                let folder = folder.to_string();
                let uids: Vec<u32> = waiting.iter().copied().collect();
                Box::pin(async move { imap::ai_claim_states(session, &folder, &uids).await })
            }),
        )
        .await;
        let states = match read {
            Ok(Ok(states)) => states,
            Ok(Err(error)) => {
                warn(format_args!(
                    "account={account}: reading claims failed: {error:#}"
                ));
                break;
            }
            Err(_) => break,
        };
        for uid in waiting.clone() {
            match states.iter().find(|state| state.uid == uid) {
                None => {
                    out.gone.push(uid);
                    waiting.remove(&uid);
                }
                // The mark comes just before the move: gone soon, or left.
                Some(state) if state.sorted => {
                    let since = decided.entry(uid).or_insert_with(Instant::now);
                    if since.elapsed() >= CLAIM_MOVE_WAIT {
                        waiting.remove(&uid);
                    }
                }
                Some(state) if state.claims.is_empty() => {
                    out.open.push((uid, Vec::new()));
                    waiting.remove(&uid);
                }
                Some(state) => {
                    claims.insert(uid, state.claims.clone());
                }
            }
        }
        if waiting.is_empty() || Instant::now() + CLAIM_POLL >= deadline {
            break;
        }
        tokio::time::sleep(CLAIM_POLL).await;
    }
    out.open.extend(
        waiting
            .into_iter()
            .filter(|uid| !decided.contains_key(uid))
            .map(|uid| (uid, claims.remove(&uid).unwrap_or_default())),
    );
    out
}

/// Inbox mail another device claimed and didn't finish, per account: what
/// this device takes over once the claim is [`claim_stale`].
pub(super) struct Watched {
    pub(super) candidate: AiSortCandidate,
    pub(super) uidvalidity: u32,
    /// The claims seen on it at `since`: the ones that are stale later. A
    /// claim that came after is another device taking the mail over.
    pub(super) claims: Vec<String>,
    pub(super) since: Instant,
}

static CLAIM_WATCH: std::sync::Mutex<Option<HashMap<String, Vec<Watched>>>> =
    std::sync::Mutex::new(None);

/// Remember `candidates` (seen at `since` with these claims of other
/// devices) and look at them again once the claims are stale.
pub(super) fn watch_claims(
    engine: &Arc<Engine>,
    account: &str,
    uidvalidity: u32,
    candidates: Vec<(AiSortCandidate, Vec<String>)>,
    since: Instant,
) {
    if candidates.is_empty() {
        return;
    }
    CLAIM_WATCH
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .entry(account.to_string())
        .or_default()
        .extend(candidates.into_iter().map(|(candidate, claims)| Watched {
            candidate,
            uidvalidity,
            claims,
            since,
        }));
    let engine = engine.clone();
    let account = account.to_string();
    let after = claim_stale().saturating_sub(since.elapsed()) + Duration::from_secs(1);
    tokio::spawn(async move {
        tokio::time::sleep(after).await;
        take_over_stuck(&engine, &account).await;
    });
}

/// Take over Inbox messages that still have the same stale, undecided claim.
///
/// Remove that claim and use the normal new-mail path to claim each message again.
/// Watch newer claims from their observed time. A stale timer and the periodic
/// watched-folder check both trigger this recovery.
pub async fn take_over_stuck(engine: &Arc<Engine>, account: &str) {
    let folder = store::EXISTING_RULE_FOLDER;
    let stale = claim_stale();
    let due: Vec<Watched> = {
        let mut watch = CLAIM_WATCH.lock().unwrap();
        let Some(watched) = watch.as_mut().and_then(|watch| watch.get_mut(account)) else {
            return;
        };
        let (due, rest) = std::mem::take(watched)
            .into_iter()
            .partition(|watched| watched.since.elapsed() >= stale);
        *watched = rest;
        due
    };
    if due.is_empty() || engine.is_paused(account) {
        return;
    }
    // Sorting was turned off meanwhile: nothing to take over.
    let Some(plan) = plan_for(engine, account, folder).await else {
        return;
    };
    let Some(id) = claim_id(engine) else {
        return;
    };
    let uids: Vec<u32> = due.iter().map(|watched| watched.candidate.uid).collect();
    let read = tokio::time::timeout(
        MARK_WAIT,
        engine.with_read_session(account, |session| {
            let uids = uids.clone();
            Box::pin(async move { imap::ai_claim_states(session, folder, &uids).await })
        }),
    )
    .await;
    let states = match read {
        Ok(Ok(states)) => states,
        // Asked again with the next check.
        _ => {
            CLAIM_WATCH
                .lock()
                .unwrap()
                .get_or_insert_with(HashMap::new)
                .entry(account.to_string())
                .or_default()
                .extend(due);
            return;
        }
    };
    let validity = store::get_folder_state(&engine.db.lock().unwrap(), account, folder)
        .ok()
        .flatten()
        .map(|(validity, _)| validity);
    let mut others: BTreeSet<String> = BTreeSet::new();
    let mut take: Vec<AiSortCandidate> = Vec::new();
    let mut again: Vec<(AiSortCandidate, Vec<String>)> = Vec::new();
    for watched in due {
        let state = states
            .iter()
            .find(|state| state.uid == watched.candidate.uid);
        // Moved away, decided, or the mailbox started over: no longer open.
        let Some(state) = state.filter(|state| !state.sorted) else {
            continue;
        };
        if validity != Some(watched.uidvalidity) {
            continue;
        }
        let theirs: Vec<String> = state
            .claims
            .iter()
            .filter(|claim| !claim.eq_ignore_ascii_case(&id))
            .cloned()
            .collect();
        if theirs.iter().any(|claim| !watched.claims.contains(claim)) {
            again.push((watched.candidate, theirs));
            continue;
        }
        others.extend(theirs);
        take.push(watched.candidate);
    }
    if let Some(validity) = validity {
        watch_claims(engine, account, validity, again, Instant::now());
    }
    // What this device decided on or filed itself meanwhile stays as it is.
    let take = store::ai_sort_still_open(&engine.db.lock().unwrap(), account, folder, &take)
        .unwrap_or_default();
    let Some(validity) = validity.filter(|_| !take.is_empty()) else {
        return;
    };
    let uids: Vec<u32> = take.iter().map(|candidate| candidate.uid).collect();
    unclaim_mail(engine, account, folder, uids, others.into_iter().collect()).await;
    if let Err(error) = store::ai_sort_count_taken_over(&engine.db.lock().unwrap(), take.len()) {
        warn(format_args!("taken-over mail not counted: {error:#}"));
    }
    let sorted = sort_new_mail(engine, account, folder, validity, plan, take).await;
    if !sorted.silenced.is_empty() {
        spawn_rule_worker(engine, account, folder);
    }
}
