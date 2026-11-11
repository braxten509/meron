//! The AI sorting pipeline: the step every sync of INBOX runs over its new mail
//! (and every sync of a plain folder server filters deliver into, whose mail
//! is filed into Meron folders or shown in the Inbox, never moved on the
//! server), the "sort mail already in Inbox" job, and Move back. The moves themselves go
//! through the rules action queue (retries, UIDVALIDITY checks, no alert for
//! moved mail), under rule ids `ai-sort:<log id>` that never show in Rules.

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
/// One batch of the "sort Inbox now" job, retry included.
const JOB_BATCH_BUDGET: Duration = Duration::from_secs(40);
/// Bodies for one batch of that job.
const JOB_BODY_WAIT: Duration = Duration::from_secs(15);

fn warn(what: std::fmt::Arguments<'_>) {
    crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "{what}");
}

// ---- Keys ------------------------------------------------------------------

/// The keys, loaded from the host once (off the store's lock, as keychain
/// calls can hang). The load goes on in its own task, so a caller that stops
/// waiting doesn't lose it.
pub async fn loaded_keys(engine: &Arc<Engine>) -> HashMap<String, String> {
    let slot = engine.ai_sort.keys.clone();
    let engine = engine.clone();
    let task = tokio::spawn(async move {
        let mut guard = slot.lock().await;
        if let Some(keys) = guard.as_ref() {
            return keys.clone();
        }
        let (keys, complete) = tokio::task::spawn_blocking(move || {
            let mut keys = HashMap::new();
            let mut complete = true;
            for provider in ai::PROVIDERS {
                match engine
                    .host
                    .load_named_secret(&engine.db, &ai::key_name(provider))
                {
                    Ok(Some(key)) => {
                        keys.insert(provider.to_string(), key);
                    }
                    Ok(None) => {}
                    Err(err) => {
                        complete = false;
                        warn(format_args!("could not load the {provider} key: {err:#}"));
                    }
                }
            }
            (keys, complete)
        })
        .await
        .unwrap_or_default();
        // A store that failed to answer is asked again next time.
        if complete {
            *guard = Some(keys.clone());
        }
        keys
    });
    task.await.unwrap_or_default()
}

/// Save or remove (`None`) a provider's key.
pub async fn store_key(
    engine: &Arc<Engine>,
    provider: &str,
    key: Option<String>,
) -> anyhow::Result<()> {
    let host_engine = engine.clone();
    let name = ai::key_name(provider);
    let value = key.clone();
    tokio::task::spawn_blocking(move || {
        host_engine
            .host
            .store_named_secret(&host_engine.db, &name, value.as_deref())
    })
    .await??;
    let _ = loaded_keys(engine).await;
    {
        let mut guard = engine.ai_sort.keys.lock().await;
        let keys = guard.get_or_insert_with(HashMap::new);
        match key {
            Some(key) => keys.insert(provider.to_string(), key),
            None => keys.remove(provider),
        };
    }
    if provider == "tinfoil" {
        *engine.ai_sort.tinfoil.lock().await = None;
    }
    Ok(())
}

// ---- Providers -------------------------------------------------------------

fn proxy_blocks_tinfoil() -> Option<AiError> {
    crate::proxy::global().is_some().then(|| {
        AiError::new(
            ErrorKind::Other,
            "Tinfoil can't be used while an app proxy is set.",
        )
    })
}

/// The verified Tinfoil client for `key`, verifying again past
/// [`ai::TINFOIL_TTL`] or after a failed check.
pub(crate) async fn tinfoil_client(
    engine: &Arc<Engine>,
    key: &str,
) -> Result<Arc<::tinfoil::Client>, AiError> {
    if let Some(err) = proxy_blocks_tinfoil() {
        return Err(err);
    }
    let hash = ai::key_hash(key);
    let mut slot = engine.ai_sort.tinfoil.lock().await;
    if let Some(cached) = slot.as_ref()
        && cached.key_hash == hash
        && cached.checked.elapsed() < ai::TINFOIL_TTL
    {
        return Ok(cached.client.clone());
    }
    let client = Arc::new(ai::tinfoil::connect(key).await?);
    *slot = Some(ai::TinfoilClient {
        client: client.clone(),
        checked: Instant::now(),
        key_hash: hash,
    });
    Ok(client)
}

/// Verify Tinfoil ahead of time, so new mail doesn't spend its 20 s on it.
pub(crate) fn warm_tinfoil(engine: &Arc<Engine>, key: &str) {
    let engine = engine.clone();
    let key = key.to_string();
    tokio::spawn(async move {
        {
            let slot = engine.ai_sort.tinfoil.lock().await;
            if slot.as_ref().is_some_and(|cached| {
                cached.key_hash == ai::key_hash(&key) && cached.checked.elapsed() < ai::TINFOIL_TTL
            }) {
                return;
            }
        }
        if let Err(err) = tinfoil_client(&engine, &key).await {
            warn(format_args!("tinfoil check: {err}"));
            if err.kind == ErrorKind::Verify {
                engine.ai_sort.set_error(&err);
            }
        }
    });
}

/// Whether `model` is known to take a JSON schema.
fn model_structured(engine: &Engine, provider: &str, model: &str) -> bool {
    engine
        .ai_sort
        .cached_models(provider)
        .and_then(|models| models.into_iter().find(|it| it.id == model))
        .is_some_and(|it| it.structured)
}

