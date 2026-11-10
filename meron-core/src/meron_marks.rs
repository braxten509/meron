//! Meron-folder filing shared between devices through the mail server.
//!
//! Meron folders live on each device, with the same ids on devices set up
//! from one another ("Copy setup to another device", or a backup restore).
//! When a conversation is filed into one here (by AI sorting or by hand), its
//! Inbox copies on the server get the hidden keyword `$MeronF_<id>`; when it
//! is taken out (moved out, released by Archive/Delete/Move, the folder
//! deleted), the keyword goes; a move between two Meron folders swaps it.
//! Another device's sync reads the keyword and files the conversation the
//! same way, without asking the AI (see [`crate::store::meron_marks_observe`]).
//!
//! Only the copies in the real Inbox and in the account's archive mailbox
//! carry marks (the *mark folders*, [`mark_folders`]); of the Inbox's, only
//! mail within [`store::AI_SORT_MAX_AGE_DAYS`] gets one (archived mail and
//! the built-in Archive's marks have no age limit), and nothing else on the
//! server changes. Writes are best effort, like `$MeronSorted`: a mailbox that takes
//! no new keywords (no `\*` in PERMANENTFLAGS) is skipped silently. Applying
//! a mark read from the server never writes one back, so there is no loop.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};
use tokio::sync::Notify;

use crate::engine::Engine;
use crate::store;

/// What every mark keyword starts with; the rest is the folder id without its
/// `mf_` prefix.
pub const KEYWORD_PREFIX: &str = "$MeronF_";

/// How long writing marks may take.
const WRITE_WAIT: Duration = Duration::from_secs(5);

/// The keyword that names Meron folder `folder_id`, when its id makes a valid
/// IMAP atom (every id this app creates does).
pub fn keyword_for(folder_id: &str) -> Option<String> {
    let rest = folder_id.strip_prefix(store::MERON_FOLDER_ID_PREFIX)?;
    let valid = !rest.is_empty()
        && rest.len() <= 64
        && rest.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    valid.then(|| format!("{KEYWORD_PREFIX}{}", rest.to_ascii_lowercase()))
}

/// The Meron folder id a keyword names, if it is a mark.
pub fn folder_of_keyword(keyword: &str) -> Option<String> {
    let head = keyword.get(..KEYWORD_PREFIX.len())?;
    if !head.eq_ignore_ascii_case(KEYWORD_PREFIX) {
        return None;
    }
    let rest = keyword[KEYWORD_PREFIX.len()..].to_ascii_lowercase();
    let id = format!("{}{rest}", store::MERON_FOLDER_ID_PREFIX);
    keyword_for(&id).is_some().then_some(id)
}

/// One write of marks: `uids` of the account's mailbox `folder` get
/// `target`'s mark, or lose every mark when `target` is `None`.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkJob {
    pub account: String,
    pub folder: String,
    pub uids: Vec<u32>,
    pub target: Option<String>,
}

fn is_inbox(folder: &str) -> bool {
    folder.eq_ignore_ascii_case("INBOX")
}

/// The account's mailboxes whose copies carry marks: the Inbox and, when it
/// has one, its archive mailbox.
pub fn mark_folders(conn: &Connection, account: &str) -> Vec<String> {
    let mut out = vec!["INBOX".to_string()];
    if let Some(archive) = store::meron_archive_mailbox(conn, account) {
        out.push(archive);
    }
    out
}

/// One job per mailbox of `by_folder` (empty ones left out).
pub fn jobs_for(
    account: &str,
    by_folder: BTreeMap<String, Vec<u32>>,
    target: Option<&str>,
) -> Vec<MarkJob> {
    by_folder
        .into_iter()
        .filter(|(_, uids)| !uids.is_empty())
        .map(|(folder, uids)| MarkJob {
            account: account.to_string(),
            folder,
            uids,
            target: target.map(str::to_string),
        })
        .collect()
}

