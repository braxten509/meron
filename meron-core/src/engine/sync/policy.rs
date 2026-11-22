//! Policy in the mail synchronization engine.
//!
//! Coordinates the corresponding IMAP and store operations during folder updates.
//! The parent module exposes these operations to foreground and background sync.

use super::*;

/// How long a sync waits on the server work its rules queued.
pub(super) const RULE_ACTIONS_BUDGET: Duration = Duration::from_secs(60);

pub(in crate::engine) const DEFAULT_BACKGROUND_SYNC_TIMEOUT_SECS: u64 = 30;
pub(in crate::engine) const MAX_BACKGROUND_SYNC_TIMEOUT_SECS: u64 = 24 * 60 * 60;

pub(in crate::engine) fn parse_background_sync_timeout(value: Option<&str>) -> Duration {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|&secs| (1..=MAX_BACKGROUND_SYNC_TIMEOUT_SECS).contains(&secs))
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_BACKGROUND_SYNC_TIMEOUT_SECS))
}

/// Per-folder sync budget, shared by background syncs and each piggybacked companion sync so one
/// slow mailbox can't hold the post-sync tail (and the emit that follows it) indefinitely. The 30s
/// default suits direct IMAP servers; slow gateways (e.g. DavMail bridging Exchange EWS) can need
/// more, so it can be overridden with `MERON_SYNC_TIMEOUT` (seconds, up to one day).
pub fn background_sync_timeout() -> Duration {
    static TIMEOUT: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        let value = std::env::var("MERON_SYNC_TIMEOUT").ok();
        parse_background_sync_timeout(value.as_deref())
    })
}

/// The companion mailboxes (Sent, then Drafts) to piggyback onto a sync of another folder, each
/// paired with its role label for the caller's logs. Callers resolve the names via
/// `cached_sent_folder`/`cached_drafts_folder`, which already exclude the folder being synced; this
/// only guards against the two roles resolving to the same mailbox.
pub(in crate::engine) fn companion_folders(
    sent: Option<String>,
    drafts: Option<String>,
) -> Vec<(&'static str, String)> {
    let mut companions = Vec::new();
    if let Some(sent) = sent {
        companions.push(("Sent", sent));
    }
    if let Some(drafts) = drafts
        && !companions
            .iter()
            .any(|(_, existing)| existing.eq_ignore_ascii_case(&drafts))
    {
        companions.push(("Drafts", drafts));
    }
    companions
}

/// Outcome of one companion-folder sync from [`sync_companion_folders`]:
/// `role` is "Sent" or "Drafts", `folder` the resolved mailbox name.
pub struct CompanionSync {
    pub role: &'static str,
    pub folder: String,
    pub result: anyhow::Result<SyncMessagesResult>,
}