/// One request, no retry.
async fn request(
    engine: &Arc<Engine>,
    provider: &str,
    key: &str,
    structured: bool,
    body: serde_json::Value,
) -> Result<String, AiError> {
    if provider == "tinfoil" {
        let client = tinfoil_client(engine, key).await?;
        let result = ai::tinfoil::chat(&client, body).await;
        if let Err(err) = &result
            && ai::tinfoil::needs_reverify(err)
        {
            *engine.ai_sort.tinfoil.lock().await = None;
        }
        return result;
    }
    let key = key.to_string();
    // The blocking call has its own timeout and never touches the store; a
    // caller that stopped waiting just drops its answer.
    tokio::task::spawn_blocking(move || ai::openrouter::chat(&key, body, structured))
        .await
        .map_err(|_| AiError::new(ErrorKind::Other, "The AI request stopped."))?
}

/// A request and, when it failed in a way worth it and time is left, one
/// retry, after the service's `Retry-After`.
async fn request_with_retry(
    engine: &Arc<Engine>,
    provider: &str,
    key: &str,
    structured: bool,
    body: serde_json::Value,
    deadline: Instant,
) -> Result<String, AiError> {
    match request(engine, provider, key, structured, body.clone()).await {
        Ok(text) => Ok(text),
        Err(err) if err.retry => {
            let wait = err.retry_after.unwrap_or(Duration::from_millis(500));
            let left = deadline.saturating_duration_since(Instant::now());
            if left <= wait + Duration::from_secs(2) {
                return Err(err);
            }
            tokio::time::sleep(wait).await;
            request(engine, provider, key, structured, body).await
        }
        Err(err) => Err(err),
    }
}

/// How long a sort waits for the model list to learn if its model is a
/// decision model.
const MODEL_LIST_WAIT: Duration = Duration::from_secs(5);

/// Whether `model` is an OpenRouter decision model, from the day's full model
/// list (fetched when not cached). A list that can't be had fails the batch
/// rather than send a decision model to the chat endpoint.
async fn model_decision(engine: &Arc<Engine>, provider: &str, model: &str) -> Result<bool, AiError> {
    if provider != "openrouter" {
        return Ok(false);
    }
    if engine.ai_sort.cached_models(provider).is_none() {
        let _ = tokio::time::timeout(MODEL_LIST_WAIT, models(engine, provider)).await;
    }
    let Some(list) = engine.ai_sort.cached_models(provider) else {
        return Err(AiError::network("Couldn't load the model list."));
    };
    Ok(list.iter().any(|it| it.id == model && it.decision))
}

/// One decision request, no retry.
async fn decide_request(key: &str, body: serde_json::Value) -> Result<serde_json::Value, AiError> {
    let key = key.to_string();
    tokio::task::spawn_blocking(move || ai::openrouter::decide(&key, body))
        .await
        .map_err(|_| AiError::new(ErrorKind::Other, "The AI request stopped."))?
}

/// A decision request and, as for chat, one retry when worth it and time is left.
async fn decide_with_retry(
    key: &str,
    body: serde_json::Value,
    deadline: Instant,
) -> Result<serde_json::Value, AiError> {
    match decide_request(key, body.clone()).await {
        Ok(value) => Ok(value),
        Err(err) if err.retry => {
            let wait = err.retry_after.unwrap_or(Duration::from_millis(500));
            let left = deadline.saturating_duration_since(Instant::now());
            if left <= wait + Duration::from_secs(2) {
                return Err(err);
            }
            tokio::time::sleep(wait).await;
            decide_request(key, body).await
        }
        Err(err) => Err(err),
    }
}

/// Ask a decision model about each email of a batch, [`ai::DECISION_PARALLEL`]
/// at a time. A failed request fails the batch, as a failed chat request does.
async fn decide_batch_by_decision(
    config: &AiSortConfig,
    key: &str,
    folders: &[PromptFolder],
    batch: &[AiSortCandidate],
    emails: &[PromptEmail],
    spam: Option<&ai::SpamChoice>,
    deadline: Instant,
) -> Result<Vec<AiSortDecision>, AiError> {
    use futures::StreamExt;
    // Owned bodies and boxed futures, so the stream is `Send` for the spawned job.
    let key = key.to_string();
    let requests: Vec<_> = emails
        .iter()
        .map(|email| {
            let body = ai::decision_body_taught(&config.model, folders, email, spam);
            let key = key.clone();
            Box::pin(async move { decide_with_retry(&key, body, deadline).await })
        })
        .collect();
    let mut answers = futures::stream::iter(requests).buffered(ai::DECISION_PARALLEL);
    let mut picks = Vec::with_capacity(emails.len());
    let mut asked = emails.iter();
    while let Some(answer) = answers.next().await {
        // Dropping the stream drops the requests not yet answered.
        let email = asked.next();
        let automated = email.is_some_and(|email| email.automated.is_some());
        let spam = spam.filter(|_| email.is_some_and(|email| email.spam_backed));
        picks.push(ai::parse_decision_taught(&answer?, folders, automated, spam));
    }
    Ok(batch
        .iter()
        .zip(picks)
        .map(|(candidate, (pick, why))| AiSortDecision {
            candidate: candidate.clone(),
            target: pick.map(|pick| pick.target),
            why,
        })
        .collect())
}

fn sender_of(candidate: &AiSortCandidate) -> String {
    match (candidate.from_name.trim(), candidate.from_addr.trim()) {
        ("", addr) => addr.to_string(),
        (name, "") => name.to_string(),
        (name, addr) => format!("{name} <{addr}>"),
    }
}

/// The longest the bulk-header read may take, and the time it must leave for
/// the AI itself; with less time left it is skipped.
const BULK_SIGNAL_BUDGET: Duration = Duration::from_secs(3);
const BULK_SIGNAL_SPARE: Duration = Duration::from_secs(8);