/// The copies among `rows` a mark for `target` goes on, per mark folder:
/// cached; in the Inbox within the age limit, unless `target` is the built-in
/// Archive. Archive-mailbox copies have no age limit.
pub fn uids_to_mark(
    conn: &Connection,
    account: &str,
    rows: &[(String, u32)],
    target: Option<&str>,
) -> BTreeMap<String, Vec<u32>> {
    let mut out: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    if store::is_local_account(account) {
        return out;
    }
    let archive = store::meron_archive_mailbox(conn, account);
    let builtin = target.is_some_and(store::meron_is_builtin);
    let now = store::now_unix();
    for (folder, uid) in rows {
        let in_archive = archive.as_deref() == Some(folder.as_str());
        if !is_inbox(folder) && !in_archive {
            continue;
        }
        if out.get(folder).is_some_and(|uids| uids.contains(uid)) {
            continue;
        }
        let date: Option<i64> = conn
            .query_row(
                "SELECT COALESCE(date, 0) FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                params![account, folder, uid],
                |row| row.get(0),
            )
            .ok();
        let Some(date) = date else {
            continue;
        };
        if in_archive || builtin || !store::ai_sort_too_old(date, now) {
            out.entry(folder.clone()).or_default().push(*uid);
        }
    }
    out
}

/// The mark-folder copies among `rows` this device knows to carry a mark, or
/// is still putting one on, per mailbox.
pub fn uids_marked(
    conn: &Connection,
    account: &str,
    rows: &[(String, u32)],
) -> BTreeMap<String, Vec<u32>> {
    let folders = mark_folders(conn, account);
    let is_mark_folder =
        |folder: &str| folders.iter().any(|known| known == folder) || is_inbox(folder);
    let mut by_folder: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for (folder, uid) in rows {
        if is_mark_folder(folder) {
            by_folder.entry(folder.clone()).or_default().push(*uid);
        }
    }
    let mut out: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for (folder, uids) in by_folder {
        let mut marked = store::meron_marked_uids(conn, account, &folder, &uids).unwrap_or_default();
        for uid in uids {
            if !marked.contains(&uid) && pending_mark(account, &folder, uid) {
                marked.push(uid);
            }
        }
        if !marked.is_empty() {
            out.insert(folder, marked);
        }
    }
    out
}

/// The `(folder, uid)` of an entry's cached messages.
pub fn entry_rows(conn: &Connection, entry_id: &str) -> (String, Vec<(String, u32)>) {
    let members = store::meron_entry_members(conn, entry_id).unwrap_or_default();
    let account = members.first().map(|m| m.account.clone()).unwrap_or_default();
    (account, members.into_iter().map(|m| (m.folder, m.uid)).collect())
}

/// The jobs that mark an entry's mark-folder copies with the folder it is in
/// now, one per mailbox.
pub fn job_for_filed_entry(conn: &Connection, entry_id: &str, folder_id: &str) -> Vec<MarkJob> {
    let (account, rows) = entry_rows(conn, entry_id);
    let by_folder = uids_to_mark(conn, &account, &rows, Some(folder_id));
    jobs_for(&account, by_folder, Some(folder_id))
}

/// The jobs that take the marks off an entry's mark-folder copies, one per
/// mailbox, read before the entry is released (its rows can't be found
/// after).
pub fn job_for_released_entry(conn: &Connection, entry_id: &str) -> Vec<MarkJob> {
    let (account, rows) = entry_rows(conn, entry_id);
    let by_folder = uids_marked(conn, &account, &rows);
    jobs_for(&account, by_folder, None)
}

/// Unmarks the store asked for while it held the database (a conversation in
/// the built-in Archive got Inbox mail again, see
/// [`crate::store::meron_adopt`]'s caller), written by the next
/// [`flush_archive_unmarks`].
static ARCHIVE_UNMARKS: Mutex<Vec<MarkJob>> = Mutex::new(Vec::new());

/// Remember an unmark to write once the store's transaction is over.
pub fn queue_archive_unmark(job: MarkJob) {
    if job.uids.is_empty() {
        return;
    }
    ARCHIVE_UNMARKS.lock().unwrap().push(job);
}

/// Write the queued Archive unmarks in the background. Cheap when none wait.
pub fn flush_archive_unmarks(engine: &Arc<Engine>) {
    let jobs = std::mem::take(&mut *ARCHIVE_UNMARKS.lock().unwrap());
    if !jobs.is_empty() {
        spawn_writes(engine, jobs);
    }
}

