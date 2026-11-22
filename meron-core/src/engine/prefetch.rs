//! Downloads recent message bodies for offline reading.
//! Both hosts start the worker after a folder sync and share the session pool with the reader.
//! The worker tracks completed UIDs and schedules attachment metadata backfill separately.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::{imap, parse, store};

use super::*;

/// How far back the body prefetcher reaches: everything received within this
/// many days, read or not.
pub const PREFETCH_DAYS: u32 = 14;

/// Messages downloaded per IMAP session checkout: one `UID FETCH`. The session
/// goes back to the pool between batches so opening a message can reuse it
/// instead of waiting out the rest of the backlog or paying for a new TLS
/// handshake.
const PREFETCH_BATCH: usize = imap::BODY_FETCH_BATCH;

/// Downloads that may fail in a row before a run stops to check whether
/// anything can be downloaded at all.
const PREFETCH_MAX_FAILURES: usize = 3;

/// How long a prefetch download may go without receiving bytes.
const PREFETCH_IDLE: Duration = Duration::from_secs(120);

#[derive(Clone, Debug)]
pub struct BodyPrefetchOptions {
    pub days: u32,
    pub max_count: Option<usize>,
    pub media_root: PathBuf,
}

impl Default for BodyPrefetchOptions {
    fn default() -> Self {
        Self {
            days: PREFETCH_DAYS,
            max_count: None,
            media_root: parse::media_root(),
        }
    }
}

pub(super) fn limit_prefetch_uids(mut pending: Vec<u32>, max_count: Option<usize>) -> Vec<u32> {
    // SEARCH returns ascending UIDs. The reader opens the newest mail first, so
    // warm that end before older messages. A cap (mobile) then keeps only it.
    pending.reverse();
    if let Some(max_count) = max_count {
        pending.truncate(max_count);
    }
    pending
}

/// Cache recent messages and attachments for offline reading.
///
/// Fetches the newest missing bodies first and returns the IMAP connection between batches.
/// Each saved body survives an interruption. Opening an uncached message still uses the on-demand reader.
pub async fn prefetch_bodies(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
) -> anyhow::Result<usize> {
    prefetch_bodies_with_options(engine, account, folder, BodyPrefetchOptions::default()).await
}

pub async fn prefetch_bodies_with_options(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    options: BodyPrefetchOptions,
) -> anyhow::Result<usize> {
    let _snapshot_guard = MessageSyncGuard::begin(engine)?;
    let uids = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            let days = options.days;
            Box::pin(async move { imap::search_prefetch_uids(session, &folder, days).await })
        })
        .await?;
    let wanted: Vec<u32> = {
        let db = engine.db.lock().unwrap();
        uids.into_iter()
            .filter(|uid| {
                store::has_message(&db, account, folder, *uid).unwrap_or(false)
                    && !store::has_cached_body(&db, account, folder, *uid).unwrap_or(false)
            })
            .collect()
    };
    let pending: Vec<u32> = {
        let wanted_set: HashSet<u32> = wanted.iter().copied().collect();
        // Only mail that is still waiting stays on the list: not once the
        // on-demand reader cached it, it left the window, or it was deleted.
        let mut skipped = engine.prefetch_skipped.lock().unwrap();
        let ours = folder_key_prefix(account, folder);
        skipped.retain(|key| {
            key.strip_prefix(ours.as_str())
                .and_then(|uid| uid.parse::<u32>().ok())
                .is_none_or(|uid| wanted_set.contains(&uid))
        });
        wanted
            .into_iter()
            .filter(|uid| !skipped.contains(&message_key(account, folder, *uid)))
            .collect()
    };
    let pending = limit_prefetch_uids(pending, options.max_count);
    let run = PrefetchRun {
        engine,
        account,
        folder,
        media_root: &options.media_root,
    };

    // Newest first. A batch that fails hands what it did not deliver to
    // `singles`, to be taken one message at a time ahead of the next batch.
    let mut queue: VecDeque<u32> = pending.into();
    let mut singles: VecDeque<u32> = VecDeque::new();
    let mut blame = Blame::default();
    let mut fetched = 0usize;
    let mut failures = 0usize;
    let mut probed = false;
    loop {
        if failures >= PREFETCH_MAX_FAILURES {
            // Nothing but failures: an expired login, a folder that is gone or
            // a dead network — or the newest few messages all being ones that
            // cannot be downloaded. One message from the far end of the backlog
            // tells them apart, once. If it arrives the run goes on.
            let Some(probe) = queue.pop_back().filter(|_| !probed) else {
                break;
            };
            probed = true;
            let (saved, result) = run.download(&[probe]).await;
            fetched += saved.len();
            if result.is_err() {
                break;
            }
            failures = 0;
            fetched += run.after_success(&mut blame).await;
            continue;
        }
        if let Some(uid) = singles.pop_front() {
            let (saved, result) = run.download(&[uid]).await;
            fetched += saved.len();
            match result {
                Ok(()) => {
                    failures = 0;
                    fetched += run.after_success(&mut blame).await;
                }
                Err(error) => {
                    eprintln!("meron-core: prefetch {folder} uid {uid}: {error:#}");
                    failures += 1;
                    if prefetch_failure_is_the_messages(&error) {
                        blame.suspects.push(uid);
                    }
                }
            }
            continue;
        }
        let chunk: Vec<u32> = queue.drain(..queue.len().min(PREFETCH_BATCH)).collect();
        if chunk.is_empty() {
            break;
        }
        let (saved, result) = run.download(&chunk).await;
        fetched += saved.len();
        let Err(error) = result else {
            failures = 0;
            fetched += run.after_success(&mut blame).await;
            continue;
        };
        eprintln!("meron-core: prefetch {folder}: {error:#}");
        // Part of a batch arriving means the account works, but the link has
        // just failed: not the moment to judge anything by.
        if saved.is_empty() {
            failures += 1;
        } else {
            failures = 0;
        }
        if chunk.len() > 1 {
            // What arrived before the failure is kept. The order never changes,
            // so one message that cannot be downloaded would fail the same
            // batch on every run and keep everything older cold.
            singles.extend(chunk.iter().filter(|uid| !saved.contains(uid)));
        } else if prefetch_failure_is_the_messages(&error) {
            blame.suspects.push(chunk[0]);
        }
    }
    Ok(fetched)
}