/// Bulk or automated mail headers for a batch, by UID, from one header-only
/// fetch. Any failure, timeout or lack of time gives an empty map: the sort
/// goes on without them.
async fn bulk_signals(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    batch: &[AiSortCandidate],
    deadline: Instant,
) -> HashMap<u32, &'static str> {
    let left = deadline.saturating_duration_since(Instant::now());
    let Some(budget) = left
        .checked_sub(BULK_SIGNAL_SPARE)
        .map(|room| room.min(BULK_SIGNAL_BUDGET))
        .filter(|budget| !budget.is_zero())
    else {
        return HashMap::new();
    };
    let uids: Vec<u32> = batch.iter().map(|candidate| candidate.uid).collect();
    let folder = folder.to_string();
    let read = engine.with_transfer_session(account, budget, |session| {
        let uids = uids.clone();
        let folder = folder.clone();
        Box::pin(async move { imap::fetch_bulk_signals(session, &folder, &uids).await })
    });
    // The session's own budget should end it; this is only a backstop.
    match tokio::time::timeout(budget + Duration::from_secs(1), read).await {
        Ok(Ok(signals)) => signals,
        _ => HashMap::new(),
    }
}

/// Push the lesson tags that changed since the last call (see
/// [`store::ai_lessons_take_unpushed`]) to their servers in the background,
/// from the desktop's runtime or the phone's.
pub fn push_lesson_tags(engine: &Arc<Engine>) {
    for account in store::ai_lessons_take_unpushed() {
        let engine = engine.clone();
        let work = async move {
            if let Err(err) = push_pending_tags(&engine, &account).await {
                warn(format_args!("account={account}: lesson tag push failed: {err:#}"));
            }
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(work);
            }
            Err(_) => {
                crate::ffi::spawn_engine_task(work);
            }
        }
    }
}

/// The reader's lessons for a batch: for each email the lessons most like it
/// under the names the AI sees, whether one backs a Spam answer, and the
/// batch's Spam choice when any email is backed for it. A lesson into a
/// Meron folder that isn't one of `folders`, or into Spam on an account
/// without a Spam folder, isn't shown (it still counts as the reader's
/// latest word when it is). Lessons from every account count: the
/// same sender writes to more than one of the reader's addresses.
struct Taught {
    earlier: Vec<Vec<ai::lessons::PromptLesson>>,
    backed: Vec<bool>,
    spam: Option<ai::SpamChoice>,
}

fn taught_for(
    engine: &Engine,
    account: &str,
    folders: &[PromptFolder],
    batch: &[AiSortCandidate],
) -> Taught {
    use ai::lessons::{Lesson, PromptLesson};
    let none = || Taught {
        earlier: vec![Vec::new(); batch.len()],
        backed: vec![false; batch.len()],
        spam: None,
    };
    let (rows, junk) = {
        let db = engine.db.lock().unwrap();
        let rows = match store::ai_lessons(&db) {
            Ok(rows) => rows,
            Err(err) => {
                warn(format_args!("lessons not read: {err:#}"));
                return none();
            }
        };
        let junk = store::folder_for_role(&db, account, "junk").ok().flatten();
        (rows, junk)
    };
    if rows.is_empty() {
        return none();
    }
    let spam = junk.and_then(|folder| ai::SpamChoice::new(folders, &folder));
    // Each usable lesson with the name the AI sees for its place.
    let mut names = Vec::new();
    let mut lessons = Vec::new();
    for row in rows {
        let Some(place) = store::LessonPlace::from_code(&row.place) else {
            continue;
        };
        let name = match &place {
            store::LessonPlace::Spam => spam.as_ref().map(|spam| spam.name.clone()),
            store::LessonPlace::Inbox => Some("Inbox".to_string()),
            store::LessonPlace::Meron(id) => folders
                .iter()
                .find(|folder| folder.target == ai::SortTarget::Meron(id.clone()))
                .map(|folder| folder.name.clone()),
        };
        names.push(name);
        lessons.push(Lesson {
            from_name: row.from_name,
            from_addr: row.from_addr,
            subject: row.subject,
            place,
            at: row.at,
        });
    }
    let mut out = none();
    for (at, candidate) in batch.iter().enumerate() {
        for lesson in ai::lessons::alike(&lessons, &candidate.from_addr, &candidate.subject) {
            // `alike` answers with borrows of `lessons`: find its name by place in the list.
            let index = lessons
                .iter()
                .position(|known| std::ptr::eq(known, lesson))
                .unwrap_or_default();
            let Some(name) = &names[index] else {
                continue;
            };
            if lesson.place == store::LessonPlace::Spam {
                out.backed[at] = true;
            }
            out.earlier[at].push(PromptLesson {
                from: lesson.sender(),
                subject: lesson.subject.clone(),
                moved_to: name.clone(),
            });
        }
    }
    if out.backed.iter().any(|backed| *backed) {
        out.spam = spam;
    }
    out
}

/// The emails as the AI sees them: sender, subject, when chosen the start of
/// the cached body, and whether it looks like bulk or automated mail (from
/// `signals`, the sender address, or "unsubscribe" in a cached body's start).
/// Nothing else leaves the device.
fn prompt_emails(
    engine: &Engine,
    account: &str,
    folder: &str,
    candidates: &[AiSortCandidate],
    with_start: bool,
    signals: &HashMap<u32, &'static str>,
    taught: &Taught,
) -> Vec<PromptEmail> {
    let db = engine.db.lock().unwrap();
    candidates
        .iter()
        .enumerate()
        .map(|(at, candidate)| {
            let start = store::get_cached_message(&db, account, folder, candidate.uid)
                .ok()
                .flatten()
                .map(|message| crate::parse::text_start(&message.body, ai::START_CHARS))
                .filter(|start| !start.trim().is_empty());
            let automated = ai::automated_reason(
                signals.get(&candidate.uid).copied(),
                &candidate.from_addr,
                start.as_deref(),
            );
            PromptEmail {
                id: (at + 1).to_string(),
                from: sender_of(candidate),
                subject: candidate.subject.clone(),
                start: start.filter(|_| with_start),
                automated,
                earlier: taught.earlier.get(at).cloned().unwrap_or_default(),
                spam_backed: taught.backed.get(at).copied().unwrap_or(false),
            }
        })
        .collect()
}

