//! Snapshot in the mail synchronization engine.
//!
//! Coordinates the corresponding IMAP and store operations during folder updates.
//! The parent module exposes these operations to foreground and background sync.

use super::*;

/// Reconnect to IMAP, fetch the most recent `limit` messages of a folder into
/// the store, resetting cached messages if the server's UIDVALIDITY changed.
pub struct SyncMessagesResult {
    /// Messages in the synced window.
    pub count: usize,
    /// Headers read in full this sync: the whole window on a full sync, only
    /// the window's messages the cache lacked on an incremental one.
    pub messages: Vec<imap::MessageHeader>,
    pub arrivals: Vec<imap::MessageHeader>,
    /// New mail AI sorting filed into Meron folders with notifications on,
    /// to announce under those folders.
    pub meron_arrivals: Vec<ai_sort::MeronArrivals>,
}

/// `(uid, seen, starred, tags, Meron folder mark)`, as in
/// [`imap::FlagSync::changes`].
type FlagState = (u32, bool, bool, Vec<String>, Option<Option<String>>);

/// What the network phase of a folder sync read.
pub(in crate::engine) struct FolderSyncState {
    pub(in crate::engine) batch: imap::RecentBatch,
    pub(in crate::engine) flag_sync: Option<imap::FlagSync>,
    pub(in crate::engine) server_uids: Option<HashSet<u32>>,
    pub(in crate::engine) server_unseen: Option<HashSet<u32>>,
    /// Set by an incremental sync: the window's UIDs, newest first. Then
    /// `batch.messages` holds only the window's messages the cache lacked;
    /// the rest are cached and only their flags can have changed.
    pub(in crate::engine) window: Option<Vec<u32>>,
    /// Incremental sync whose CONDSTORE changes were not complete: the FLAGS
    /// of the window's cached messages, read on their own.
    pub(in crate::engine) window_flags: Vec<FlagState>,
}

/// The flag and UID state a folder sync reads after its window.
pub(super) struct FlagReads {
    pub(super) flag_sync: Option<imap::FlagSync>,
    pub(super) server_uids: Option<HashSet<u32>>,
    pub(super) server_unseen: Option<HashSet<u32>>,
    pub(super) window_flags: Vec<FlagState>,
}

/// Read the folder's flag changes, UIDs and unseen UIDs after its window.
///
/// `flags_for` is set for an incremental sync: the window's cached messages,
/// whose flags no full FETCH read. When CONDSTORE reports every change since
/// the stored modseq they cost nothing; otherwise their FLAGS are read here,
/// and a failure to read them fails the call, since the sync would otherwise
/// miss flag changes a full read would have seen. `listed_uids` is a UID
/// listing already taken after the window's UIDNEXT, reused for pruning.
#[allow(clippy::too_many_arguments)]
pub(super) async fn read_flag_state(
    session: &mut imap::Session,
    folder: &str,
    uidvalidity: u32,
    flags_for: Option<Vec<u32>>,
    listed_uids: Option<HashSet<u32>>,
    prior_modseq: u64,
    prior_validity: u32,
    needs_tag_baseline: bool,
) -> anyhow::Result<FlagReads> {
    // Reconcile \Seen across the whole folder (catches reads on other
    // devices, even for messages older than the recent window).
    // Best-effort: a no-op when there's no baseline, UIDVALIDITY changed,
    // or the server lacks CONDSTORE.
    let validity_matches = prior_validity != 0 && prior_validity == uidvalidity;
    let mut flag_sync = imap::sync_flags(session, folder, prior_modseq, validity_matches)
        .await
        .ok();
    if needs_tag_baseline
        && validity_matches
        && let Some(flag_sync) = flag_sync.as_mut()
    {
        flag_sync.tag_baseline = imap::fetch_all_tags(session).await.ok();
    }
    let window_flags = match flags_for {
        Some(uids) if !flag_sync.as_ref().is_some_and(|sync| sync.complete) => {
            imap::fetch_flags_by_uid(session, folder, &uids).await?
        }
        _ => Vec::new(),
    };
    // Server-side UID set so we can drop locally cached messages another
    // client moved or deleted. Best-effort: a failure here skips the prune.
    let server_uids = match listed_uids {
        Some(uids) => Some(uids),
        None => imap::list_all_uids(session, folder).await.ok(),
    };
    // Unread mail older than the window still counts toward the
    // folder's unread total. Best-effort like the prune above. The whole
    // set is kept: Meron folders count (and mark read) the members among
    // it. The Inbox's own count leaves these out, as it counts only mail it
    // lists (see `store::inbox_listed_sql` and `folders::unread_sql`).
    let server_unseen = imap::list_unseen_uids(session, folder).await.ok();
    Ok(FlagReads {
        flag_sync,
        server_uids,
        server_unseen,
        window_flags,
    })
}