/// What a run holds against messages, short of skipping them.
///
/// A failure on its own proves nothing: a stalled network fails every message
/// it touches. A message is only skipped once it has failed, failed again
/// straight after a clean download, and a clean download has followed that —
/// working mail on both sides of its second failure.
#[derive(Default)]
struct Blame {
    /// Failed once on their own, for a reason that may be theirs.
    suspects: Vec<u32>,
    /// Failed again when retried after a clean download.
    failed_twice: Vec<u32>,
}

/// Whether a failed download may be the message's own doing; only asked of a message downloaded on
/// its own. A connection that went away is not. An error the server reported, or a download that
/// went silent or outlasted its allowance, may be — or may be the account or the network, which is
/// why it only makes the message a suspect (see [`PrefetchRun::after_success`]).
pub(super) fn prefetch_failure_is_the_messages(error: &anyhow::Error) -> bool {
    error.is::<TransferTimedOut>() || !background_sync::is_transient_sync_error(error)
}

struct PrefetchRun<'a> {
    engine: &'a Arc<Engine>,
    account: &'a str,
    folder: &'a str,
    media_root: &'a std::path::Path,
}

impl PrefetchRun<'_> {
    /// Download and cache `uids` on one session checkout. Returns the UIDs
    /// saved, which on a failure are the ones that arrived before it.
    async fn download(&self, uids: &[u32]) -> (Vec<u32>, anyhow::Result<()>) {
        prefetch_batch(
            self.engine,
            self.account,
            self.folder,
            uids,
            self.media_root,
        )
        .await
    }

    /// Called straight after a download completed cleanly, the one moment
    /// the account, the folder and the network are known to be fine. Returns
    /// how many suspects arrived after all.
    async fn after_success(&self, blame: &mut Blame) -> usize {
        // Their second failure now has a clean download after it as well.
        if !blame.failed_twice.is_empty() {
            let mut skipped = self.engine.prefetch_skipped.lock().unwrap();
            for uid in blame.failed_twice.drain(..) {
                skipped.insert(message_key(self.account, self.folder, uid));
            }
        }
        let mut arrived = 0;
        for uid in std::mem::take(&mut blame.suspects) {
            let (saved, result) = self.download(&[uid]).await;
            arrived += saved.len();
            if let Err(error) = result
                && prefetch_failure_is_the_messages(&error)
            {
                blame.failed_twice.push(uid);
            }
        }
        arrived
    }
}