/// Ask the AI about one batch and turn its answer into decisions.
#[allow(clippy::too_many_arguments)]
async fn decide_batch(
    engine: &Arc<Engine>,
    config: &AiSortConfig,
    key: &str,
    folders: &[PromptFolder],
    account: &str,
    folder: &str,
    batch: &[AiSortCandidate],
    deadline: Instant,
) -> Result<Vec<AiSortDecision>, AiError> {
    let signals = bulk_signals(engine, account, folder, batch, deadline).await;
    let taught = taught_for(engine, account, folders, batch);
    let emails = prompt_emails(
        engine,
        account,
        folder,
        batch,
        config.reads == "start",
        &signals,
        &taught,
    );
    let spam = taught.spam.as_ref();
    if model_decision(engine, &config.provider, &config.model).await? {
        return decide_batch_by_decision(config, key, folders, batch, &emails, spam, deadline)
            .await;
    }
    let ids: Vec<String> = emails.iter().map(|email| email.id.clone()).collect();
    let backed: std::collections::HashSet<String> = emails
        .iter()
        .filter(|email| email.spam_backed)
        .map(|email| email.id.clone())
        .collect();
    let automated: std::collections::HashSet<String> = emails
        .iter()
        .filter(|email| email.automated.is_some())
        .map(|email| email.id.clone())
        .collect();
    let body = ai::chat_body_taught(&config.model, folders, &emails, spam);
    let structured = model_structured(engine, &config.provider, &config.model);
    let text =
        request_with_retry(engine, &config.provider, key, structured, body, deadline).await?;
    let mut picks = ai::parse_answer_taught(
        &text,
        &ids,
        folders,
        &automated,
        spam.map(|spam| (spam, &backed)),
    );
    Ok(batch
        .iter()
        .zip(ids)
        .map(|(candidate, id)| {
            let (pick, why) = picks.remove(&id).unwrap_or((None, String::new()));
            AiSortDecision {
                candidate: candidate.clone(),
                target: pick.as_ref().map(|pick| pick.target.clone()),
                why: pick.map(|pick| pick.why).unwrap_or(why),
            }
        })
        .collect())
}

/// One answer per conversation: the AI is asked about each group's newest
/// email (its first) and the answer stands for every email of the group (see
/// [`expand_decisions`]).
fn representatives(groups: &[Vec<AiSortCandidate>]) -> Vec<AiSortCandidate> {
    groups.iter().filter_map(|group| group.first().cloned()).collect()
}

/// Give each group's answer to every email of the group.
///
/// Except Spam (`spam`: the account's Spam folder), the careful answer: it was
/// backed by a lesson about the asked email's sender, so it only stands for
/// the group's emails from that same sender. Another person's email in the
/// same conversation stays in the Inbox.
fn expand_decisions(
    groups: &[Vec<AiSortCandidate>],
    decisions: Vec<AiSortDecision>,
    spam: Option<&str>,
) -> Vec<AiSortDecision> {
    let mut out = Vec::new();
    for (group, decision) in groups.iter().zip(decisions) {
        let to_spam = matches!(
            (&decision.target, spam),
            (Some(ai::SortTarget::Server(target)), Some(spam)) if target == spam
        );
        let asked = decision.candidate.from_addr.trim().to_lowercase();
        for member in group {
            let other_sender = member.from_addr.trim().to_lowercase() != asked;
            if to_spam && other_sender {
                out.push(AiSortDecision {
                    candidate: member.clone(),
                    target: None,
                    why: String::new(),
                });
                continue;
            }
            out.push(AiSortDecision {
                candidate: member.clone(),
                target: decision.target.clone(),
                why: decision.why.clone(),
            });
        }
    }
    out
}

/// The account's Spam folder, which only a lesson-backed answer can name.
fn spam_folder(engine: &Engine, account: &str) -> Option<String> {
    store::folder_for_role(&engine.db.lock().unwrap(), account, "junk")
        .ok()
        .flatten()
}

/// Group `candidates` by conversation (see
/// [`store::ai_sort_group_conversations`]); on a store error each email is its
/// own conversation.
fn group_conversations(
    engine: &Engine,
    account: &str,
    folder: &str,
    candidates: Vec<AiSortCandidate>,
) -> Vec<Vec<AiSortCandidate>> {
    let fallback: Vec<Vec<AiSortCandidate>> =
        candidates.iter().cloned().map(|candidate| vec![candidate]).collect();
    store::ai_sort_group_conversations(&engine.db.lock().unwrap(), account, folder, candidates)
        .unwrap_or_else(|err| {
            warn(format_args!("account={account}: grouping by conversation: {err:#}"));
            fallback
        })
}

// ---- Helpers ---------------------------------------------------------------

fn report(engine: &Engine, account: &str, folder: &str) {
    if let Some(report) = engine.rule_progress.get() {
        report(account, folder);
    }
}

/// Refresh the screen once a hold has run out, whatever became of the step that
/// set it.
fn refresh_after(engine: &Arc<Engine>, account: &str, folder: &str, after: Duration) {
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
async fn record(
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
        Ok(Err(err)) => {
            warn(format_args!("could not record the AI's decisions: {err:#}"));
            Some(Default::default())
        }
        Err(_) => Some(Default::default()),
    }
}

