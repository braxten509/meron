//! Costs, progress and cancellation for sorting mail already in Inbox.
//! The manual action groups stored candidates into provider batches.
//! Each batch records the work completed so the frontend can report progress.

use super::*;

/// The setting, the key and what is to be sorted, per account: its emails
/// grouped by conversation, newest email first in each group.
type Work = Vec<(String, Vec<PromptFolder>, Vec<Vec<AiSortCandidate>>)>;

fn existing_work(engine: &Engine, config: &AiSortConfig) -> anyhow::Result<Work> {
    let db = engine.db.lock().unwrap();
    let meron = store::meron_folder_names(&db)?;
    // A live Meron folder takes every account's Inbox.
    let accounts = if store::ai_sort_has_live_meron(&db, config)? {
        store::sort_accounts(&db)?
    } else {
        config.accounts()
    };
    let mut work = Vec::new();
    for account in accounts {
        let folders = ai::prompt_folders(config, &account, &meron);
        if folders.is_empty() {
            continue;
        }
        let candidates = store::ai_sort_existing_candidates(&db, &account)?;
        let groups = store::ai_sort_group_conversations(
            &db,
            &account,
            store::EXISTING_RULE_FOLDER,
            candidates,
        )?;
        if !groups.is_empty() {
            work.push((account, folders, groups));
        }
    }
    Ok(work)
}

fn emails_in(groups: &[Vec<AiSortCandidate>]) -> usize {
    groups.iter().map(Vec::len).sum()
}

/// The models of `provider`, from the day's cache or fetched now.
pub(crate) async fn models(
    engine: &Arc<Engine>,
    provider: &str,
) -> Result<Vec<ai::Model>, AiError> {
    if let Some(models) = engine.ai_sort.cached_models(provider) {
        return Ok(models);
    }
    let key = loaded_keys(engine).await.get(provider).cloned();
    let list = if provider == "tinfoil" {
        // The list needs no key; the client is still verified first.
        let client = tinfoil_client(engine, key.as_deref().unwrap_or("none")).await?;
        (ai::tinfoil::models(&client).await?, true)
    } else {
        tokio::task::spawn_blocking(move || ai::openrouter::models(key.as_deref()))
            .await
            .map_err(|_| AiError::new(ErrorKind::Other, "The model list stopped."))??
    };
    let (list, complete) = list;
    // A list missing the decision models is shown but not kept, so a sort
    // never takes a decision model for a chat one from it.
    if complete {
        engine.ai_sort.cache_models(provider, list.clone());
    }
    Ok(list)
}

/// What "sort mail already in Inbox" would send, and about what it costs.
pub async fn estimate(engine: &Arc<Engine>) -> anyhow::Result<serde_json::Value> {
    let config = store::ai_sort_config(&engine.db.lock().unwrap())?;
    let work = existing_work(engine, &config)?;
    let total: usize = work.iter().map(|(_, _, groups)| emails_in(groups)).sum();
    let asked: usize = work.iter().map(|(_, _, groups)| groups.len()).sum();
    let model = if config.model.is_empty() || total == 0 {
        None
    } else {
        tokio::time::timeout(Duration::from_secs(8), models(engine, &config.provider))
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|list| list.into_iter().find(|model| model.id == config.model))
    };
    // A decision model is asked once per email and bills only what it reads.
    let decision = model.as_ref().is_some_and(|model| model.decision);
    let requests: usize = if decision {
        asked
    } else {
        work.iter()
            .map(|(_, _, groups)| groups.len().div_ceil(ai::BATCH))
            .sum()
    };
    let price = model.and_then(|model| Some((model.price_in_per_m?, model.price_out_per_m?)));
    let cost_usd = price.map(|(price_in, price_out)| {
        if decision {
            // Characters ÷ 4 as tokens: each email with the folder list.
            let mut tokens_in = 0usize;
            for (_, folders, groups) in &work {
                let overhead = ai::decision_overhead(folders);
                for candidate in groups.iter().filter_map(|group| group.first()) {
                    let start = if config.reads == "start" {
                        ai::START_CHARS
                    } else {
                        0
                    };
                    tokens_in += (sender_of(candidate).len()
                        + candidate.subject.len()
                        + 20
                        + start
                        + overhead)
                        / 4;
                }
            }
            return tokens_in as f64 * price_in / 1_000_000.0;
        }
        // Characters ÷ 4 as tokens: the instructions once per request, then
        // each email, its JSON and the answer's line for it.
        let mut tokens_in = 0usize;
        for (_, folders, groups) in &work {
            let system =
                ai::system_prompt(folders).len() + ai::answer_schema(folders).to_string().len();
            tokens_in += groups.len().div_ceil(ai::BATCH) * system / 4;
            for candidate in groups.iter().filter_map(|group| group.first()) {
                let start = if config.reads == "start" {
                    ai::START_CHARS
                } else {
                    0
                };
                tokens_in +=
                    (sender_of(candidate).len() + candidate.subject.len() + 40 + start) / 4;
            }
        }
        let tokens_out = asked * 30 + requests * 10;
        (tokens_in as f64 * price_in + tokens_out as f64 * price_out) / 1_000_000.0
    });
    let accounts: Vec<serde_json::Value> = work
        .iter()
        .map(|(account, _, groups)| {
            serde_json::json!({ "account": account, "count": emails_in(groups) })
        })
        .collect();
    Ok(serde_json::json!({
        "total": total,
        "requests": requests,
        "reads": config.reads,
        "provider": config.provider,
        "model": config.model,
        "cost_usd": cost_usd,
        "accounts": accounts,
    }))
}

