//! Plans and runs sorting for newly arrived mail.
//! The sync path supplies the folder and candidate headers for this pass.
//! The result identifies which arrivals to silence or announce under a Meron folder.

use super::*;

/// What a sync of INBOX (or of a folder sorting watches, see
/// `store::ai_sort_watches_folder`) needs to sort its new mail: the setting,
/// the key and the account's folders. `None` when sorting is off or not set
/// up for it.
#[derive(Clone)]
pub struct AiSortPlan {
    pub config: AiSortConfig,
    key: String,
    folders: Vec<PromptFolder>,
}

pub async fn plan_for(engine: &Arc<Engine>, account: &str, folder: &str) -> Option<AiSortPlan> {
    if store::is_local_account(account) {
        return None;
    }
    let inbox = folder.eq_ignore_ascii_case("INBOX");
    let (config, folders) = {
        let db = engine.db.lock().unwrap();
        let config = store::ai_sort_config(&db).ok()?;
        if !config.enabled || config.model.is_empty() {
            return None;
        }
        // Besides the Inbox, the plain folders server filters deliver into
        // (see `store::ai_sort_watches_folder`): their new mail is sorted the
        // same way, shown in the Inbox when the AI leaves it.
        if !inbox && !store::ai_sort_watches_folder(&db, &config, account, folder).ok()? {
            return None;
        }
        // Meron folders take every mail account's Inbox, but never a feed's.
        let meron = store::meron_folder_names(&db).ok()?;
        let folders = ai::prompt_folders(&config, account, &meron);
        let sortable = store::sort_accounts(&db)
            .ok()?
            .iter()
            .any(|id| id == account);
        if !sortable {
            return None;
        }
        (config, folders)
    };
    if folders.is_empty() {
        return None;
    }
    let keys = tokio::time::timeout(KEY_LOAD_WAIT, loaded_keys(engine))
        .await
        .ok()?;
    let key = keys.get(&config.provider)?.clone();
    if config.provider == "tinfoil" {
        if let Some(error) = proxy_blocks_tinfoil() {
            engine.ai_sort.set_error(&error);
            return None;
        }
        warm_tinfoil(engine, &key);
    }
    Some(AiSortPlan {
        config,
        key,
        folders,
    })
}

/// What [`sort_new_mail`] did with a sync's new mail.
#[derive(Clone, Debug, Default)]
pub struct SortedNewMail {
    /// UIDs queued to move or filed into a Meron folder, which the caller
    /// doesn't alert for as the folder's own arrivals.
    pub silenced: Vec<u32>,
    /// UIDs filed into a Meron folder, each with that folder's id.
    pub filed: Vec<(u32, String)>,
}

/// New mail AI sorting filed into a Meron folder that has notifications on:
/// announced like an opted-in folder's arrivals, under the Meron folder's name.
#[derive(Clone, Default)]
pub struct MeronArrivals {
    /// The mailbox the mail is in (the Inbox, or a folder a server filter
    /// delivered it to).
    pub folder: String,
    pub meron_id: String,
    pub meron_name: String,
    /// Newest first, as a sync's arrivals are.
    pub headers: Vec<imap::MessageHeader>,
}

/// Of `arrivals` (a sync's genuinely new, unread mail of `folder`), what AI
/// sorting `filed` into a Meron folder with notifications on, per folder.
/// Mail it filed that wasn't an arrival (already read, seen before) stays
/// silent, as does everything filed into a folder with notifications off.
pub(crate) fn meron_arrivals(
    engine: &Engine,
    folder: &str,
    arrivals: &[imap::MessageHeader],
    filed: &[(u32, String)],
) -> Vec<MeronArrivals> {
    if arrivals.is_empty() || filed.is_empty() {
        return Vec::new();
    }
    let into: HashMap<u32, &str> = filed.iter().map(|(uid, id)| (*uid, id.as_str())).collect();
    let db = engine.db.lock().unwrap();
    let mut out: Vec<MeronArrivals> = Vec::new();
    for header in arrivals {
        let Some(id) = into.get(&header.uid) else {
            continue;
        };
        if let Some(group) = out.iter_mut().find(|group| group.meron_id == *id) {
            group.headers.push(header.clone());
            continue;
        }
        if !store::meron_folder_notifies(&db, id).unwrap_or(false) {
            continue;
        }
        let Some(name) = store::meron_folder_name(&db, id).ok().flatten() else {
            continue;
        };
        out.push(MeronArrivals {
            folder: folder.to_string(),
            meron_id: id.to_string(),
            meron_name: name,
            headers: vec![header.clone()],
        });
    }
    out
}