/// Put `$MeronSorted` on mail the AI decided on. Best effort: a mailbox that
/// refuses new keywords leaves only this device's memory.
async fn mark_sorted(engine: &Arc<Engine>, account: &str, folder: &str, uids: Vec<u32>) {
    if uids.is_empty() {
        return;
    }
    let marked = tokio::time::timeout(
        MARK_WAIT,
        engine.with_write_session(account, |session| {
            let folder = folder.to_string();
            let uids = uids.clone();
            Box::pin(async move { imap::mark_ai_sorted(session, &folder, &uids).await })
        }),
    )
    .await;
    match marked {
        Ok(Ok(true)) => {}
        Ok(Ok(false)) => warn(format_args!(
            "account={account}: the mailbox takes no new keywords; sorted mail is remembered here only"
        )),
        Ok(Err(err)) => warn(format_args!("account={account}: sorted mark failed: {err:#}")),
        Err(_) => warn(format_args!("account={account}: sorted mark timed out")),
    }
}

/// Put the Meron-folder mark on Inbox mail the AI just filed into a Meron
/// folder, so the user's other devices file it the same way (see
/// [`crate::meron_marks`]). Best effort.
async fn mark_filed(engine: &Arc<Engine>, account: &str, filed: &[(u32, String)]) {
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
        crate::meron_marks::write_all(engine, crate::meron_marks::jobs_for(account, uids, Some(id)))
            .await;
    }
}

async fn give_up(
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
        if let Err(err) = store::ai_sort_give_up(
            &engine.db.lock().unwrap(),
            &account,
            &folder,
            uidvalidity,
            &candidates,
            state,
            &detail,
        ) {
            warn(format_args!("could not show held mail again: {err:#}"));
        }
    })
    .await;
}

// ---- New mail ----------------------------------------------------------------

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
        let sortable = store::sort_accounts(&db).ok()?.iter().any(|id| id == account);
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
        if let Some(err) = proxy_blocks_tinfoil() {
            engine.ai_sort.set_error(&err);
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

/// Sort the new mail a sync of `folder` held back, within [`AI_SORT_BUDGET`].
/// Answers with the UIDs queued to move or filed into a Meron folder, which
/// the caller doesn't alert for, and what was filed where. Mail the AI isn't
/// sure about, or that it couldn't answer for in time, shows in the Inbox; a
/// late answer is dropped.
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
    let deadline = Instant::now() + AI_SORT_BUDGET;
    let inbox = folder.eq_ignore_ascii_case("INBOX");
    // Set by the timeout path, so a recording still waiting for the store gives up.
    let cancelled = Arc::new(AtomicBool::new(false));
    let settled: std::sync::Mutex<BTreeSet<u32>> = Default::default();
    let moving: std::sync::Mutex<Vec<u32>> = Default::default();
    let filed: std::sync::Mutex<Vec<(u32, String)>> = Default::default();
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
            let fetch = fetch_bodies_for_uids(engine, account, folder, &uids, engine.host.media_root());
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
            filed.lock().unwrap().extend(recorded.filed_to.iter().cloned());
            // After the decision, before the move, so the mark travels with it;
            // only on what the store took on. Not outside the Inbox: sorting
            // there is this device's view only and writes nothing to the server.
            if inbox {
                mark_sorted(engine, account, folder, recorded.accepted).await;
                mark_filed(engine, account, &recorded.filed_to).await;
            }
            engine.ai_sort.clear_error();
        }
        Ok::<(), Option<AiError>>(())
    };
    let outcome = tokio::time::timeout(AI_SORT_BUDGET, work).await;
    cancelled.store(true, Ordering::SeqCst);
    let settled = settled.into_inner().unwrap();
    let rest: Vec<AiSortCandidate> = candidates
        .into_iter()
        .filter(|candidate| !settled.contains(&candidate.uid))
        .collect();
    let rest_uids: Vec<u32> = rest.iter().map(|candidate| candidate.uid).collect();
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(Some(err))) => {
            warn(format_args!("account={account}: new mail left in Inbox: {err}"));
            engine.ai_sort.set_error(&err);
            let detail = err.message.clone();
            give_up(engine, account, folder, uidvalidity, rest, "failed", detail).await;
        }
        Ok(Err(None)) | Err(_) => {
            warn(format_args!("account={account}: the AI took too long; new mail left in Inbox"));
            let detail = "The AI took too long.".to_string();
            give_up(engine, account, folder, uidvalidity, rest, "slow", detail).await;
        }
    }
    let mut moving = moving.into_inner().unwrap();
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

// ---- New mail outside the Inbox -------------------------------------------

/// Least time between two checks of one account's other folders, so repeated
/// pulls to refresh don't repeat the STATUS round.
const FOLDER_CHECK_EVERY: Duration = Duration::from_secs(60);
/// How many headers a sync of a changed folder reads, as an IDLE refresh does.
const FOLDER_CHECK_LIMIT: u32 = 50;

/// When each account's other folders were last checked.
static FOLDER_CHECKS: std::sync::Mutex<Option<HashMap<String, Instant>>> =
    std::sync::Mutex::new(None);
/// Most new mail one check reads per folder past a sync's window: more than
/// [`FOLDER_CHECK_LIMIT`] arrived at once; the rest waits for the next check.
const FOLDER_CARRY_MAX: usize = 100;

/// Folders the server says can't be opened (a `\Noselect` parent, or gone),
/// skipped from then on while the app runs.
static FOLDER_CHECK_FAILED: std::sync::Mutex<BTreeSet<(String, String)>> =
    std::sync::Mutex::new(BTreeSet::new());