/// Download and cache `uids` on one session checkout. Returns the UIDs saved,
/// which on a failure are the ones that arrived before it.
async fn prefetch_batch(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: &[u32],
    media_root: &std::path::Path,
) -> (Vec<u32>, anyhow::Result<()>) {
    let wanted = BTreeMap::from([(folder.to_string(), uids.to_vec())]);
    let fetched = fetch_bodies_keeping_partial(
        engine,
        account,
        &wanted,
        media_root,
        TransferLimits {
            idle: PREFETCH_IDLE,
            total: Duration::from_secs(600),
        },
    )
    .await;
    let saved = fetched.messages.iter().map(|(_, uid, _)| *uid).collect();
    (saved, fetched.result)
}

/// How long a body download may run.
#[derive(Clone, Copy)]
pub(crate) struct TransferLimits {
    /// The longest the transfer may go without receiving bytes. Connecting
    /// is not on this clock; a large message can keep making progress.
    pub idle: Duration,
    /// The longest the call may take altogether, connecting included, however
    /// steadily mail arrives.
    pub total: Duration,
}

impl TransferLimits {
    /// One allowance for the whole download: for a caller that is waiting.
    pub fn within(total: Duration) -> Self {
        Self { idle: total, total }
    }
}

/// What [`fetch_bodies_keeping_partial`] came back with.
pub(crate) struct PartialBodies {
    /// `(folder, uid, message)` for everything that arrived, also when the
    /// download then failed or ran out of time.
    pub messages: Vec<(String, u32, parse::Message)>,
    /// The folders a download was actually started for. A message asked for in
    /// one of these and not in `messages` was not delivered; one in any other
    /// folder was never tried.
    pub attempted: HashSet<String>,
    pub result: anyhow::Result<()>,
}

#[derive(Default)]
struct TransferProgress {
    messages: Vec<(String, u32, parse::Message)>,
    attempted: HashSet<String>,
}

// Clear the idle clock whenever an operation ends, including errors and
// cancellation. Reconnection and credential refresh are not transfer stalls.
struct ActiveTransfer(imap::ReadProgress);

impl Drop for ActiveTransfer {
    fn drop(&mut self) {
        *self.0.lock().unwrap() = None;
    }
}

/// Download `wanted` (UIDs by folder) on one session checkout within `limits`,
/// keeping whatever arrives. A failure or a deadline part-way through keeps
/// what came before it, so a retry only has the rest to fetch.
pub(crate) async fn fetch_bodies_keeping_partial(
    engine: &Engine,
    account: &str,
    wanted: &BTreeMap<String, Vec<u32>>,
    media_root: &std::path::Path,
    limits: TransferLimits,
) -> PartialBodies {
    // Outside the session future: that is dropped when time runs out.
    let progress: Arc<std::sync::Mutex<TransferProgress>> = Arc::default();
    let clock: imap::ReadProgress = Arc::default();
    let transfer = engine.with_transfer_session(account, limits.total, |session| {
        let clock = clock.clone();
        session.get_mut().observe_reads(clock.clone());
        let progress = progress.clone();
        let account = account.to_string();
        let media_root = media_root.to_path_buf();
        let db = engine.db.clone();
        // A second run, after a lost connection, only wants what is left.
        let remaining: Vec<(String, Vec<u32>)> = {
            let progress = progress.lock().unwrap();
            wanted
                .iter()
                .map(|(folder, uids)| {
                    let left =
                        uids.iter()
                            .copied()
                            .filter(|uid| {
                                !progress.messages.iter().any(|(have_folder, have, _)| {
                                    have_folder == folder && have == uid
                                })
                            })
                            .collect::<Vec<u32>>();
                    (folder.clone(), left)
                })
                .filter(|(_, left)| !left.is_empty())
                .collect()
        };
        Box::pin(async move {
            let _active = ActiveTransfer(clock.clone());
            *clock.lock().unwrap() = Some(tokio::time::Instant::now());
            for (folder, uids) in remaining {
                progress.lock().unwrap().attempted.insert(folder.clone());
                // peek via fetch_bodies: warming must not flip unread mail to read.
                imap::fetch_bodies(
                    session,
                    &folder,
                    &uids,
                    media_root.clone(),
                    &account,
                    &mut |uid, message| {
                        // Persist before yielding to the socket again: an outer
                        // caller timeout can cancel the entire batch at any point.
                        let _ = store::save_cached_message(
                            &db.lock().unwrap(),
                            &account,
                            &folder,
                            uid,
                            &message,
                        );
                        let mut progress = progress.lock().unwrap();
                        progress.messages.push((folder.clone(), uid, message));
                    },
                )
                .await?;
            }
            Ok(())
        })
    });
    tokio::pin!(transfer);
    let result = loop {
        let last_progress = *clock.lock().unwrap();
        // Not transferring yet: nothing to time, look again shortly.
        let wake = match last_progress {
            Some(at) => at + limits.idle,
            None => tokio::time::Instant::now() + limits.idle.min(Duration::from_secs(1)),
        };
        tokio::select! {
            result = &mut transfer => break result,
            // Bytes may have arrived while this slept: look again.
            () = tokio::time::sleep_until(wake) => {
                let stalled = clock
                    .lock()
                    .unwrap()
                    .is_some_and(|at| at + limits.idle <= tokio::time::Instant::now());
                if stalled {
                    break Err(TransferTimedOut(limits.idle).into());
                }
            }
        }
    };
    let mut progress = progress.lock().unwrap();
    PartialBodies {
        messages: std::mem::take(&mut progress.messages),
        attempted: std::mem::take(&mut progress.attempted),
        result,
    }
}

