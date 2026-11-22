//! Records the outcome of sorting and schedules the related mailbox work.
//! New-mail and existing-inbox jobs use these helpers to update local state.
//! Server flags and subsequent refreshes let the other devices observe the outcome.

use super::*;

pub(super) fn report(engine: &Engine, account: &str, folder: &str) {
    if let Some(report) = engine.rule_progress.get() {
        report(account, folder);
    }
}

/// Refresh the screen once a hold has run out, whatever became of the step that
/// set it.
pub(super) fn refresh_after(engine: &Arc<Engine>, account: &str, folder: &str, after: Duration) {
    let engine = engine.clone();
    let account = account.to_string();
    let folder = folder.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(after).await;
        report(&engine, &account, &folder);
    });
}

/// Record decisions in the store. `cutoff`: the time limit and the flag the
/// timeout path sets. Checked inside the transaction, as dropping this future
/// doesn't stop the blocking task: past it, nothing is recorded and the answer
/// is `None`, so the mail stays in the Inbox.
pub(super) async fn record(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    decisions: Vec<AiSortDecision>,
    run: i64,
    cutoff: Option<(Instant, Arc<AtomicBool>)>,
) -> Option<store::AiSortRecorded> {
    let engine = engine.clone();
    let account = account.to_string();
    let folder = folder.to_string();
    let recorded = tokio::task::spawn_blocking(move || {
        let in_time = || match &cutoff {
            Some((deadline, cancelled)) => {
                Instant::now() < *deadline && !cancelled.load(Ordering::SeqCst)
            }
            None => true,
        };
        store::ai_sort_record_decisions(
            &engine.db.lock().unwrap(),
            &account,
            &folder,
            uidvalidity,
            &decisions,
            run,
            &in_time,
        )
    })
    .await;
    match recorded {
        Ok(Ok(recorded)) => recorded,
        Ok(Err(error)) => {
            warn(format_args!(
                "could not record the AI's decisions: {error:#}"
            ));
            Some(Default::default())
        }
        Err(_) => Some(Default::default()),
    }
}

/// Put `$MeronSorted` on mail the AI decided on, and take this device's claim
/// (`claim`, see [`claim_mail`]) off it. Best effort: a mailbox that refuses
/// new keywords leaves only this device's memory.
pub(super) async fn mark_sorted(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: Vec<u32>,
    claim: Option<&str>,
) {
    if uids.is_empty() {
        return;
    }
    let marked = tokio::time::timeout(
        MARK_WAIT,
        engine.with_write_session(account, |session| {
            let folder = folder.to_string();
            let uids = uids.clone();
            let claim = claim.map(str::to_string);
            Box::pin(async move {
                imap::mark_ai_sorted(session, &folder, &uids, claim.as_deref()).await
            })
        }),
    )
    .await;
    match marked {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) => warn(format_args!(
            "account={account}: the mailbox takes no new keywords; sorted mail is remembered here only"
        )),
        Ok(Err(error)) => warn(format_args!(
            "account={account}: sorted mark failed: {error:#}"
        )),
        Err(_) => warn(format_args!("account={account}: sorted mark timed out")),
    }
}

/// Put the Meron-folder mark on Inbox mail the AI just filed into a Meron
/// folder, so the user's other devices file it the same way (see
/// [`crate::meron_marks`]). Best effort.
pub(super) async fn mark_filed(engine: &Arc<Engine>, account: &str, filed: &[(u32, String)]) {
    let mut by_folder: std::collections::BTreeMap<&str, Vec<(String, u32)>> = Default::default();
    for (uid, id) in filed {
        by_folder
            .entry(id.as_str())
            .or_default()
            .push((store::EXISTING_RULE_FOLDER.to_string(), *uid));
    }
    for (id, rows) in by_folder {
        let uids =
            crate::meron_marks::uids_to_mark(&engine.db.lock().unwrap(), account, &rows, Some(id));
        crate::meron_marks::write_all(
            engine,
            crate::meron_marks::jobs_for(account, uids, Some(id)),
        )
        .await;
    }
}

pub(super) async fn give_up(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    candidates: Vec<AiSortCandidate>,
    state: &'static str,
    detail: String,
) {
    if candidates.is_empty() {
        return;
    }
    let engine = engine.clone();
    let account = account.to_string();
    let folder = folder.to_string();
    let _ = tokio::task::spawn_blocking(move || {
        if let Err(error) = store::ai_sort_give_up(
            &engine.db.lock().unwrap(),
            &account,
            &folder,
            uidvalidity,
            &candidates,
            state,
            &detail,
        ) {
            warn(format_args!("could not show held mail again: {error:#}"));
        }
    })
    .await;
}