/// Spam mail already looked at for another device's lesson, per account, so
/// mail a sync can't reach doesn't start one every check: mail found with the
/// lesson's tag, and (`1`) mail found to have lost it.
static SPAM_LESSONS_SEEN: std::sync::Mutex<Option<HashMap<String, (BTreeSet<u32>, BTreeSet<u32>)>>> =
    std::sync::Mutex::new(None);

/// Most Spam mail one check reads for its lesson tag; the rest waits for the
/// next check.
const SPAM_LESSON_READ_MAX: usize = 200;

/// Learn the Spam lessons the reader's other devices taught: mail in the
/// account's Spam folder carries the lesson's tag (see `store::ai_lessons`),
/// but nothing syncs that folder in the background, so the tag would only be
/// read once the reader opens it. One SEARCH for the tag; then just the mail
/// it names whose tag this device hasn't read, and the mail it no longer
/// names that this device read the tag on (the lesson was forgotten
/// elsewhere), is read, headers only. Inbox and Meron folder lessons need
/// none of this: their mail is in the Inbox, which every sync reads.
async fn check_taught_spam(engine: &Arc<Engine>, account: &str) {
    let Some(junk) = store::folder_for_role(&engine.db.lock().unwrap(), account, "junk")
        .ok()
        .flatten()
    else {
        return;
    };
    let tag = store::LessonPlace::Spam.tag();
    let keyword = crate::tags::tag_to_keyword(&tag);
    let found = engine
        .with_read_session(account, |session| {
            let junk = junk.clone();
            let keyword = keyword.clone();
            Box::pin(async move { imap::uids_with_keyword(session, &junk, &keyword).await })
        })
        .await;
    let uids = match found {
        Ok(uids) => uids,
        Err(err) => {
            warn(format_args!("account={account}: Spam lesson check: {err:#}"));
            return;
        }
    };
    let found: BTreeSet<u32> = uids.iter().copied().collect();
    // Mail this device read the tag on that the server no longer names.
    let had = store::ai_lesson_tagged_uids(&engine.db.lock().unwrap(), account, &junk, &tag)
        .unwrap_or_default();
    let (fresh, lost): (Vec<u32>, Vec<u32>) = {
        let mut seen = SPAM_LESSONS_SEEN.lock().unwrap();
        let (tagged, untagged) = seen
            .get_or_insert_with(HashMap::new)
            .entry(account.to_string())
            .or_default();
        // Tagged again later counts as new again, and the other way round.
        tagged.retain(|uid| found.contains(uid));
        untagged.retain(|uid| !found.contains(uid));
        (
            uids.into_iter().filter(|uid| tagged.insert(*uid)).collect(),
            had.into_iter()
                .filter(|uid| !found.contains(uid) && untagged.insert(*uid))
                .collect(),
        )
    };
    let unread = !fresh.is_empty()
        && store::ai_lesson_tag_unread(&engine.db.lock().unwrap(), account, &junk, &fresh, &tag)
            .unwrap_or(false);
    let mut wanted: Vec<u32> = lost.clone();
    if unread {
        wanted.extend(&fresh);
    }
    if wanted.is_empty() {
        return;
    }
    wanted.sort_unstable_by(|a, b| b.cmp(a));
    let later = wanted.split_off(wanted.len().min(SPAM_LESSON_READ_MAX));
    // Mail not read this time is looked at again at the next check.
    let look_again = |uids: &[u32]| {
        let mut seen = SPAM_LESSONS_SEEN.lock().unwrap();
        if let Some((tagged, untagged)) = seen.as_mut().and_then(|seen| seen.get_mut(account)) {
            for uid in uids {
                tagged.remove(uid);
                untagged.remove(uid);
            }
        }
    };
    look_again(&later);
    let fetched = engine
        .with_read_session(account, |session| {
            let junk = junk.clone();
            let wanted = wanted.clone();
            Box::pin(async move { imap::fetch_headers_by_uid(session, &junk, &wanted).await })
        })
        .await;
    let stored = fetched.and_then(|mut headers| {
        for header in &mut headers {
            header.folder = junk.clone();
        }
        store::upsert_read_messages(&engine.db.lock().unwrap(), account, &junk, &headers)
    });
    if let Err(err) = stored {
        warn(format_args!("account={account}: Spam lessons not read: {err:#}"));
        look_again(&wanted);
    }
}