/// Piggyback Sent and Drafts envelope syncs onto a completed sync of `folder`,
/// so messages sent or drafted from another client surface in the cross-folder
/// conversation view straight from the local store — the per-folder recent
/// sync only ever covers the open folder, and thread-gap filling only fetches
/// referenced *ancestors*, so nothing else ever pulls in a reply another
/// client added to Sent. Shared by desktop and mobile so the two can't drift.
/// Failures are returned per folder, never propagated: a companion hiccup must
/// not fail the primary sync that already succeeded.
pub async fn sync_companion_folders(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> Vec<CompanionSync> {
    let sent = cached_sent_folder(engine, account, folder);
    let drafts = cached_drafts_folder(engine, account, folder);
    let mut outcomes = Vec::new();
    let companions = companion_folders(sent, drafts);
    let archive = archive_companion(engine, account, folder, &companions);
    for (role, companion) in companions {
        let result = sync_background_messages(engine, account, &companion, limit).await;
        outcomes.push(CompanionSync {
            role,
            folder: companion,
            result,
        });
    }
    // The archive mailbox too, now and then: another device's archives reach
    // the built-in Archive Meron folder through the marks it reads there.
    if let Some(archive) = archive {
        let result = sync_background_messages(engine, account, &archive, limit).await;
        outcomes.push(CompanionSync {
            role: "Archive",
            folder: archive,
            result,
        });
    }
    outcomes
}

/// How often a companion sync of the archive mailbox runs, per account.
const ARCHIVE_COMPANION_EVERY: Duration = Duration::from_secs(5 * 60);

/// When each account's archive mailbox last had its companion sync.
static ARCHIVE_COMPANION_AT: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, std::time::Instant>>,
> = std::sync::LazyLock::new(Default::default);

/// The account's archive mailbox when its companion sync is due: not the
/// folder just synced (or another companion), not for local or feed
/// accounts, at most once per [`ARCHIVE_COMPANION_EVERY`].
fn archive_companion(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    companions: &[(&'static str, String)],
) -> Option<String> {
    if store::is_local_account(account) {
        return None;
    }
    let archive = store::meron_archive_mailbox(&engine.db.lock().unwrap(), account)?;
    if archive.eq_ignore_ascii_case(folder)
        || companions
            .iter()
            .any(|(_, companion)| companion.eq_ignore_ascii_case(&archive))
    {
        return None;
    }
    let mut last = ARCHIVE_COMPANION_AT.lock().unwrap();
    let now = std::time::Instant::now();
    if last
        .get(account)
        .is_some_and(|at| now.duration_since(*at) < ARCHIVE_COMPANION_EVERY)
    {
        return None;
    }
    last.insert(account.to_string(), now);
    Some(archive)
}

/// Most archive-mailbox copies [`fetch_uncached_marked`] reads in one batch.
const UNCACHED_MARKED_CAP: usize = 200;

/// Archive-mailbox copies whose mark names a Meron folder here but which the sync didn't cache:
/// read in batches of [`UNCACHED_MARKED_CAP`] (newest first), stored, and their marks acted on.
/// Answers whether every one was read; when not, the sync keeps its old CONDSTORE position, so the
/// next one reports the rest again. Logged.
pub(super) async fn fetch_uncached_marked(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    mut uids: Vec<u32>,
) -> bool {
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.dedup();
    let mut all_read = true;
    for batch in uids.chunks(UNCACHED_MARKED_CAP) {
        if !fetch_uncached_marked_batch(engine, account, folder, batch).await {
            all_read = false;
            break;
        }
    }
    crate::meron_marks::flush_archive_unmarks(engine);
    all_read
}

/// One batch of [`fetch_uncached_marked`]; false when it couldn't be read or
/// stored.
async fn fetch_uncached_marked_batch(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> bool {
    let read_at = store::meron_mark_tick();
    let fetched = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            let uids = uids.to_vec();
            Box::pin(async move { imap::fetch_headers_by_uid(session, &folder, &uids).await })
        })
        .await;
    let mut headers = match fetched {
        Ok(headers) => headers,
        Err(err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.meronMarks",
                "account={account}: reading archived marked mail skipped: {err:#}"
            );
            return false;
        }
    };
    for header in &mut headers {
        header.folder = folder.to_string();
    }
    let db = engine.db.lock().unwrap();
    if let Err(err) = store::upsert_read_messages(&db, account, folder, &headers) {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.meronMarks",
            "account={account}: storing archived marked mail skipped: {err:#}"
        );
        return false;
    }
    let seen: Vec<store::MeronMarkSeen> = headers
        .iter()
        .filter(|header| header.server_tags.is_some())
        .filter_map(|header| {
            header.meron_mark.clone().map(|mark| store::MeronMarkSeen {
                uid: header.uid,
                mark,
            })
        })
        .collect();
    match store::meron_marks_observe(&db, account, folder, &seen, read_at, store::now_unix()) {
        // A mark this device wrote since the read began is read again later.
        Ok(applied) => !applied.skipped_stale,
        Err(err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.meronMarks",
                "account={account}: archived marks skipped: {err:#}"
            );
            false
        }
    }
}