/// Sort the arrivals held by a folder sync within [`AI_SORT_BUDGET`].
///
/// Return UIDs queued to move or filed locally so the caller can suppress duplicate
/// notifications. Uncertain decisions and requests past the deadline leave the
/// message in Inbox. Discard late responses.
pub async fn sort_new_mail(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    plan: AiSortPlan,
    candidates: Vec<AiSortCandidate>,
) -> SortedNewMail {
    if candidates.is_empty() {
        return SortedNewMail::default();
    }
    refresh_after(
        engine,
        account,
        folder,
        Duration::from_secs(store::AI_SORT_HOLD_SECS as u64 + 1),
    );
    let started = Instant::now();
    let deadline = started + AI_SORT_BUDGET;
    let inbox = folder.eq_ignore_ascii_case("INBOX");
    // Only one of the reader's devices sorts each email: the one that gets
    // the claim (see "Claims" above). Only in the Inbox: sorting elsewhere is
    // this device's view and writes nothing to the server.
    let claim = if inbox { claim_id(engine) } else { None };
    let mut exact = true;
    let (candidates, others) = match &claim {
        Some(id) => {
            let uids: Vec<u32> = candidates.iter().map(|candidate| candidate.uid).collect();
            let claimed = claim_mail(engine, account, folder, &uids, id, CLAIM_WAIT).await;
            exact = claimed.exact;
            candidates
                .into_iter()
                .partition(|candidate| claimed.mine.contains(&candidate.uid))
        }
        None => (candidates, Vec::new()),
    };
    // Mail whose claim went to another device after all (see `confirm_claims`).
    let yielded: std::sync::Mutex<Vec<u32>> = Default::default();
    let other_uids: Vec<u32> = others.iter().map(|candidate| candidate.uid).collect();
    // Set by the timeout path, so a recording still waiting for the store gives up.
    let cancelled = Arc::new(AtomicBool::new(false));
    let settled: std::sync::Mutex<BTreeSet<u32>> = Default::default();
    let moving: std::sync::Mutex<Vec<u32>> = Default::default();
    let filed: std::sync::Mutex<Vec<(u32, String)>> = Default::default();
    // What got the `$MeronSorted` mark: its claim went with that.
    let marked: std::sync::Mutex<BTreeSet<u32>> = Default::default();
    // One answer per conversation; a conversation with rule work queued is
    // shown again and left to the rules.
    let groups = group_conversations(engine, account, folder, candidates.clone());
    let grouped: BTreeSet<u32> = groups.iter().flatten().map(|it| it.uid).collect();
    settled.lock().unwrap().extend(
        candidates
            .iter()
            .map(|candidate| candidate.uid)
            .filter(|uid| !grouped.contains(uid)),
    );
    // `Err(None)`: out of time; `Err(Some(_))`: the AI failed.
    let work = async {
        let lock = engine.ai_sort.sort_lock(account, folder);
        let _turn = lock.lock().await;
        if plan.config.reads == "start" {
            let uids: Vec<u32> = candidates.iter().map(|candidate| candidate.uid).collect();
            let fetch =
                fetch_bodies_for_uids(engine, account, folder, &uids, engine.host.media_root());
            let wait = BODY_WAIT.min(deadline.saturating_duration_since(Instant::now()));
            // Mail whose body isn't here by then goes with sender and subject.
            let _ = tokio::time::timeout(wait, fetch).await;
        }
        for chunk in groups.chunks(ai::BATCH) {
            let batch = representatives(chunk);
            let decisions = decide_batch(
                engine,
                &plan.config,
                &plan.key,
                &plan.folders,
                account,
                folder,
                &batch,
                deadline,
            )
            .await
            .map_err(Some)?;
            let decisions =
                expand_decisions(chunk, decisions, spam_folder(engine, account).as_deref());
            let decisions = match &claim {
                Some(id) if !exact => {
                    let (kept, lost) = confirm_claims(engine, account, folder, decisions, id).await;
                    // At once, before the device that sorts it moves it away.
                    unclaim_mail(engine, account, folder, lost.clone(), vec![id.clone()]).await;
                    settled.lock().unwrap().extend(lost.iter().copied());
                    yielded.lock().unwrap().extend(lost);
                    kept
                }
                _ => decisions,
            };
            let decided: Vec<u32> = decisions.iter().map(|it| it.candidate.uid).collect();
            let cutoff = Some((deadline, cancelled.clone()));
            let Some(recorded) =
                record(engine, account, folder, uidvalidity, decisions, 0, cutoff).await
            else {
                return Err(None);
            };
            settled.lock().unwrap().extend(decided);
            // The Inbox copy (Gmail) of label mail sent to a server folder:
            // moved by the Inbox's own queue, as new Inbox mail would be.
            if !recorded.inbox_moving.is_empty() {
                spawn_rule_worker(engine, account, store::EXISTING_RULE_FOLDER);
            }
            moving.lock().unwrap().extend(recorded.moving);
            filed
                .lock()
                .unwrap()
                .extend(recorded.filed_to.iter().cloned());
            // After the decision, before the move, so the mark travels with it;
            // only on what the store took on. Not outside the Inbox: sorting
            // there is this device's view only and writes nothing to the server.
            if inbox {
                marked
                    .lock()
                    .unwrap()
                    .extend(recorded.accepted.iter().copied());
                mark_sorted(engine, account, folder, recorded.accepted, claim.as_deref()).await;
                mark_filed(engine, account, &recorded.filed_to).await;
            }
            engine.ai_sort.clear_error();
        }
        Ok::<(), Option<AiError>>(())
    };
    // Meanwhile, what the other device does with the mail it claimed.
    let waiting = async {
        if other_uids.is_empty() {
            return OtherDevice::default();
        }
        await_other_device(engine, account, folder, &other_uids, deadline).await
    };
    // The claim came out of the same budget: the mail shows when it is over.
    let left = deadline.saturating_duration_since(Instant::now());
    let (outcome, other) = tokio::join!(tokio::time::timeout(left, work), waiting);
    cancelled.store(true, Ordering::SeqCst);
    // This device's claim on what it didn't decide after all (the AI failed
    // or was too slow, a rule took the mail): another device may try.
    if let Some(id) = &claim {
        let marked = marked.into_inner().unwrap();
        let undecided: Vec<u32> = candidates
            .iter()
            .map(|candidate| candidate.uid)
            .filter(|uid| !marked.contains(uid))
            .collect();
        unclaim_mail(engine, account, folder, undecided, vec![id.clone()]).await;
    }
    let settled = settled.into_inner().unwrap();
    let rest: Vec<AiSortCandidate> = candidates
        .into_iter()
        .filter(|candidate| !settled.contains(&candidate.uid))
        .collect();
    let rest_uids: Vec<u32> = rest.iter().map(|candidate| candidate.uid).collect();
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(Some(error))) => {
            warn(format_args!(
                "account={account}: new mail left in Inbox: {error}"
            ));
            engine.ai_sort.set_error(&error);
            let detail = error.message.clone();
            give_up(engine, account, folder, uidvalidity, rest, "failed", detail).await;
        }
        Ok(Err(None)) | Err(_) => {
            warn(format_args!(
                "account={account}: the AI took too long; new mail left in Inbox"
            ));
            let detail = "The AI took too long.".to_string();
            give_up(engine, account, folder, uidvalidity, rest, "slow", detail).await;
        }
    }
    let mut moving = moving.into_inner().unwrap();
    let yielded = yielded.into_inner().unwrap();
    if !yielded.is_empty() {
        let left = store::ai_sort_left_to_other(
            &engine.db.lock().unwrap(),
            account,
            folder,
            &yielded,
            &[],
        );
        if let Err(error) = left {
            warn(format_args!(
                "could not show mail another device sorts: {error:#}"
            ));
        }
    }
    // Mail another device sorts: what it moved away isn't shown or announced
    // here, the rest shows; what it never finished is taken over later.
    if !others.is_empty() {
        let shown: Vec<u32> = other_uids
            .iter()
            .copied()
            .filter(|uid| !other.gone.contains(uid))
            .collect();
        if let Err(error) = store::ai_sort_left_to_other(
            &engine.db.lock().unwrap(),
            account,
            folder,
            &shown,
            &other.gone,
        ) {
            warn(format_args!(
                "could not show mail another device sorts: {error:#}"
            ));
        }
        moving.extend(other.gone.iter().copied());
        let mut open = other.open;
        let open: Vec<(AiSortCandidate, Vec<String>)> = others
            .into_iter()
            .filter_map(|candidate| {
                let at = open.iter().position(|(uid, _)| *uid == candidate.uid)?;
                Some((candidate, open.swap_remove(at).1))
            })
            .collect();
        watch_claims(engine, account, uidvalidity, open, started);
    }
    // A recording that got the store just before the flag was set stands: its
    // moves are queued, so they don't alert either.
    if !rest_uids.is_empty() {
        let late = store::ai_sort_queued_new_moves(
            &engine.db.lock().unwrap(),
            account,
            folder,
            &rest_uids,
        )
        .unwrap_or_default();
        moving.extend(late);
    }
    report(engine, account, folder);
    // What the AI left from another folder now lists in the Inbox.
    if !inbox {
        report(engine, account, "INBOX");
    }
    if !moving.is_empty() {
        refresh_after(
            engine,
            account,
            folder,
            Duration::from_secs(store::AI_SORT_MOVE_HOLD_SECS as u64 + 1),
        );
    }
    // Filed into a Meron folder: handled, and it arrives silently in its
    // mailbox (the caller announces it under the Meron folder when that
    // folder has notifications on; see `meron_arrivals`).
    let filed = filed.into_inner().unwrap();
    for (uid, _) in &filed {
        if !moving.contains(uid) {
            moving.push(*uid);
        }
    }
    SortedNewMail {
        silenced: moving,
        filed,
    }
}