/// Check the folders AI sorting watches besides the Inbox (mail a server filter
/// delivered straight there) for new mail, and sync the ones that got some, so
/// their new mail is sorted like the Inbox's. Run after a refresh or background
/// sync of the account's Inbox. Cheap: nothing when sorting is off or not set
/// up for the account; at most once a minute per account; one STATUS per
/// folder over one pooled session; only folders whose UIDNEXT or UIDVALIDITY
/// moved are synced. Nothing is written to the server. Answers with the new
/// mail it filed into Meron folders with notifications on, for the caller to
/// announce.
pub async fn check_watched_folders(engine: &Arc<Engine>, account: &str) -> Vec<MeronArrivals> {
    let mut alerts = Vec::new();
    if store::is_local_account(account) || engine.is_paused(account) {
        return alerts;
    }
    let Some(plan) = plan_for(engine, account, "INBOX").await else {
        return alerts;
    };
    {
        let mut checks = FOLDER_CHECKS.lock().unwrap();
        let checks = checks.get_or_insert_with(HashMap::new);
        if checks
            .get(account)
            .is_some_and(|at| at.elapsed() < FOLDER_CHECK_EVERY)
        {
            return alerts;
        }
        checks.insert(account.to_string(), Instant::now());
    }
    // One check per account at a time.
    let key = format!("ai-folders:{account}");
    if !engine.syncing.lock().unwrap().insert(key.clone()) {
        return alerts;
    }
    struct Claim<'a>(&'a Engine, String);
    impl Drop for Claim<'_> {
        fn drop(&mut self) {
            self.0.syncing.lock().unwrap().remove(&self.1);
        }
    }
    let _claim = Claim(engine, key);

    check_taught_spam(engine, account).await;

    let folders = {
        let db = engine.db.lock().unwrap();
        store::ai_sort_check_folders(&db, &plan.config, account).unwrap_or_else(|err| {
            warn(format_args!("account={account}: folder check skipped: {err:#}"));
            Vec::new()
        })
    };
    let failed = FOLDER_CHECK_FAILED.lock().unwrap().clone();
    let folders: Vec<store::AiSortCheckFolder> = folders
        .into_iter()
        .filter(|folder| !failed.contains(&(account.to_string(), folder.name.clone())))
        .collect();
    if folders.is_empty() {
        return alerts;
    }
    let names: Vec<String> = folders.iter().map(|folder| folder.name.clone()).collect();
    // Per folder: its `(UIDVALIDITY, UIDNEXT)`, or the server's refusal and
    // whether it means the folder can't be opened at all. A failing connection
    // ends the round instead; every folder is asked again next check.
    let statuses = engine
        .with_read_session(account, |session| {
            let names = names.clone();
            Box::pin(async move {
                let mut out = Vec::with_capacity(names.len());
                for folder in names {
                    match imap::folder_status(session, &folder).await {
                        Ok(status) => out.push(Ok(status)),
                        Err(err) if imap::is_server_refusal(&err) => out.push(Err((
                            imap::is_unselectable_mailbox(&err),
                            format!("{err:#}"),
                        ))),
                        Err(err) => return Err(err),
                    }
                }
                anyhow::Ok(out)
            })
        })
        .await;
    let statuses = match statuses {
        Ok(statuses) => statuses,
        Err(err) => {
            warn(format_args!("account={account}: folder check: {err:#}"));
            return alerts;
        }
    };
    let mut changed = Vec::new();
    for (folder, status) in folders.iter().zip(statuses) {
        match status {
            // Never synced, new UIDs, or new mail since the last sync. A first
            // sync only starts the folder's watch: nothing is sorted then.
            Ok(now) if folder.state != Some(now) => changed.push(folder.name.clone()),
            Ok(_) => {}
            // `\Noselect` or gone: skipped from now on while the app runs.
            Err((true, err)) => {
                warn(format_args!(
                    "account={account} folder={}: can't be opened, skipped: {err}",
                    folder.name
                ));
                FOLDER_CHECK_FAILED
                    .lock()
                    .unwrap()
                    .insert((account.to_string(), folder.name.clone()));
            }
            // Refused for now (busy, unavailable…): asked again next check.
            Err((false, err)) => warn(format_args!(
                "account={account} folder={}: STATUS refused, trying again later: {err}",
                folder.name
            )),
        }
    }
    for folder in &changed {
        match sync_background_messages(engine, account, folder, FOLDER_CHECK_LIMIT).await {
            Ok(synced) => alerts.extend(synced.meron_arrivals),
            Err(err) if err.is::<BackgroundSyncCancelled>() => {}
            Err(err) => {
                warn(format_args!("account={account} folder={folder}: check sync failed: {err:#}"))
            }
        }
    }
    // More new mail than one sync's window holds: read the rest, oldest first,
    // a bounded amount per folder per check; what's left waits for the next.
    let behind: Vec<String> = {
        let db = engine.db.lock().unwrap();
        store::ai_sort_check_folders(&db, &plan.config, account)
            .unwrap_or_default()
            .into_iter()
            .filter(|folder| folder.behind())
            .map(|folder| folder.name)
            .collect()
    };
    for folder in behind {
        if engine.is_paused(account) {
            break;
        }
        match super::sync::sort_carried_mail(engine, account, &folder, FOLDER_CARRY_MAX).await {
            Ok(filed) => alerts.extend(filed),
            Err(err) => warn(format_args!(
                "account={account} folder={folder}: reading older new mail failed: {err:#}"
            )),
        }
    }
    alerts
}

/// What a sync announces: its arrivals less what rules or the AI moved or are
/// moving (`silenced`), plus mail whose AI move failed for good (`late`).
pub(crate) fn settle_arrivals(
    arrivals: &mut Vec<imap::MessageHeader>,
    silenced: &BTreeSet<u32>,
    late: Vec<imap::MessageHeader>,
) {
    arrivals.retain(|message| !silenced.contains(&message.uid));
    for header in late {
        if !arrivals.iter().any(|known| known.uid == header.uid) {
            arrivals.push(header);
        }
    }
}

