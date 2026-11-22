//! Turns groups of candidate messages into folder decisions.
//! Sender lessons and bulk-mail signals supply context before the provider runs.
//! One representative can stand for a conversation when the decisions are expanded.

use super::*;

pub(super) fn sender_of(candidate: &AiSortCandidate) -> String {
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
            if let Err(error) = push_pending_tags(&engine, &account).await {
                warn(format_args!(
                    "account={account}: lesson tag push failed: {error:#}"
                ));
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

/// Reader lessons and spam decisions available for one provider batch.
///
/// Lessons follow a sender across accounts. A destination must be available to
/// appear in the prompt. A lesson for an unavailable destination still contributes
/// to deciding which lesson is most recent.
pub(super) struct Taught {
    pub(super) earlier: Vec<Vec<ai::lessons::PromptLesson>>,
    pub(super) backed: Vec<bool>,
    pub(super) spam: Option<ai::SpamChoice>,
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
            Err(error) => {
                warn(format_args!("lessons not read: {error:#}"));
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
pub(super) async fn decide_batch(
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
pub(super) fn representatives(groups: &[Vec<AiSortCandidate>]) -> Vec<AiSortCandidate> {
    groups
        .iter()
        .filter_map(|group| group.first().cloned())
        .collect()
}

/// Give each group's answer to every email of the group.
///
/// Except Spam (`spam`: the account's Spam folder), the careful answer: it was
/// backed by a lesson about the asked email's sender, so it only stands for
/// the group's emails from that same sender. Another person's email in the
/// same conversation stays in the Inbox.
pub(super) fn expand_decisions(
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
pub(super) fn spam_folder(engine: &Engine, account: &str) -> Option<String> {
    store::folder_for_role(&engine.db.lock().unwrap(), account, "junk")
        .ok()
        .flatten()
}

/// Group `candidates` by conversation (see
/// [`store::ai_sort_group_conversations`]); on a store error each email is its
/// own conversation.
pub(super) fn group_conversations(
    engine: &Engine,
    account: &str,
    folder: &str,
    candidates: Vec<AiSortCandidate>,
) -> Vec<Vec<AiSortCandidate>> {
    let fallback: Vec<Vec<AiSortCandidate>> = candidates
        .iter()
        .cloned()
        .map(|candidate| vec![candidate])
        .collect();
    store::ai_sort_group_conversations(&engine.db.lock().unwrap(), account, folder, candidates)
        .unwrap_or_else(|error| {
            warn(format_args!(
                "account={account}: grouping by conversation: {error:#}"
            ));
            fallback
        })
}