/// The marks each account mailbox's copies should carry, not written yet.
/// Writes go through one worker per account and mailbox, in order, and only the latest
/// wish for a copy is written: filing then quickly taking out never ends with
/// the filing's write landing last.
#[derive(Default)]
struct Pending {
    /// Wished for and not taken by the worker yet; a newer wish replaces one.
    want: BTreeMap<u32, Option<String>>,
    /// Being written by the worker now.
    writing: BTreeMap<u32, Option<String>>,
    /// A worker runs for the account's mailbox.
    running: bool,
}

/// Keyed by `(account, folder)`.
static PENDING: LazyLock<Mutex<HashMap<(String, String), Pending>>> = LazyLock::new(Default::default);
/// Woken each time a worker finishes a round of writes.
static WRITTEN: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Whether `uid` of the account's mailbox `folder` is due to carry a mark
/// this device hasn't finished writing.
fn pending_mark(account: &str, folder: &str, uid: u32) -> bool {
    let pending = PENDING.lock().unwrap();
    pending.get(&(account.to_string(), folder.to_string())).is_some_and(|state| {
        state
            .want
            .get(&uid)
            .or_else(|| state.writing.get(&uid))
            .is_some_and(|target| target.is_some())
    })
}

/// Whether any of `uids` of `folder` still waits to be written.
fn any_pending(account: &str, folder: &str, uids: &[u32]) -> bool {
    let pending = PENDING.lock().unwrap();
    pending.get(&(account.to_string(), folder.to_string())).is_some_and(|state| {
        uids.iter()
            .any(|uid| state.want.contains_key(uid) || state.writing.contains_key(uid))
    })
}

/// Record each job's wish and make sure the account's worker runs. On the
/// running runtime (desktop), else on the engine's (phone commands).
fn enqueue(engine: &Arc<Engine>, jobs: Vec<MarkJob>) {
    let mut start: Vec<(String, String)> = Vec::new();
    {
        let mut pending = PENDING.lock().unwrap();
        for job in jobs {
            if job.uids.is_empty() || job.folder.is_empty() || store::is_local_account(&job.account) {
                continue;
            }
            if job.target.as_deref().is_some_and(|id| keyword_for(id).is_none()) {
                continue;
            }
            let key = (job.account.clone(), job.folder.clone());
            let state = pending.entry(key.clone()).or_default();
            for uid in job.uids {
                state.want.insert(uid, job.target.clone());
            }
            if !state.running {
                state.running = true;
                start.push(key);
            }
        }
    }
    for account in start {
        let engine = engine.clone();
        let (worker_account, worker_folder) = account.clone();
        let work = async move { run_worker(&engine, &worker_account, &worker_folder).await };
        let started = match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(work);
                true
            }
            Err(_) => crate::ffi::spawn_engine_task(work),
        };
        // No runtime to write on: drop the wishes rather than wait forever.
        if !started {
            PENDING.lock().unwrap().remove(&account);
            WRITTEN.notify_waiters();
        }
    }
}

/// Write the wishes for the account's mailbox until none are left.
async fn run_worker(engine: &Arc<Engine>, account: &str, folder: &str) {
    let key = (account.to_string(), folder.to_string());
    loop {
        let round = {
            let mut pending = PENDING.lock().unwrap();
            let Some(state) = pending.get_mut(&key) else {
                return;
            };
            state.writing.clear();
            if state.want.is_empty() {
                pending.remove(&key);
                drop(pending);
                WRITTEN.notify_waiters();
                return;
            }
            state.writing = std::mem::take(&mut state.want);
            state.writing.clone()
        };
        let mut by_target: BTreeMap<Option<String>, Vec<u32>> = BTreeMap::new();
        for (uid, target) in round {
            by_target.entry(target).or_default().push(uid);
        }
        for (target, uids) in by_target {
            write_now(engine, account, folder, uids, target).await;
        }
        WRITTEN.notify_waiters();
    }
}

/// Write one job's marks and wait (a few seconds at most) until they are on
/// the server, or failed. Best effort.
pub async fn write(engine: &Arc<Engine>, job: MarkJob) {
    let account = job.account.clone();
    let folder = job.folder.clone();
    let uids = job.uids.clone();
    enqueue(engine, vec![job]);
    let _ = tokio::time::timeout(WRITE_WAIT + Duration::from_secs(1), async {
        loop {
            let woken = WRITTEN.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            if !any_pending(&account, &folder, &uids) {
                return;
            }
            woken.await;
        }
    })
    .await;
}