// ---- Sort mail already in Inbox ------------------------------------------

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
                    let start = if config.reads == "start" { ai::START_CHARS } else { 0 };
                    tokens_in +=
                        (sender_of(candidate).len() + candidate.subject.len() + 20 + start + overhead) / 4;
                }
            }
            return tokens_in as f64 * price_in / 1_000_000.0;
        }
        // Characters ÷ 4 as tokens: the instructions once per request, then
        // each email, its JSON and the answer's line for it.
        let mut tokens_in = 0usize;
        for (_, folders, groups) in &work {
            let system = ai::system_prompt(folders).len() + ai::answer_schema(folders).to_string().len();
            tokens_in += groups.len().div_ceil(ai::BATCH) * system / 4;
            for candidate in groups.iter().filter_map(|group| group.first()) {
                let start = if config.reads == "start" { ai::START_CHARS } else { 0 };
                tokens_in += (sender_of(candidate).len() + candidate.subject.len() + 40 + start) / 4;
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
        && let Some(err) = proxy_blocks_tinfoil()
    {
        anyhow::bail!("{}", err.message);
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
            if chunk.is_empty() {
                add_done(engine, count);
                report(engine, &account, folder);
                continue;
            }
            let batch = representatives(&chunk);
            let batch = batch.as_slice();
            if config.reads == "start" {
                let uids: Vec<u32> = batch.iter().map(|candidate| candidate.uid).collect();
                let fetch =
                    fetch_bodies_for_uids(engine, &account, folder, &uids, engine.host.media_root());
                let _ = tokio::time::timeout(JOB_BODY_WAIT, fetch).await;
            }
            let deadline = Instant::now() + JOB_BATCH_BUDGET;
            let decided = tokio::time::timeout(
                JOB_BATCH_BUDGET,
                decide_batch(engine, config, key, &folders, &account, folder, batch, deadline),
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
                        break 'accounts;
                    }
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
                        mark_sorted(engine, &account, folder, recorded.accepted.clone()).await;
                        mark_filed(engine, &account, &recorded.filed_to).await;
                        recorded
                    };
                    if !recorded.moving.is_empty() {
                        spawn_rule_worker(engine, &account, folder);
                    }
                    engine.ai_sort.clear_error();
                }
                Err(err) => {
                    warn(format_args!("account={account}: sorting the Inbox: {err}"));
                    engine.ai_sort.set_error(&err);
                    let detail = err.message.clone();
                    let members: Vec<AiSortCandidate> = chunk.iter().flatten().cloned().collect();
                    give_up(engine, &account, folder, uidvalidity, members, "failed", detail)
                        .await;
                    if matches!(err.kind, ErrorKind::Key | ErrorKind::Credit | ErrorKind::Verify) {
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

// ---- Move back -------------------------------------------------------------

/// Put an email the AI moved back in the Inbox. It isn't sorted again: it is
/// already observed (not new), decided on this device, and carries the mark.
pub async fn move_back(engine: &Arc<Engine>, id: i64) -> anyhow::Result<()> {
    let entry = store::ai_sort_log_entry(&engine.db.lock().unwrap(), id)?
        .ok_or_else(|| anyhow::anyhow!("That entry is gone from the log."))?;
    anyhow::ensure!(entry.state == "moved", "This email wasn't moved by the AI.");
    if !entry.meron_folder_id.is_empty() {
        return move_back_from_meron(engine, id, &entry);
    }
    let target = entry.to_folder.clone();
    let shown = crate::utf7::decode(&target);
    let not_found = || anyhow::anyhow!("Couldn't find it; look in {shown}");
    if entry.message_id.trim().is_empty() {
        return Err(not_found());
    }
    let cached = store::ai_sort_find_cached(
        &engine.db.lock().unwrap(),
        &entry.account,
        &target,
        &entry.message_id,
    )?;
    let uid = match cached {
        Some(uid) => Some(uid),
        None => {
            let media_root = engine.host.media_root();
            let found = engine
                .with_read_session(&entry.account, |session| {
                    let target = target.clone();
                    let ids = vec![entry.message_id.clone()];
                    let media_root = media_root.clone();
                    let account = entry.account.clone();
                    Box::pin(async move {
                        imap::fetch_by_message_ids(session, &target, &ids, &media_root, &account).await
                    })
                })
                .await?;
            found.first().map(|message| message.header.uid)
        }
    };
    let uid = uid.ok_or_else(not_found)?;
    move_within_account(engine, &entry.account, &target, "INBOX", &[uid]).await?;
    {
        let db = engine.db.lock().unwrap();
        store::ai_sort_set_log_state(&db, id, "moved_back", "")?;
        // The AI was wrong here: mail like this stays in the Inbox from now on.
        store::ai_lesson_from_log(&db, &entry);
    }
    push_lesson_tags(engine);
    report(engine, &entry.account, "INBOX");
    Ok(())
}

/// Move back for mail the AI filed into a Meron folder: take its conversation
/// out of that folder, on this device only. Not sorted again.
fn move_back_from_meron(
    engine: &Arc<Engine>,
    id: i64,
    entry: &store::AiSortLogEntry,
) -> anyhow::Result<()> {
    let db = engine.db.lock().unwrap();
    let message_id = entry
        .message_id
        .trim()
        .trim_matches(|c| c == ' ' || c == '<' || c == '>')
        .to_lowercase();
    // Where the email was when the AI saw it: the Inbox, or (rows kept since
    // other folders are sorted) the folder a server filter delivered it to.
    let from = if entry.from_folder.is_empty() {
        store::EXISTING_RULE_FOLDER
    } else {
        entry.from_folder.as_str()
    };
    let key = if message_id.is_empty() {
        format!("uid:{from}:{}", entry.uid)
    } else {
        message_id
    };
    let found = store::meron_entry_of_key(&db, &entry.account, &key)?;
    let unmark = match found {
        Some((entry_id, folder_id)) if folder_id == entry.meron_folder_id => {
            let unmark = crate::meron_marks::job_for_released_entry(&db, &entry_id);
            store::meron_release_entry(&db, &entry_id)?;
            unmark
        }
        _ => anyhow::bail!("It's no longer in {}.", entry.to_folder),
    };
    // Back "in the Inbox" for mail from another folder means shown there, on
    // this device; the server copy stays where it is.
    store::ai_sort_inbox_show_again(&db, &entry.account, from, entry.uid, entry.uidvalidity)?;
    store::ai_sort_set_log_state(&db, id, "moved_back", "")?;
    // The AI was wrong here: mail like this stays in the Inbox from now on.
    store::ai_lesson_from_log(&db, entry);
    drop(db);
    push_lesson_tags(engine);
    crate::meron_marks::spawn_writes(engine, unmark);
    report(engine, &entry.account, "INBOX");
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