/// Fetch and cache the bodies of specific UIDs, skipping any already cached.
///
/// Unlike [`prefetch_bodies`] this takes the UIDs from the caller instead of a
/// server-side SEARCH, because the caller (new-mail notification) already knows
/// exactly which messages it needs and awaits the result — a SEARCH round trip
/// per arrival would add latency to the notification for nothing.
pub async fn fetch_bodies_for_uids(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: &[u32],
    media_root: PathBuf,
) -> anyhow::Result<usize> {
    let pending: Vec<u32> = {
        let db = engine.db.lock().unwrap();
        uids.iter()
            .copied()
            .filter(|uid| !store::has_cached_body(&db, account, folder, *uid).unwrap_or(false))
            .collect()
    };
    if pending.is_empty() {
        return Ok(0);
    }
    let _snapshot_guard = MessageSyncGuard::begin(engine)?;
    let wanted = BTreeMap::from([(folder.to_string(), pending)]);
    let fetched = fetch_bodies_keeping_partial(
        engine,
        account,
        &wanted,
        &media_root,
        TransferLimits {
            idle: PREFETCH_IDLE,
            total: Duration::from_secs(600),
        },
    )
    .await;
    fetched.result.map(|()| fetched.messages.len())
}

/// Warm a folder's bodies in the background (deduped per account/folder). Stays
/// silent: this is an optimization, so failures go to stderr rather than
/// surfacing as UI error toasts. The stored data it fills is observed lazily when the
/// user opens a message.
pub fn spawn_body_prefetch(engine: Arc<Engine>, account: String, folder: String) {
    if engine.is_paused(&account) {
        return;
    }
    let key = format!("body:{account}/{folder}");
    if !engine.syncing.lock().unwrap().insert(key.clone()) {
        return;
    }
    tokio::spawn(async move {
        let result = tokio::time::timeout(
            Duration::from_secs(600),
            prefetch_bodies(&engine, &account, &folder),
        )
        .await;
        engine.syncing.lock().unwrap().remove(&key);
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("meron-core: prefetch {folder}: {e:#}"),
            // Partial progress persisted; the next trigger resumes the rest.
            Err(_) => eprintln!("meron-core: prefetch {folder}: timed out (will resume)"),
        }
    });
}

/// Rows per [`store::backfill_files_batch`] call: small enough that the store
/// is never held long enough to stall a UI request queued behind it.
pub const FILES_BACKFILL_BATCH: u32 = 500;

/// Fill `messages.files` for mail cached before the column existed, one short
/// batch at a time off the async runtime, releasing the store between batches.
/// A no-op once finished; an interrupted run resumes on the next start.
pub fn spawn_files_backfill(engine: Arc<Engine>) {
    tokio::task::spawn_blocking(move || {
        loop {
            let result = {
                let db = crate::log::timed_db_lock(&engine.db, "files_backfill");
                store::backfill_files_batch(&db, FILES_BACKFILL_BATCH)
            };
            match result {
                Ok(true) => return,
                Ok(false) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => {
                    eprintln!("meron-core: attachment backfill: {e:#}");
                    return;
                }
            }
        }
    });
}

#[cfg(test)]
pub(crate) mod transfer_clock_tests;