/// [`write`] for several jobs at once (one per mailbox, say): all are queued,
/// then waited for together, a few seconds at most.
///
/// Inside a bulk run this is the marks step every member shares (see
/// [`crate::engine::bulk`]): the marks of all of them are written together.
pub async fn write_all(engine: &Arc<Engine>, jobs: Vec<MarkJob>) {
    if jobs.is_empty() {
        return;
    }
    match crate::engine::bulk::seat() {
        Some(seat) => seat.marks(jobs).await,
        None => write_all_alone(engine, jobs).await,
    }
}

/// [`write_all`] as one round of writes of its own.
pub(crate) async fn write_all_alone(engine: &Arc<Engine>, jobs: Vec<MarkJob>) {
    let waits: Vec<(String, String, Vec<u32>)> = jobs
        .iter()
        .map(|job| (job.account.clone(), job.folder.clone(), job.uids.clone()))
        .collect();
    if waits.is_empty() {
        return;
    }
    enqueue(engine, jobs);
    let _ = tokio::time::timeout(WRITE_WAIT + Duration::from_secs(1), async {
        loop {
            let woken = WRITTEN.notified();
            tokio::pin!(woken);
            woken.as_mut().enable();
            if !waits
                .iter()
                .any(|(account, folder, uids)| any_pending(account, folder, uids))
            {
                return;
            }
            woken.await;
        }
    })
    .await;
}

/// Write marks on the server now. Logged; what the server took is remembered
/// so this device's own next read doesn't act on it.
async fn write_now(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: Vec<u32>,
    target: Option<String>,
) {
    let account = account.to_string();
    let folder = folder.to_string();
    let add = target.as_deref().and_then(keyword_for);
    if target.is_some() && add.is_none() {
        return;
    }
    // Every mark this device could have left: those of its Meron folders, and
    // those it remembers on these copies (a folder deleted since).
    let (remove, remembers) = {
        let db = engine.db.lock().unwrap();
        let mut ids: Vec<String> = store::list_meron_folders(&db)
            .unwrap_or_default()
            .into_iter()
            .map(|folder| folder.id)
            .collect();
        for uid in &uids {
            if let Ok(Some(id)) = db
                .query_row(
                    "SELECT folder_id FROM meron_marks WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                    params![account, folder, uid],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                && !id.is_empty()
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
        let remove: Vec<String> = ids.iter().filter_map(|id| keyword_for(id)).collect();
        // Only mark folders' marks are remembered: a copy marked in another
        // folder (just before an Archive moves it) is read in its new one.
        let remembers = mark_folders(&db, &account).iter().any(|known| known == &folder);
        (remove, remembers)
    };
    let written = tokio::time::timeout(
        WRITE_WAIT,
        engine.with_write_session(&account, |session| {
            let uids = uids.clone();
            let remove = remove.clone();
            let add = add.clone();
            let folder = folder.clone();
            Box::pin(async move {
                crate::imap::store_meron_marks(session, &folder, &uids, &remove, add.as_deref()).await
            })
        }),
    )
    .await;
    match written {
        Ok(Ok(true)) if remembers => {
            let db = engine.db.lock().unwrap();
            if let Err(err) = store::meron_marks_written(
                &db,
                &account,
                &folder,
                &uids,
                target.as_deref(),
                store::meron_mark_tick(),
            ) {
                warn(format_args!("account={account}: could not remember marks: {err:#}"));
            }
        }
        Ok(Ok(true)) => {}
        // The mailbox keeps no keywords of its own: filing stays on this device.
        Ok(Ok(false)) => {}
        Ok(Err(err)) => warn(format_args!("account={account}: Meron folder mark failed: {err:#}")),
        Err(_) => warn(format_args!("account={account}: Meron folder mark timed out")),
    }
}

/// Write each job in the background, in order with every other mark write of
/// its account (the latest wish for a copy wins).
pub fn spawn_writes(engine: &Arc<Engine>, jobs: Vec<MarkJob>) {
    enqueue(engine, jobs);
}

fn warn(what: std::fmt::Arguments<'_>) {
    crate::mlog!(crate::log::Level::Warn, "mail.meronMarks", "{what}");
}