/// Start sorting the mail already in the Inbox in the background. Answers with
/// how many emails it takes on. One job at a time.
pub async fn run_existing(engine: &Arc<Engine>) -> anyhow::Result<usize> {
    let config = store::ai_sort_config(&engine.db.lock().unwrap())?;
    anyhow::ensure!(!config.model.is_empty(), "Pick a model first.");
    let key = loaded_keys(engine)
        .await
        .get(&config.provider)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Add an API key first."))?;
    if config.provider == "tinfoil"
        && let Some(error) = proxy_blocks_tinfoil()
    {
        anyhow::bail!("{}", error.message);
    }
    let (work, total) = {
        let mut job = engine.ai_sort.job.lock().unwrap();
        anyhow::ensure!(job.is_none(), "AI sorting of the Inbox is already running.");
        let work = existing_work(engine, &config)?;
        let total: usize = work.iter().map(|(_, _, groups)| emails_in(groups)).sum();
        if total == 0 {
            return Ok(0);
        }
        *job = Some(Progress { total, done: 0 });
        (work, total)
    };
    engine.ai_sort.stop.store(false, Ordering::SeqCst);
    let run = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_micros() as i64;
    let job_engine = engine.clone();
    tokio::spawn(async move {
        run_job(&job_engine, &config, &key, work, run).await;
        *job_engine.ai_sort.job.lock().unwrap() = None;
    });
    Ok(total)
}