/// The window's UIDs no full FETCH in `batch` read.
pub(super) fn cached_in_window(window: &[u32], batch: &imap::RecentBatch) -> Vec<u32> {
    let fetched: HashSet<u32> = batch.messages.iter().map(|message| message.uid).collect();
    window
        .iter()
        .copied()
        .filter(|uid| !fetched.contains(uid))
        .collect()
}

/// Rebuild what the batched sync would have produced, for a folder holding at
/// least one message whose FETCH response cannot be parsed. Establishes the
/// window as UIDs, reads in full what needs it (only the messages missing from
/// `known` when the folder was synced before, else the whole window, see
/// [`recover_recent_batch`]) a narrower range at a time, then re-runs the
/// best-effort flag reconciliation and UID listing that the failed attempt
/// never reached.
#[allow(clippy::too_many_arguments)]
pub(in crate::engine) async fn sync_state_isolating_unparseable(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
    prior_modseq: u64,
    prior_validity: u32,
    known: Option<&HashSet<u32>>,
    needs_tag_baseline: bool,
) -> anyhow::Result<FolderSyncState> {
    let listing = match known {
        Some(_) if prior_validity != 0 => Some(
            engine
                .with_read_session(account, |session| {
                    let folder = folder.to_string();
                    Box::pin(async move { imap::folder_listing(session, &folder).await })
                })
                .await?,
        )
        .filter(|listing| listing.uidvalidity == prior_validity && listing.consistent()),
        _ => None,
    };
    let (batch, window, listed_uids) = match (listing, known) {
        (Some(listing), Some(known)) => {
            let window = listing.window(limit);
            let missing: Vec<u32> = window
                .iter()
                .copied()
                .filter(|uid| !known.contains(uid))
                .collect();
            let messages = read_isolating(engine, account, folder, missing).await?;
            let batch = imap::RecentBatch {
                uidvalidity: listing.uidvalidity,
                uid_next: listing.uid_next,
                messages,
            };
            (batch, Some(window), Some(listing.uids))
        }
        _ => (
            recover_recent_batch(engine, account, folder, limit).await?,
            None,
            None,
        ),
    };
    let uidvalidity = batch.uidvalidity;
    let flags_for = window
        .as_deref()
        .map(|window| cached_in_window(window, &batch));
    let reads = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            let flags_for = flags_for.clone();
            let listed_uids = listed_uids.clone();
            Box::pin(async move {
                read_flag_state(
                    session,
                    &folder,
                    uidvalidity,
                    flags_for,
                    listed_uids,
                    prior_modseq,
                    prior_validity,
                    needs_tag_baseline,
                )
                .await
            })
        })
        .await;
    let reads = match reads {
        Ok(reads) => reads,
        // An incremental read fetched no flags for the cached messages, so
        // without this read their read/star state and deletions would go
        // stale while the sync reported success.
        Err(err) if window.is_some() => return Err(err),
        Err(err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "account={account} folder={folder}: flag and UID reads skipped: {err:#}"
            );
            FlagReads {
                flag_sync: None,
                server_uids: None,
                server_unseen: None,
                window_flags: Vec::new(),
            }
        }
    };
    Ok(FolderSyncState {
        batch,
        flag_sync: reads.flag_sync,
        server_uids: reads.server_uids,
        server_unseen: reads.server_unseen,
        window,
        window_flags: reads.window_flags,
    })
}