async fn run_job(engine: &Arc<Engine>, config: &AiSortConfig, key: &str, work: Work, run: i64) {
    let folder = store::EXISTING_RULE_FOLDER;
    'accounts: for (account, folders, groups) in work {
        let uidvalidity = store::get_folder_state(&engine.db.lock().unwrap(), &account, folder)
            .ok()
            .flatten()
            .map(|(validity, _)| validity);
        let Some(uidvalidity) = uidvalidity else {
            add_done(engine, emails_in(&groups));
            continue;
        };
        for chunk in groups.chunks(ai::BATCH) {
            if engine.ai_sort.stop.load(Ordering::SeqCst) {
                break 'accounts;
            }
            let count = emails_in(chunk);
            // One AI sort at a time per mailbox, as for new mail; and what that
            // sorted (or a rule took, or the user filed) meanwhile is no longer
            // this batch's.
            let lock = engine.ai_sort.sort_lock(&account, folder);
            let _sorting = lock.lock().await;
            let members: Vec<AiSortCandidate> = chunk.iter().flatten().cloned().collect();
            let open: BTreeSet<u32> =
                store::ai_sort_still_open(&engine.db.lock().unwrap(), &account, folder, &members)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|candidate| candidate.uid)
                    .collect();
            let chunk: Vec<Vec<AiSortCandidate>> = chunk
                .iter()
                .map(|group| {
                    group
                        .iter()
                        .filter(|candidate| open.contains(&candidate.uid))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .filter(|group| !group.is_empty())
                .collect();
            // Only one of the reader's devices sorts each email: what another
            // one claimed or decided on is not this batch's (see "Claims").
            let claim = claim_id(engine);
            let mut exact = true;
            let chunk: Vec<Vec<AiSortCandidate>> = match &claim {
                Some(id) => {
                    let uids: Vec<u32> = chunk.iter().flatten().map(|it| it.uid).collect();
                    let claimed = claim_mail(engine, &account, folder, &uids, id, CLAIM_WAIT).await;
                    exact = claimed.exact;
                    chunk
                        .into_iter()
                        .map(|group| {
                            group
                                .into_iter()
                                .filter(|candidate| claimed.mine.contains(&candidate.uid))
                                .collect::<Vec<_>>()
                        })
                        .filter(|group| !group.is_empty())
                        .collect()
                }
                None => chunk,
            };
            if chunk.is_empty() {
                add_done(engine, count);
                report(engine, &account, folder);
                continue;
            }
            // This device's claim goes off whatever it doesn't decide here.
            let claimed: Vec<u32> = chunk.iter().flatten().map(|it| it.uid).collect();
            let unclaim = |decided: Vec<u32>| {
                let open: Vec<u32> = claimed
                    .iter()
                    .copied()
                    .filter(|uid| !decided.contains(uid))
                    .collect();
                let ids: Vec<String> = claim.iter().cloned().collect();
                unclaim_mail(engine, &account, folder, open, ids)
            };
            let batch = representatives(&chunk);
            let batch = batch.as_slice();
            if config.reads == "start" {
                let uids: Vec<u32> = batch.iter().map(|candidate| candidate.uid).collect();
                let fetch = fetch_bodies_for_uids(
                    engine,
                    &account,
                    folder,
                    &uids,
                    engine.host.media_root(),
                );
                let _ = tokio::time::timeout(JOB_BODY_WAIT, fetch).await;
            }
            let deadline = Instant::now() + JOB_BATCH_BUDGET;
            let decided = tokio::time::timeout(
                JOB_BATCH_BUDGET,
                decide_batch(
                    engine, config, key, &folders, &account, folder, batch, deadline,
                ),
            )
            .await
            .unwrap_or_else(|_| Err(AiError::network("The AI took too long.")));
            match decided {
                Ok(decisions) => {
                    let decisions = expand_decisions(
                        &chunk,
                        decisions,
                        spam_folder(engine, &account).as_deref(),
                    );
                    if engine.ai_sort.stop.load(Ordering::SeqCst) {
                        unclaim(Vec::new()).await;
                        break 'accounts;
                    }
                    let decisions = match &claim {
                        Some(id) if !exact => {
                            confirm_claims(engine, &account, folder, decisions, id)
                                .await
                                .0
                        }
                        _ => decisions,
                    };
                    // Record first, then mark only what the store took on. The
                    // account's rule turn is held meanwhile, so a running rule
                    // worker can't move the new entries before their mark lands.
                    let rule_turn = engine
                        .tag_pushes
                        .lock()
                        .unwrap()
                        .entry(account.clone())
                        .or_default()
                        .clone();
                    let recorded = {
                        let _rule_turn = rule_turn.lock().await;
                        let recorded =
                            record(engine, &account, folder, uidvalidity, decisions, run, None)
                                .await
                                .unwrap_or_default();
                        let accepted = recorded.accepted.clone();
                        mark_sorted(engine, &account, folder, accepted, claim.as_deref()).await;
                        mark_filed(engine, &account, &recorded.filed_to).await;
                        recorded
                    };
                    unclaim(recorded.accepted.clone()).await;
                    if !recorded.moving.is_empty() {
                        spawn_rule_worker(engine, &account, folder);
                    }
                    engine.ai_sort.clear_error();
                }
                Err(error) => {
                    warn(format_args!(
                        "account={account}: sorting the Inbox: {error}"
                    ));
                    engine.ai_sort.set_error(&error);
                    let detail = error.message.clone();
                    let members: Vec<AiSortCandidate> = chunk.iter().flatten().cloned().collect();
                    give_up(
                        engine,
                        &account,
                        folder,
                        uidvalidity,
                        members,
                        "failed",
                        detail,
                    )
                    .await;
                    unclaim(Vec::new()).await;
                    if matches!(
                        error.kind,
                        ErrorKind::Key | ErrorKind::Credit | ErrorKind::Verify
                    ) {
                        break 'accounts;
                    }
                }
            }
            add_done(engine, count);
            report(engine, &account, folder);
        }
    }
}

fn add_done(engine: &Engine, count: usize) {
    if let Some(progress) = engine.ai_sort.job.lock().unwrap().as_mut() {
        progress.done = (progress.done + count).min(progress.total);
    }
}

/// Stop the job: what is done stays done, what is queued is dropped.
pub fn stop_existing(engine: &Engine) -> anyhow::Result<()> {
    engine.ai_sort.stop.store(true, Ordering::SeqCst);
    store::ai_sort_stop_run(&engine.db.lock().unwrap())
}
