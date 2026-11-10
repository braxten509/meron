//! Folder and message sync against the server, including companion folders.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use crate::{imap, store};

use super::*;

/// Reconnect to IMAP, list folders, and refresh the store.
pub async fn sync_folders(
    engine: &Arc<Engine>,
    account: &str,
) -> anyhow::Result<Vec<imap::Folder>> {
    let folders = engine
        .with_read_session(account, |session| {
            Box::pin(async move { imap::list_folders(session).await })
        })
        .await?;
    crate::mlog!(
        crate::log::Level::Debug,
        "mail.sync",
        "folders account={account}: {} listed [{}]",
        folders.len(),
        folders
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let db = engine.db.lock().unwrap();
    store::upsert_folders(&db, account, &folders)?;
    Ok(folders)
}

/// Update every physical copy selected by a star action. Commit each mailbox to
/// the cache only after its server write succeeds, including on partial failure.
pub async fn mark_starred_copies(
    engine: &Engine,
    account: &str,
    targets: &std::collections::BTreeMap<String, Vec<u32>>,
    starred: bool,
) -> anyhow::Result<()> {
    for (folder, uids) in targets {
        engine
            .with_flag_write_session(
                account,
                |session| {
                    let folder = folder.clone();
                    Box::pin(async move { imap::prepare_flag_update(session, &folder).await })
                },
                |session| {
                    let uids = uids.clone();
                    Box::pin(async move { imap::store_starred(session, &uids, starred).await })
                },
            )
            .await?;
        let db = engine.db.lock().unwrap();
        for uid in uids {
            store::update_message_starred(&db, account, folder, *uid, starred)?;
        }
    }
    Ok(())
}

/// Bring the server up to date with pending tag changes. Each write commits on
/// its own, so a failure leaves only that write pending for the next try.
pub async fn push_pending_tags(engine: &Engine, account: &str) -> anyhow::Result<()> {
    let turn = engine
        .tag_pushes
        .lock()
        .unwrap()
        .entry(account.to_string())
        .or_default()
        .clone();
    let _turn = turn.lock().await;

    let refused_key = |folder: &str, tag: &str| format!("{account}\n{folder}\n{tag}");
    let pushes = {
        let db = engine.db.lock().unwrap();
        if store::account_paused(&db, account)? {
            return Ok(());
        }
        store::pending_tag_pushes(&db, account)?
    };
    let mut refused = std::collections::HashSet::new();
    for push in &pushes {
        let key = refused_key(&push.folder, &push.tag);
        if push.add && engine.refused_tags.lock().unwrap().contains(&key) {
            refused.insert((push.folder.clone(), push.tag.clone()));
            continue;
        }
        let writable = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stored = engine
            .with_flag_write_session(
                account,
                |session| {
                    let writable = writable.clone();
                    let push = push.clone();
                    Box::pin(async move {
                        let can =
                            imap::prepare_tag_update(session, &push.folder, &push.tag).await?;
                        writable.store(can, std::sync::atomic::Ordering::Relaxed);
                        anyhow::Ok(())
                    })
                },
                |session| {
                    let writable = writable.load(std::sync::atomic::Ordering::Relaxed);
                    let push = push.clone();
                    Box::pin(async move {
                        // Only a new keyword can be refused. Taking one away is
                        // always asked for: it may be there either way.
                        if push.add && !writable {
                            return anyhow::Ok(false);
                        }
                        imap::store_tag(session, &push.uids, &push.tag, push.add).await?;
                        anyhow::Ok(true)
                    })
                },
            )
            .await;
        match stored {
            Ok(true) => store::record_tag_push(&engine.db.lock().unwrap(), account, push)?,
            Ok(false) => {
                engine.refused_tags.lock().unwrap().insert(key);
                refused.insert((push.folder.clone(), push.tag.clone()));
            }
            Err(err) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.tags",
                    "account={account} folder={}: tag push deferred: {err:#}",
                    push.folder
                );
            }
        }
    }
    store::settle_pending_tags(&engine.db.lock().unwrap(), account, &refused)
}

/// [`push_pending_tags`] without waiting for it: a tag is already saved locally,
/// and a slow or absent server must not hold up the action or the sync that
/// asked. Whatever fails stays pending for the next sync to retry.
pub fn spawn_tag_push(engine: &Arc<Engine>, account: &str) {
    let engine = engine.clone();
    let account = account.to_string();
    tokio::spawn(async move {
        if let Err(err) = push_pending_tags(&engine, &account).await {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.tags",
                "account={account}: tag push failed: {err:#}"
            );
        }
    });
}

/// Where [`delete_to_trash`] moved messages, and the Trash folder's UIDNEXT
/// from just before the move (see [`refresh_moved_copies`]).
pub struct Trashed {
    pub folder: String,
    pub uid_next_before: Option<u32>,
}

/// Delete messages on the server. Returns `None` for a permanent expunge
/// (Drafts, or items already in Trash), or `Some` when moved to Trash.
pub async fn delete_to_trash(
    engine: &Engine,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> anyhow::Result<Option<Trashed>> {
    // Drafts and Trash from the saved folder list. Asking the server is a
    // whole LIST each, and a delete used to pay for two of them; only a role
    // the saved list cannot name (a new account, a server without one) still
    // asks.
    let saved_role = |special_use: &str, looks_like: fn(&str) -> bool| {
        let folders = store::get_folders(&engine.db.lock().unwrap(), account).unwrap_or_default();
        super::folders::find_role_folder(folders, special_use, looks_like, "")
    };
    let saved_drafts = saved_role("drafts", imap::looks_like_drafts);
    let saved_trash = saved_role("trash", imap::looks_like_trash);
    engine
        .with_write_session(account, |session| {
            let folder = folder.to_string();
            let uids = uids.to_vec();
            let saved_drafts = saved_drafts.clone();
            let saved_trash = saved_trash.clone();
            Box::pin(async move {
                let drafts = match saved_drafts {
                    Some(drafts) => Some(drafts),
                    None => imap::find_drafts_folder(session).await?,
                };
                if drafts.as_deref() == Some(folder.as_str()) {
                    imap::expunge_uids(session, &folder, &uids).await?;
                    return anyhow::Ok(None);
                }
                let trash = match saved_trash {
                    Some(trash) => trash,
                    None => imap::find_trash_folder(session)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("Trash folder not found for this account"))?,
                };
                if trash == folder {
                    imap::expunge_uids(session, &folder, &uids).await?;
                    return anyhow::Ok(None);
                }
                let uid_next_before = imap::move_to_folder(session, &folder, &trash, &uids).await?;
                anyhow::Ok(Some(Trashed {
                    folder: trash,
                    uid_next_before,
                }))
            })
        })
        .await
}

/// The target folder's recent window, re-read after a move into it, and the
/// UIDs of the copies that move created there.
pub struct MovedCopies {
    /// `None` when the re-read failed; the move itself still stands.
    pub batch: Option<imap::RecentBatch>,
    pub uids: Vec<u32>,
    _snapshot_guard: Option<MessageSyncGuard>,
}

impl MovedCopies {
    /// Cache the re-read target window, if there is one.
    pub fn store(
        &self,
        conn: &rusqlite::Connection,
        account: &str,
        target: &str,
    ) -> anyhow::Result<()> {
        let Some(batch) = &self.batch else {
            return Ok(());
        };
        store::ensure_folder(conn, account, target)?;
        store::upsert_window_messages(conn, account, target, batch)?;
        store::set_folder_state(conn, account, target, batch.uidvalidity, batch.uid_next)?;
        Ok(())
    }
}

/// Move `uids` to `target` on the server and cache where they went.
pub async fn move_within_account(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    target: &str,
    uids: &[u32],
) -> anyhow::Result<MovedCopies> {
    move_within_account_with(engine, account, folder, target, uids, true).await
}

/// [`move_within_account`], with the re-read of `target`'s recent window put
/// off when `reread_window` is false (see [`refresh_moved_copies_with`]).
pub async fn move_within_account_with(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    target: &str,
    uids: &[u32],
    reread_window: bool,
) -> anyhow::Result<MovedCopies> {
    let cached_ids = store::cached_message_ids(&engine.db.lock().unwrap(), account, folder, uids)?;
    let moved_ids = moved_message_ids(engine, account, folder, uids, cached_ids).await;
    let uid_next_before = engine
        .with_write_session(account, |session| {
            let folder = folder.to_string();
            let target = target.to_string();
            let uids = uids.to_vec();
            Box::pin(async move { imap::move_to_folder(session, &folder, &target, &uids).await })
        })
        .await?;
    let copies = refresh_moved_copies_with(
        engine,
        account,
        target,
        uids.len(),
        uid_next_before,
        moved_ids.as_deref(),
        reread_window,
    )
    .await;
    let db = engine.db.lock().unwrap();
    copies.store(&db, account, target)?;
    store::delete_messages_by_uid(&db, account, folder, uids)?;
    drop(db);
    Ok(copies)
}

/// The lowercased Message-ID of each message about to be moved, `""` for one
/// without, so [`refresh_moved_copies`] can tell its copies from other mail in
/// the target. `cached` comes from `store::cached_message_ids`; what the cache
/// lacks is read from the server. `None` when a message can't be described
/// there either — then no copies are picked.
///
/// Read-only, on its own session, and run before the move: best-effort, since
/// failing here costs only the exact undo, never the move.
pub async fn moved_message_ids(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: &[u32],
    cached: Vec<Option<String>>,
) -> Option<Vec<String>> {
    let missing: Vec<u32> = uids
        .iter()
        .zip(&cached)
        .filter(|(_, id)| id.is_none())
        .map(|(uid, _)| *uid)
        .collect();
    let fetched: HashMap<u32, String> = if missing.is_empty() {
        HashMap::new()
    } else {
        match fetch_headers_isolating_unparseable(engine, account, folder, missing).await {
            Ok((headers, _)) => headers
                .into_iter()
                .map(|header| (header.uid, header.message_id.trim().to_lowercase()))
                .collect(),
            Err(err) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={folder}: reading messages before move failed: {err:#}"
                );
                return None;
            }
        }
    };
    uids.iter()
        .zip(cached)
        .map(|(uid, id)| id.or_else(|| fetched.get(uid).cloned()))
        .collect()
}

/// What deleting a sender row removes: `cached` (their cached messages in
/// `folder`, see `store::sender_folder_uids`) plus older messages of theirs in
/// `folder` that only the server holds. The server is asked with two SEARCHes,
/// FROM the address and TO it, because the row's person follows the "other
/// person" rule (`store::sender_person`): mail From one of my own addresses,
/// or any mail in a Sent-like mailbox, belongs to its first recipient. Each
/// uncached hit is read back and kept only when that rule names exactly
/// `addr` — SEARCH matches substrings, and `bob@x.com` must not take
/// `jimbob@x.com` with it. Sent copies elsewhere are never touched: the search
/// covers `folder` alone.
///
/// All or nothing: when the server cannot tell which messages are theirs (a
/// failed search, or a hit that could not be read back), this fails and
/// nothing is deleted, rather than deleting only part of the row.
pub async fn sender_delete_uids(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    addr: &str,
    cached: Vec<u32>,
) -> anyhow::Result<Vec<u32>> {
    let addr = addr.trim().to_lowercase();
    if addr.is_empty() || store::is_local_account(account) {
        return Ok(cached);
    }
    let found = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            let addr = addr.clone();
            Box::pin(async move {
                let mut found =
                    imap::search_uids(session, &folder, &format!("from:{addr}")).await?;
                found.extend(imap::search_uids(session, &folder, &format!("to:{addr}")).await?);
                Ok(found)
            })
        })
        .await
        .map_err(|err| {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "account={account} folder={folder}: sender search before delete failed: {err:#}"
            );
            err.context("searching the server for the sender's mail")
        })?;
    // A hit the cache already holds is either in `cached` or someone else's.
    let (uncached, mine) = {
        let db = engine.db.lock().unwrap();
        let held = store::cached_uids_in_folder(&db, account, folder, &found)?;
        let mut uncached: Vec<u32> = found
            .into_iter()
            .filter(|uid| !held.contains(uid))
            .collect();
        uncached.sort_unstable();
        uncached.dedup();
        (uncached, store::self_addrs(&db, account))
    };
    if uncached.is_empty() {
        return Ok(cached);
    }
    let (headers, skipped) =
        fetch_headers_isolating_unparseable(engine, account, folder, uncached.clone())
            .await
            .map_err(|err| {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={folder}: reading sender search hits failed: {err:#}"
                );
                err.context("reading the sender's mail from the server")
            })?;
    let read: std::collections::HashSet<u32> = headers.iter().map(|header| header.uid).collect();
    let missing = uncached.iter().filter(|uid| !read.contains(uid)).count();
    if !skipped.is_empty() || missing > 0 {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.sync",
            "account={account} folder={folder}: {missing} sender search hits could not be read, \
             not deleting"
        );
        anyhow::bail!("could not read {missing} of the sender's messages from the server");
    }
    let mut uids = cached;
    for header in headers {
        if store::sender_person(account, &mine, folder, &header) == addr
            && !uids.contains(&header.uid)
        {
            uids.push(header.uid);
        }
    }
    Ok(uids)
}

/// The UIDs of the copies a move created, among the target's messages at or
/// above its pre-move UIDNEXT (`arrivals`). `moved` holds the lowercased
/// Message-ID of each moved message, `""` for one without.
///
/// Arrivals whose Message-ID the move didn't carry are other mail delivered
/// meanwhile, and are passed over. Each Message-ID the move did carry — the
/// empty one included — must account for exactly as many arrivals as moved
/// messages: fewer means a copy is missing, more means a concurrent delivery
/// is indistinguishable from it. Either way the answer would be wrong, so none
/// is given, and no Undo is offered.
pub(super) fn moved_copy_uids(arrivals: &[imap::MessageHeader], moved: &[String]) -> Vec<u32> {
    let mut expected: HashMap<&str, usize> = HashMap::new();
    for id in moved {
        *expected.entry(id.as_str()).or_default() += 1;
    }
    let mut found: HashMap<&str, Vec<u32>> = HashMap::new();
    for arrival in arrivals {
        let id = arrival.message_id.trim().to_lowercase();
        if let Some((&key, _)) = expected.get_key_value(id.as_str()) {
            found.entry(key).or_default().push(arrival.uid);
        }
    }
    let mut uids = Vec::with_capacity(moved.len());
    for (id, count) in expected {
        match found.remove(id) {
            Some(copies) if copies.len() == count => uids.extend(copies),
            _ => return Vec::new(),
        }
    }
    uids.sort_unstable();
    uids
}

/// Re-read `target` after moving `count` messages into it, and pick out the
/// copies the move created (see [`moved_copy_uids`]): every message at or
/// above the target's UIDNEXT from before the move is read, not just the
/// recent window, so a large move can't lose its earliest copies. Undo moves
/// exactly these back, rather than resolving the thread key in the target —
/// which would also take older mail of the same conversation, and finds
/// nothing for a `uid:` key, as UIDs change on a move.
///
/// `moved` is `None` when the moved messages couldn't all be described, and
/// then no copies are picked, as when the pre-move UIDNEXT is unknown.
///
/// Read-only and best-effort, on its own sessions: the MOVE has landed and
/// must not be retried or reported failed because a re-read went wrong.
pub async fn refresh_moved_copies(
    engine: &Arc<Engine>,
    account: &str,
    target: &str,
    count: usize,
    uid_next_before: Option<u32>,
    moved: Option<&[String]>,
) -> MovedCopies {
    refresh_moved_copies_with(engine, account, target, count, uid_next_before, moved, true).await
}

/// Messages moved into a folder while its re-read was put off: how many the
/// next re-read of that (account, folder) still has to cover.
static OWED_REREADS: std::sync::LazyLock<std::sync::Mutex<HashMap<(String, String), u32>>> =
    std::sync::LazyLock::new(Default::default);

/// Upper bound on one catch-up re-read, however much a long run put off.
const MAX_OWED_REREAD: u32 = 1000;

fn take_owed_reread(account: &str, target: &str) -> u32 {
    OWED_REREADS
        .lock()
        .unwrap()
        .remove(&(account.to_string(), target.to_string()))
        .unwrap_or(0)
}

fn owe_reread(account: &str, target: &str, count: u32) {
    let mut owed = OWED_REREADS.lock().unwrap();
    let entry = owed.entry((account.to_string(), target.to_string())).or_insert(0);
    *entry = entry.saturating_add(count).min(MAX_OWED_REREAD);
}

/// [`refresh_moved_copies`], optionally without re-reading `target`'s recent
/// window. A bulk delete or move handles one conversation per call; re-reading
/// the same folder after each of them is the heaviest step and only the last
/// one's result matters. With `reread_window` false the window is left alone
/// and the moved count is remembered, so the next re-read of that folder (the
/// run's last call, or any later move into it) reaches back far enough to
/// cover everything that was skipped. Locating the moved copies is unchanged.
pub async fn refresh_moved_copies_with(
    engine: &Arc<Engine>,
    account: &str,
    target: &str,
    count: usize,
    uid_next_before: Option<u32>,
    moved: Option<&[String]>,
    reread_window: bool,
) -> MovedCopies {
    let guard = match MessageSyncGuard::begin(engine) {
        Ok(guard) => guard,
        Err(err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "cannot guard moved-copy refresh: {err:#}"
            );
            return MovedCopies {
                batch: None,
                uids: Vec::new(),
                _snapshot_guard: None,
            };
        }
    };
    let count = count as u32;
    let batch = if reread_window {
        let owed = take_owed_reread(account, target);
        match fetch_recent_resilient(engine, account, target, 50.max(count.saturating_add(owed)))
            .await
        {
            Ok(batch) => Some(batch),
            Err(err) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={target}: refresh after move failed: {err:#}"
                );
                // Still not read: the next re-read has to cover these too.
                owe_reread(account, target, count.saturating_add(owed));
                None
            }
        }
    } else {
        owe_reread(account, target, count);
        None
    };
    let uids = match (uid_next_before.filter(|uid_next| *uid_next > 0), moved) {
        (Some(floor), Some(moved)) => match moved_arrivals(engine, account, target, floor).await {
            Ok(arrivals) => moved_copy_uids(&arrivals, moved),
            Err(err) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={target}: locating moved copies failed: {err:#}"
                );
                Vec::new()
            }
        },
        _ => Vec::new(),
    };
    MovedCopies {
        batch,
        uids,
        _snapshot_guard: Some(guard),
    }
}

/// Every message in `target` at or above `floor`. Fails rather than return a
/// partial set, which [`moved_copy_uids`] would take for a missing copy.
async fn moved_arrivals(
    engine: &Arc<Engine>,
    account: &str,
    target: &str,
    floor: u32,
) -> anyhow::Result<Vec<imap::MessageHeader>> {
    let uids = engine
        .with_read_session(account, |session| {
            let target = target.to_string();
            Box::pin(async move { imap::uids_from(session, &target, floor).await })
        })
        .await?;
    let (headers, skipped) =
        fetch_headers_isolating_unparseable(engine, account, target, uids.clone()).await?;
    anyhow::ensure!(
        skipped.is_empty() && headers.len() == uids.len(),
        "read {} of {} messages",
        headers.len(),
        uids.len()
    );
    Ok(headers)
}

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
pub(super) struct FolderSyncState {
    pub(super) batch: imap::RecentBatch,
    pub(super) flag_sync: Option<imap::FlagSync>,
    pub(super) server_uids: Option<HashSet<u32>>,
    pub(super) server_unseen: Option<HashSet<u32>>,
    /// Set by an incremental sync: the window's UIDs, newest first. Then
    /// `batch.messages` holds only the window's messages the cache lacked;
    /// the rest are cached and only their flags can have changed.
    pub(super) window: Option<Vec<u32>>,
    /// Incremental sync whose CONDSTORE changes were not complete: the FLAGS
    /// of the window's cached messages, read on their own.
    pub(super) window_flags: Vec<FlagState>,
}

/// The flag and UID state a folder sync reads after its window.
struct FlagReads {
    flag_sync: Option<imap::FlagSync>,
    server_uids: Option<HashSet<u32>>,
    server_unseen: Option<HashSet<u32>>,
    window_flags: Vec<FlagState>,
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
async fn read_flag_state(
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
fn cached_in_window(window: &[u32], batch: &imap::RecentBatch) -> Vec<u32> {
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
pub(super) async fn sync_state_isolating_unparseable(
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

/// [`imap::fetch_recent`] for one folder, recovering from messages whose FETCH
/// response cannot be parsed instead of failing the whole batch. Prefer this to
/// calling `imap::fetch_recent` inside a session closure.
///
/// Read-only, so it must not share a session with a mutating command: a wedged
/// connection has to be discarded, and a mutation that already reached the
/// server must never be retried. Run the mutation first, then call this.
pub async fn fetch_recent_resilient(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> anyhow::Result<imap::RecentBatch> {
    let attempt = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            Box::pin(async move { imap::fetch_recent(session, &folder, limit).await })
        })
        .await;
    match attempt {
        Ok(batch) => Ok(batch),
        Err(err) if imap::is_unparseable_response(&err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "account={account} folder={folder}: unparseable FETCH response, \
                 re-reading the window message by message: {err:#}"
            );
            recover_recent_batch(engine, account, folder, limit).await
        }
        Err(err) => Err(err),
    }
}

/// Re-read the recent window of `folder` while working around messages whose
/// FETCH response cannot be parsed. Establishes the window as UIDs (a
/// `UID SEARCH` reply is immune to the failure), then reads it in ranges that
/// exclude the offending messages.
pub(super) async fn recover_recent_batch(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> anyhow::Result<imap::RecentBatch> {
    let (uidvalidity, uid_next, uids) = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            Box::pin(async move { imap::recent_uids(session, &folder, limit).await })
        })
        .await?;
    let messages = read_isolating(engine, account, folder, uids).await?;
    Ok(imap::RecentBatch {
        uidvalidity,
        uid_next,
        messages,
    })
}

/// [`fetch_headers_isolating_unparseable`], logging what it had to skip.
async fn read_isolating(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: Vec<u32>,
) -> anyhow::Result<Vec<imap::MessageHeader>> {
    let (messages, skipped) =
        fetch_headers_isolating_unparseable(engine, account, folder, uids).await?;
    if !skipped.is_empty() {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.sync",
            "account={account} folder={folder}: {} of {} messages skipped as unreadable: {skipped:?}",
            skipped.len(),
            messages.len() + skipped.len()
        );
    }
    Ok(messages)
}

/// Ceiling on FETCH attempts one poison-message recovery may spend. A run with
/// a single bad message costs about `2 * log2(window)` attempts; this only
/// bites when a folder is riddled with them, where the right answer is to store
/// what we have rather than reconnect dozens of times.
pub(super) const ISOLATION_ATTEMPT_LIMIT: usize = 32;

/// Fetch headers for `uids`, working around messages whose FETCH response the
/// IMAP parser rejects (see [`imap::is_unparseable_response`]).
///
/// Such a response wedges the connection it arrived on, so it cannot be skipped
/// mid-stream — the session has to go and the range has to be re-fetched
/// without it. This halves any failing range until the offending message sits
/// alone, drops just that one, and keeps its neighbours. Every attempt runs
/// through `with_read_session`, which discards the failed session and hands the
/// retry a fresh connection.
///
/// Returns the headers it could read plus the UIDs it gave up on. A genuine
/// network or server error aborts, since retrying narrower ranges would not
/// help.
pub(super) async fn fetch_headers_isolating_unparseable(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: Vec<u32>,
) -> anyhow::Result<(Vec<imap::MessageHeader>, Vec<u32>)> {
    fetch_headers_isolating_until(engine, account, folder, uids, None).await
}

/// A catch-up deadline returns completed headers and isolated poison UIDs;
/// cancelled or unread chunks remain eligible for the next sync.
async fn fetch_headers_isolating_until(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: Vec<u32>,
    deadline: Option<tokio::time::Instant>,
) -> anyhow::Result<(Vec<imap::MessageHeader>, Vec<u32>)> {
    let mut pending = vec![uids];
    let mut out: Vec<imap::MessageHeader> = Vec::new();
    let mut skipped: Vec<u32> = Vec::new();
    let mut attempts = 0usize;

    while let Some(chunk) = pending.pop() {
        if chunk.is_empty() {
            continue;
        }
        if attempts >= ISOLATION_ATTEMPT_LIMIT {
            let abandoned: usize = pending.iter().map(Vec::len).sum::<usize>() + chunk.len();
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "account={account} folder={folder}: giving up isolating unparseable \
                 messages after {attempts} fetches, {abandoned} UIDs left unread"
            );
            break;
        }
        attempts += 1;
        let operation = engine.with_read_session(account, |session| {
            let folder = folder.to_string();
            let chunk = chunk.clone();
            Box::pin(async move { imap::fetch_headers_by_uid(session, &folder, &chunk).await })
        });
        let result = if let Some(deadline) = deadline {
            match tokio::time::timeout_at(deadline, operation).await {
                Ok(result) => result,
                Err(_) => break,
            }
        } else {
            operation.await
        };
        match result {
            Ok(headers) => out.extend(headers),
            Err(err) if !imap::is_unparseable_response(&err) => return Err(err),
            Err(err) if chunk.len() == 1 => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={folder}: skipping uid={}, its FETCH \
                     response could not be parsed: {err:#}",
                    chunk[0]
                );
                skipped.push(chunk[0]);
            }
            Err(_) => {
                // Push the tail first so the halves are attempted in UID order.
                let mid = chunk.len() / 2;
                pending.push(chunk[mid..].to_vec());
                pending.push(chunk[..mid].to_vec());
            }
        }
    }

    out.sort_unstable_by_key(|header| std::cmp::Reverse(header.uid));
    Ok((out, skipped))
}

pub async fn sync_messages(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> anyhow::Result<SyncMessagesResult> {
    sync_messages_with_policy(
        engine,
        account,
        folder,
        limit,
        false,
        ATTACHMENT_CATCHUP_BUDGET,
    )
    .await
}

/// Automatic desktop refresh: bounded network work, respecting pause state.
/// IDLE recovery and explicit mobile refresh retain their original policy.
pub async fn sync_background_messages(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> anyhow::Result<SyncMessagesResult> {
    sync_messages_with_policy(
        engine,
        account,
        folder,
        limit,
        true,
        ATTACHMENT_CATCHUP_BUDGET,
    )
    .await
}

/// A cache refresh after a successful server write. Keep the guard until the
/// returned batch has been persisted, so cleanup cannot release stale UIDs.
pub struct WrittenFolderRefresh {
    pub batch: imap::RecentBatch,
    _guard: MessageSyncGuard,
}

/// Cache warming must not turn a completed APPEND/save into a failed write.
/// Registration and network failures skip the refresh; callers retain success.
pub async fn refresh_written_folder(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> Option<WrittenFolderRefresh> {
    let result = async {
        let guard = MessageSyncGuard::begin(engine)?;
        let batch = fetch_recent_resilient(engine, account, folder, limit).await?;
        anyhow::Ok(WrittenFolderRefresh {
            batch,
            _guard: guard,
        })
    }
    .await;
    match result {
        Ok(refresh) => Some(refresh),
        Err(err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "account={account} folder={folder}: post-write refresh skipped: {err:#}"
            );
            None
        }
    }
}

// Keep registration alive through persistence, including when the caller drops
// its future after spawn_blocking has started. Drop also covers network errors
// and cancellation without leaving markers protected by a dead snapshot.
pub struct MessageSyncGuard {
    engine: Arc<Engine>,
    id: i64,
}

impl MessageSyncGuard {
    pub fn begin(engine: &Arc<Engine>) -> anyhow::Result<Self> {
        let db = engine.db.lock().unwrap();
        let (id, _) = store::begin_message_sync(&db)?;
        Ok(Self {
            engine: engine.clone(),
            id,
        })
    }
}

impl Drop for MessageSyncGuard {
    fn drop(&mut self) {
        let db = self.engine.db.lock().unwrap();
        if let Err(err) = store::end_message_sync(&db, self.id) {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "removal marker cleanup failed: {err:#}"
            );
        }
    }
}

const ATTACHMENT_CATCHUP_BUDGET: Duration = Duration::from_secs(5);

async fn sync_messages_with_policy(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
    background: bool,
    catchup_budget: Duration,
) -> anyhow::Result<SyncMessagesResult> {
    // When this sync's reads began: a Meron-folder mark this device wrote
    // after it may not show in them (see `store::meron_marks_observe`).
    let read_at = store::meron_mark_tick();
    // Read the prior sync position before any network I/O so we can ask the
    // server for only the flag changes since then (CONDSTORE CHANGEDSINCE).
    // Callers must await this operation rather than timing out the whole sync;
    // only the network phase below is cancellable by the sync budget.
    let prepare_engine = engine.clone();
    let prepare_account = account.to_string();
    let prepare_folder = folder.to_string();
    let (
        prior_modseq,
        prior_validity,
        removed_epoch,
        removal_guard,
        missing_files,
        needs_tag_baseline,
        known,
    ) = tokio::task::spawn_blocking(move || {
        let engine = prepare_engine;
        let account = prepare_account.as_str();
        let folder = prepare_folder.as_str();
        let db = crate::log::timed_db_lock(&engine.db, "sync_messages.prepare/backfill");
        if background && store::account_paused(&db, account)? {
            return Err(anyhow::Error::new(BackgroundSyncCancelled));
        }
        let modseq = store::get_folder_modseq(&db, account, folder)?;
        let validity = store::get_folder_state(&db, account, folder)?
            .map(|(v, _)| v)
            .unwrap_or(0);
        // Removals after this point (an archive that lands while the fetch is
        // still on the wire) get a higher epoch. This snapshot must not write
        // those UIDs back.
        let missing_files = store::missing_attachment_metadata_uids(&db, account, folder, limit)?;
        let (id, removed_epoch) = store::begin_message_sync(&db)?;
        let guard = MessageSyncGuard {
            engine: engine.clone(),
            id,
        };
        let tag_baseline = store::needs_tag_baseline(&db, account, folder)?;
        // A folder synced before reads only what changed: the UIDs it already
        // caches are left out of the full FETCH.
        let known = if validity != 0 {
            Some(Arc::new(store::header_synced_uids(&db, account, folder)?))
        } else {
            None
        };
        anyhow::Ok((
            modseq,
            validity,
            removed_epoch,
            guard,
            missing_files,
            tag_baseline,
            known,
        ))
    })
    .await??;

    let fetch = || async {
        let attempt = engine
            .with_read_session(account, |session| {
                let folder = folder.to_string();
                let known = known.clone();
                Box::pin(async move {
                    // Folder synced before: read in full only the window's
                    // messages the cache lacks. Otherwise (first sync, new
                    // UIDVALIDITY, a listing that contradicts its SELECT) the
                    // whole window, as always.
                    let incremental = match known.as_deref() {
                        Some(known) => {
                            imap::fetch_recent_incremental(
                                session,
                                &folder,
                                limit,
                                prior_validity,
                                known,
                            )
                            .await?
                        }
                        None => None,
                    };
                    let (batch, window, listed_uids) = match incremental {
                        Some(read) => (read.batch, Some(read.window), Some(read.server_uids)),
                        None => (imap::fetch_recent(session, &folder, limit).await?, None, None),
                    };
                    let flags_for = window
                        .as_deref()
                        .map(|window| cached_in_window(window, &batch));
                    let reads = read_flag_state(
                        session,
                        &folder,
                        batch.uidvalidity,
                        flags_for,
                        listed_uids,
                        prior_modseq,
                        prior_validity,
                        needs_tag_baseline,
                    )
                    .await?;
                    anyhow::Ok(FolderSyncState {
                        batch,
                        flag_sync: reads.flag_sync,
                        server_uids: reads.server_uids,
                        server_unseen: reads.server_unseen,
                        window,
                        window_flags: reads.window_flags,
                    })
                })
            })
            .await;
        // A message whose FETCH response we cannot parse takes the whole batch down
        // with it, and does so again on every later sync. Re-read the window a
        // narrower range at a time so the rest of the folder still syncs.
        let state = match attempt {
            Ok(state) => Ok(state),
            Err(err) if imap::is_unparseable_response(&err) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={folder}: unparseable FETCH response, \
                 re-reading the window message by message: {err:#}"
                );
                sync_state_isolating_unparseable(
                    engine,
                    account,
                    folder,
                    limit,
                    prior_modseq,
                    prior_validity,
                    known.as_deref(),
                    needs_tag_baseline,
                )
                .await
            }
            Err(err) => Err(err),
        }?;
        Ok(state)
    };
    let network_started = std::time::Instant::now();
    let FolderSyncState {
        batch,
        flag_sync,
        server_uids,
        server_unseen,
        window,
        window_flags,
    } = if background {
        retry_background_sync(&format!("sync {folder} for {account}"), || true, fetch).await?
    } else {
        fetch().await?
    };
    let network_ms = network_started.elapsed().as_millis();
    // What the network phase read: messages in full, and FLAGS-only reads of
    // cached ones. An unchanged folder synced before reads 0 and 0.
    let fetched_full = batch.messages.len();
    let fetched_flags = window_flags.len();
    let mode = if window.is_some() { "incremental" } else { "full" };
    crate::mlog!(
        crate::log::Level::Info,
        "sync.fetch",
        "account={account} folder={folder:?} mode={mode} window={} fetched_full={fetched_full} \
         fetched_flags={fetched_flags} condstore_changes={} network_ms={network_ms}",
        window.as_ref().map_or(fetched_full, Vec::len),
        flag_sync
            .as_ref()
            .filter(|sync| sync.complete)
            .map_or(-1, |sync| sync.changes.len() as i64)
    );
    let synced_validity = batch.uidvalidity;
    let catchup_uids: Vec<_> = if prior_validity != 0 && prior_validity == synced_validity {
        missing_files
            .iter()
            .copied()
            .filter(|uid| !batch.messages.iter().any(|header| header.uid == *uid))
            .collect()
    } else {
        Vec::new()
    };
    let attempted: Vec<_> = missing_files
        .into_iter()
        .filter(|uid| batch.messages.iter().any(|header| header.uid == *uid))
        .collect();
    let tag_engine = engine.clone();
    let tag_account = account.to_string();
    let rule_folder = folder.to_string();
    let catchup_engine = engine.clone();
    let catchup_account = account.to_string();
    let catchup_folder = folder.to_string();

    let synced_messages = batch.messages.clone();
    // The window's size either way, so "synced" means what it always did.
    let count = window.as_ref().map_or(synced_messages.len(), Vec::len);
    // The lowest UID from which every message on the server was in the window:
    // all of them when the window wasn't full, else the window's oldest UID.
    let covered_from = if (count as u32) < limit {
        0
    } else {
        window
            .as_ref()
            .and_then(|window| window.iter().min().copied())
            .or_else(|| synced_messages.iter().map(|message| message.uid).min())
            .unwrap_or(u32::MAX)
    };
    crate::mlog!(
        crate::log::Level::Debug,
        "mail.sync",
        "messages account={account} folder={folder}: fetched={count} \
         uidvalidity={} uid_next={} prior_validity={prior_validity} server_uids={}",
        batch.uidvalidity,
        batch.uid_next,
        server_uids.as_ref().map_or(-1, |u| u.len() as i64)
    );
    // AI sorting of new Inbox mail, when it is on and set up for this account.
    let ai_plan = ai_sort::plan_for(engine, account, folder).await;
    let ai_hold = ai_plan.is_some();
    let engine = engine.clone();
    let account = account.to_string();
    let folder = folder.to_string();
    // Outside the network timeout: always observe the committed result.
    let (mut result, removal_guard, ai_candidates, new_unread, mark_filed, uncached_marked, deferred_modseq) = tokio::task::spawn_blocking(move || {
    let account = account.as_str();
    let folder = folder.as_str();
    let db = crate::log::timed_db_lock(&engine.db, "sync_messages.persist");
    let current = store::get_folder_state(&db, account, folder)?;
    let notifies = store::folder_notifies(&db, account, folder)?;
    // Also when AI sorting may file this folder's new mail into a Meron folder
    // with notifications on; announced as the folder's own only when it notifies.
    // And when another device's Meron-folder mark may file new Inbox mail into
    // a Meron folder with notifications on.
    let marks_notify = folder.eq_ignore_ascii_case("INBOX") && store::meron_any_folder_notifies(&db)?;
    let new_unread = if current.is_some_and(|(validity, _)| validity == batch.uidvalidity) && (notifies || ai_hold || marks_notify) {
        store::classify_arrivals(&db, account, folder, current.unwrap().1, batch.uid_next, &batch.messages)?
    } else { Vec::new() };
    let arrivals = if notifies { new_unread.clone() } else { Vec::new() };
    // Before the upsert records them as observed. Nothing on a first sync, when
    // everything is new to the store and none of it to the mailbox.
    let first_seen_headers = match current {
        Some((validity, uid_next)) if validity == batch.uidvalidity => {
            store::first_seen_messages(&db, account, folder, uid_next, batch.uid_next, &batch.messages)?
        }
        _ => Vec::new(),
    };
    let first_seen: Vec<u32> = first_seen_headers.iter().map(|message| message.uid).collect();
    // Held back before the upsert, so no other connection lists the row first.
    // Another folder AI sorting watches (mail a server filter delivered
    // there): only what arrived since its first look, never on that look.
    // Its watch only moves past what this sync's window covered, so mail that
    // didn't fit is read later (see `ai_sort::check_watched_folders`).
    let hold = || if folder.eq_ignore_ascii_case("INBOX") {
        store::ai_sort_hold(&db, account, folder, batch.uidvalidity, &first_seen_headers, store::now_unix())
    } else {
        store::ai_sort_hold_folder(&db, account, folder, batch.uidvalidity, batch.uid_next, (covered_from, batch.uid_next), &batch.messages, store::now_unix())
    };
    // New Inbox copies (Gmail) of mail the AI already sent to a server folder
    // from a label: they get that move, without asking the AI again.
    if ai_hold && folder.eq_ignore_ascii_case("INBOX") {
        if let Err(err) = store::ai_sort_reuse_for_inbox(&db, account, batch.uidvalidity, &batch.messages, store::now_unix()) {
            crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account}: reusing decisions skipped: {err:#}");
        }
    }
    // Synced while sorting is off for it: as in the Inbox, that mail is never
    // sorted later, so a watched folder's watch moves past it.
    if !ai_hold && !folder.eq_ignore_ascii_case("INBOX") {
        store::ai_sort_watch_skip(&db, account, folder, batch.uidvalidity, batch.uid_next)?;
    }
    let mut ai_candidates = if ai_hold {
        hold()
            .unwrap_or_else(|err| {
                crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account}: hold skipped: {err:#}");
                Vec::new()
            })
    } else {
        Vec::new()
    };
    let persist_started = std::time::Instant::now();
    let phase_started = std::time::Instant::now();
    if prior_validity != 0 && prior_validity != batch.uidvalidity {
        store::clear_folder_messages(&db, account, folder)?;
        store::ai_sort_clear_folder(&db, account, folder)?;
    }
    let clear_time = phase_started.elapsed();
    let phase_started = std::time::Instant::now();
    let fetched_uids: Vec<u32> = batch.messages.iter().map(|message| message.uid).collect();
    store::release_removed_messages(&db, account, folder, &fetched_uids, removed_epoch)?;
    // Inbox mail that was there before Meron watched, read now only because the
    // window reached further back: kept out of the Inbox views. Decided before
    // the upsert, which would make it look cached already, and marked in the
    // same write that stores it.
    let backfill = store::inbox_backfill_uids(&db, account, folder, batch.uidvalidity, batch.uid_next, &batch.messages)?;
    store::upsert_messages_marking(&db, account, folder, &batch.messages, &backfill)?;
    store::mark_attachment_metadata_attempted(&db, account, folder, &attempted)?;
    let upsert_time = phase_started.elapsed();
    // Make sure the folder is represented in the folders table so its unread
    // count surfaces (tray dot / badges) even before a full folder LIST sync —
    // which, in the unified view, may never run for this account.
    let phase_started = std::time::Instant::now();
    store::ensure_folder(&db, account, folder)?;
    let ensure_time = phase_started.elapsed();
    let phase_started = std::time::Instant::now();
    let mut pruned = 0;
    if let Some(uids) = server_uids.as_ref() {
        let validity_ok = prior_validity == 0 || prior_validity == batch.uidvalidity;
        // An empty UID set means "the server holds nothing here" only if the
        // fetch also came back empty. Having just read `count` messages out of
        // this same mailbox, an empty SEARCH result contradicts itself — the
        // connection lied (see the CONDSTORE desync in imap::sync_flags) — and
        // pruning against it would delete the very messages just stored.
        let uids_credible = !uids.is_empty() || count == 0;
        if validity_ok && uids_credible {
            // The batch's UIDNEXT was read before the UID list, so any row at
            // or above it was cached after this snapshot and is not stale.
            pruned =
                store::prune_missing_messages(&db, account, folder, uids, batch.uid_next)?;
        } else if !uids_credible {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "skipping prune for account={account} folder={folder}: \
                 UID SEARCH returned no UIDs but {count} messages were fetched"
            );
        }
    }
    let prune_time = phase_started.elapsed();
    // Mail the AI shows in the Inbox from another folder follows what this
    // sync stored: gone from its folder, or now in the Inbox itself.
    if let Err(err) = store::ai_sort_inbox_after_sync(&db, account, folder, batch.uidvalidity, batch.uid_next, &batch.messages) {
        crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account} folder={folder}: shown-in-Inbox upkeep skipped: {err:#}");
    }
    // Meron folder releases a failed write left behind, from any account.
    crate::meron_folders::retry_pending_releases(&db);
    let flag_count = flag_sync.as_ref().map_or(0, |fs| fs.changes.len());
    let phase_started = std::time::Instant::now();
    // Meron-folder marks this sync read on the Inbox's copies: new mail, and
    // flag reads of cached mail. Only responses that carried FLAGS count.
    let mut marks_seen: Vec<store::MeronMarkSeen> = Vec::new();
    for message in batch.messages.iter().filter(|message| message.server_tags.is_some()) {
        if let Some(mark) = &message.meron_mark {
            marks_seen.push(store::MeronMarkSeen { uid: message.uid, mark: mark.clone() });
        }
    }
    // Cached window messages an incremental sync did not re-read in full:
    // only their flags and tags change, leaving every other column as is.
    for (uid, seen, starred, tags, mark) in &window_flags {
        store::update_message_seen(&db, account, folder, *uid, *seen)?;
        store::update_message_starred(&db, account, folder, *uid, *starred)?;
        store::observe_server_tags(&db, account, folder, *uid, tags)?;
        if let Some(mark) = mark {
            marks_seen.push(store::MeronMarkSeen { uid: *uid, mark: mark.clone() });
        }
    }
    // The CONDSTORE position this sync reached; stored after the marks are
    // read, as one skipped there must be read again.
    let mut reached_modseq = 0;
    if let Some(fs) = flag_sync {
        for (uid, seen, starred, tags, mark) in &fs.changes {
            if let Some(mark) = mark {
                marks_seen.push(store::MeronMarkSeen { uid: *uid, mark: mark.clone() });
            }
            store::update_message_seen(&db, account, folder, *uid, *seen)?;
            store::update_message_starred(&db, account, folder, *uid, *starred)?;
            store::observe_server_tags(&db, account, folder, *uid, tags)?;
        }
        if let Some(baseline) = &fs.tag_baseline {
            for (uid, tags) in baseline {
                store::observe_server_tags(&db, account, folder, *uid, tags)?;
            }
            store::mark_tag_baseline(&db, account, folder)?;
        }
        reached_modseq = fs.highest_modseq;
    }
    let flags_time = phase_started.elapsed();
    // Another device filed (or took out) Inbox or archived mail in a Meron
    // folder: follow.
    let mut mark_filed = Vec::new();
    let mut uncached_marked = Vec::new();
    if store::meron_reads_marks(&db, account, folder) {
        // Copies gone from the server first: one still remembered would keep
        // its conversation filed when its last other copy loses the mark.
        if let Some(uids) = server_uids.as_ref().filter(|uids| !uids.is_empty()) {
            let _ = store::meron_marks_prune(&db, account, folder, uids);
        }
        match store::meron_marks_observe(&db, account, folder, &marks_seen, read_at, store::now_unix()) {
            Ok(applied) => {
                mark_filed = applied.filed;
                uncached_marked = applied.uncached_marked;
                // Marks left unread: the next sync's CHANGEDSINCE reads them again.
                if applied.skipped_stale {
                    reached_modseq = 0;
                }
            }
            Err(err) => {
                crate::mlog!(crate::log::Level::Warn, "mail.meronMarks", "account={account}: reading Meron folder marks skipped: {err:#}");
                reached_modseq = 0;
            }
        }
    }
    // Archived marked mail this sync couldn't cache is read after it: the
    // CONDSTORE position moves past those marks only once every one is read
    // (see `fetch_uncached_marked`), so none is lost if that read fails.
    let deferred_modseq = if uncached_marked.is_empty() { 0 } else { reached_modseq };
    if reached_modseq > 0 && deferred_modseq == 0 {
        store::set_folder_modseq(&db, account, folder, reached_modseq)?;
    }
    if let Some(unseen) = server_unseen.as_ref() {
        store::set_uncached_unseen(&db, account, folder, unseen)?;
    }
    if let Err(err) = store::apply_rules(&db, account, folder, batch.uidvalidity, &first_seen) {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.rules",
            "account={account} folder={folder}: rules skipped: {err:#}"
        );
    }
    // The user's rules come first: what they queued work for isn't the AI's.
    if !ai_candidates.is_empty() {
        let candidates = std::mem::take(&mut ai_candidates);
        let uids: Vec<u32> = candidates.iter().map(|candidate| candidate.uid).collect();
        ai_candidates = store::ai_sort_skip_rule_actions(&db, account, folder, candidates)
            .unwrap_or_else(|err| {
                crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account}: sort skipped: {err:#}");
                let _ = store::ai_sort_release(&db, account, folder, &uids);
                Vec::new()
            });
    }
    // A conversation already in a Meron folder (the upsert just adopted it) is
    // never re-sorted: shown at once, its cap charge given back.
    if !ai_candidates.is_empty() {
        let candidates = std::mem::take(&mut ai_candidates);
        let uids: Vec<u32> = candidates.iter().map(|candidate| candidate.uid).collect();
        ai_candidates = store::ai_sort_skip_meron_foldered(&db, account, folder, candidates)
            .unwrap_or_else(|err| {
                crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account}: sort skipped: {err:#}");
                let _ = store::ai_sort_release(&db, account, folder, &uids);
                Vec::new()
            });
    }
    let phase_started = std::time::Instant::now();
    store::set_folder_state(&db, account, folder, batch.uidvalidity, batch.uid_next)?;
    let state_time = phase_started.elapsed();
    let total = persist_started.elapsed();
    drop(db);
    if total.as_millis() >= 100 {
        crate::mlog!(
            crate::log::Level::Warn,
            "sync.persist.timing",
            "account={account} folder={folder:?} fetched={count} mode={mode} fetched_full={fetched_full} fetched_flags={fetched_flags} server_uids={} pruned={pruned} flag_changes={flag_count} clear_ms={} upsert_ms={} ensure_ms={} prune_ms={} flags_ms={} state_ms={} total_ms={}",
            server_uids.as_ref().map_or(0, |uids| uids.len()),
            clear_time.as_millis(),
            upsert_time.as_millis(),
            ensure_time.as_millis(),
            prune_time.as_millis(),
            flags_time.as_millis(),
            state_time.as_millis(),
            total.as_millis()
        );
    }
    Ok::<_, anyhow::Error>((SyncMessagesResult {
        count, messages: synced_messages, arrivals, meron_arrivals: Vec::new(),
    }, removal_guard, ai_candidates, new_unread, mark_filed, uncached_marked, deferred_modseq))
    }).await??;
    // A conversation in the built-in Archive that got Inbox mail left it in
    // the upsert above: its marks go now.
    crate::meron_marks::flush_archive_unmarks(&tag_engine);
    // Archived mail another device filed whose copy this sync didn't cache:
    // read and stored, then its mark is acted on.
    if !uncached_marked.is_empty() {
        let all_read = fetch_uncached_marked(&tag_engine, &tag_account, &rule_folder, uncached_marked).await;
        // Every one read: the position the sync reached can be kept now. A
        // later sync that already moved further is left alone.
        if all_read && deferred_modseq > 0 {
            let db = tag_engine.db.lock().unwrap();
            let stored = store::get_folder_modseq(&db, &tag_account, &rule_folder).unwrap_or(0);
            if stored < deferred_modseq
                && let Err(err) = store::set_folder_modseq(&db, &tag_account, &rule_folder, deferred_modseq)
            {
                crate::mlog!(crate::log::Level::Warn, "mail.meronMarks", "account={tag_account}: keeping the sync position skipped: {err:#}");
            }
        }
    }

    // The main sync is already committed. Catch-up gets its own small budget;
    // an exhausted deadline cannot discard new mail, flags, or sync markers.
    if !catchup_uids.is_empty() {
        let deadline = tokio::time::Instant::now() + catchup_budget;
        match fetch_headers_isolating_until(
            &catchup_engine,
            &catchup_account,
            &catchup_folder,
            catchup_uids,
            Some(deadline),
        )
        .await
        {
            Ok((headers, skipped)) => {
                let persist = tokio::task::spawn_blocking(move || {
                    let _removal_guard = removal_guard;
                    let db = catchup_engine.db.lock().unwrap();
                    if store::get_folder_state(&db, &catchup_account, &catchup_folder)?
                        .map(|(validity, _)| validity)
                        != Some(synced_validity)
                    {
                        return anyhow::Ok(());
                    }
                    let attempted: Vec<_> = headers
                        .iter()
                        .map(|header| header.uid)
                        .chain(skipped)
                        .collect();
                    store::upsert_messages(&db, &catchup_account, &catchup_folder, &headers)?;
                    store::mark_attachment_metadata_attempted(
                        &db,
                        &catchup_account,
                        &catchup_folder,
                        &attempted,
                    )?;
                    anyhow::Ok(())
                })
                .await;
                if let Err(err) = persist
                    .map_err(anyhow::Error::from)
                    .and_then(|result| result)
                {
                    crate::mlog!(
                        crate::log::Level::Warn,
                        "mail.sync",
                        "attachment metadata persistence skipped: {err:#}"
                    );
                }
            }
            Err(err) => crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "attachment metadata catch-up skipped: {err:#}"
            ),
        }
    }
    let tags_pending = store::has_pending_tags(&tag_engine.db.lock().unwrap(), &tag_account)?;
    if tags_pending {
        spawn_tag_push(&tag_engine, &tag_account);
    }
    // Awaited, unlike the tags, so what the caller announces next is where the
    // rules left it: mail they read or moved doesn't notify, and mail they failed
    // to still does. Whatever doesn't finish stays queued for the next sync.
    // Within its own budget, before the queue runs the moves it decides on.
    let sorted = match ai_plan {
        Some(plan) if !ai_candidates.is_empty() => {
            ai_sort::sort_new_mail(
                &tag_engine,
                &tag_account,
                &rule_folder,
                synced_validity,
                plan,
                ai_candidates,
            )
            .await
        }
        _ => ai_sort::SortedNewMail::default(),
    };
    // What another device's marks filed is announced the same way, and stays
    // silent as the folder's own arrivals.
    let mut filed = sorted.filed;
    filed.extend(mark_filed.iter().cloned());
    result.meron_arrivals = ai_sort::meron_arrivals(&tag_engine, &rule_folder, &new_unread, &filed);
    let mut ai_moved = sorted.silenced;
    ai_moved.extend(mark_filed.iter().map(|(uid, _)| *uid));
    let mut silenced = std::collections::BTreeSet::new();
    let rules_ran = tokio::time::timeout(
        RULE_ACTIONS_BUDGET,
        run_rule_actions(&tag_engine, &tag_account, &rule_folder, &mut silenced),
    )
    .await;
    match rules_ran {
        Ok(Ok(())) => {}
        Ok(Err(err)) => crate::mlog!(
            crate::log::Level::Warn,
            "mail.rules",
            "account={tag_account}: rule actions skipped: {err:#}"
        ),
        // A large message can take longer to copy than any sync may wait.
        Err(_) => spawn_rule_worker(&tag_engine, &tag_account, &rule_folder),
    }
    // Mail the AI is moving doesn't alert, even while the move still waits.
    silenced.extend(ai_moved);
    // An AI move that failed for good alerts now, as a failed rule move does.
    let late = if rule_folder.eq_ignore_ascii_case("INBOX") {
        let db = tag_engine.db.lock().unwrap();
        store::ai_sort_take_alerts(&db, &tag_account, &rule_folder)
            .ok()
            .filter(|_| store::folder_notifies(&db, &tag_account, &rule_folder).unwrap_or(false))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    ai_sort::settle_arrivals(&mut result.arrivals, &silenced, late);
    Ok(result)
}

/// New mail of a folder AI sorting watches (besides the Inbox) that a sync's
/// window couldn't hold, because more arrived at once than it reads: the
/// oldest `max` of it not cached yet, from the folder's watch up, read and put
/// through the same steps as a sync's new mail (held, stored, rules, then the
/// AI). The watch moves past what was read; the rest waits for the next check.
/// Answers with what it filed into Meron folders with notifications on.
pub(super) async fn sort_carried_mail(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    max: usize,
) -> anyhow::Result<Vec<ai_sort::MeronArrivals>> {
    let Some(plan) = ai_sort::plan_for(engine, account, folder).await else {
        return Ok(Vec::new());
    };
    let (validity, uid_next, from) = {
        let db = engine.db.lock().unwrap();
        let state = store::get_folder_state(&db, account, folder)?;
        let watch = store::ai_sort_watch(&db, account, folder)?;
        match (state, watch) {
            (Some((validity, uid_next)), Some((watched, from)))
                if validity == watched && from < uid_next =>
            {
                (validity, uid_next, from)
            }
            _ => return Ok(Vec::new()),
        }
    };
    let guard = MessageSyncGuard::begin(engine)?;
    let listed = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            Box::pin(async move { imap::uids_from(session, &folder, from).await })
        })
        .await?;
    let cached = store::header_synced_uids(&engine.db.lock().unwrap(), account, folder)?;
    let missing: Vec<u32> = listed
        .into_iter()
        .filter(|uid| *uid < uid_next && !cached.contains(uid))
        .collect();
    let take: Vec<u32> = missing.iter().copied().take(max).collect();
    // Past the last UID read when some are left for later; else all the way.
    let read_to = match take.last() {
        Some(last) if take.len() < missing.len() => last + 1,
        _ => uid_next,
    };
    let (headers, skipped) = if take.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        fetch_headers_isolating_unparseable(engine, account, folder, take).await?
    };
    // Mail that couldn't be read this time is asked for again next check:
    // the mark stops before the first such UID.
    let read_to = skipped.iter().copied().min().map_or(read_to, |first| read_to.min(first));
    let persist_engine = engine.clone();
    let persist_account = account.to_string();
    let persist_folder = folder.to_string();
    let (candidates, ruled, new_unread) = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        let account = persist_account.as_str();
        let folder = persist_folder.as_str();
        let db = persist_engine.db.lock().unwrap();
        if store::get_folder_state(&db, account, folder)?.map(|(validity, _)| validity)
            != Some(validity)
        {
            return anyhow::Ok((Vec::new(), false, Vec::new()));
        }
        // As a sync does: first seen and held before the upsert stores them.
        let first_seen_headers =
            store::first_seen_messages(&db, account, folder, from, uid_next, &headers)?;
        let first_seen: Vec<u32> = first_seen_headers.iter().map(|header| header.uid).collect();
        // What a sync would count as arrivals: new to the account and unread.
        let new_unread: Vec<imap::MessageHeader> = first_seen_headers
            .into_iter()
            .filter(|header| !header.seen)
            .collect();
        let held = store::ai_sort_hold_folder(
            &db,
            account,
            folder,
            validity,
            uid_next,
            (from, read_to),
            &headers,
            store::now_unix(),
        )?;
        store::upsert_messages(&db, account, folder, &headers)?;
        if let Err(err) = store::apply_rules(&db, account, folder, validity, &first_seen) {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.rules",
                "account={account} folder={folder}: rules skipped: {err:#}"
            );
        }
        let held = store::ai_sort_skip_rule_actions(&db, account, folder, held)?;
        let held = store::ai_sort_skip_meron_foldered(&db, account, folder, held)?;
        anyhow::Ok((held, !first_seen.is_empty(), new_unread))
    })
    .await??;
    let mut alerts = Vec::new();
    if !candidates.is_empty() {
        let sorted =
            ai_sort::sort_new_mail(engine, account, folder, validity, plan, candidates).await;
        alerts = ai_sort::meron_arrivals(engine, folder, &new_unread, &sorted.filed);
    }
    if ruled {
        spawn_rule_worker(engine, account, folder);
    }
    Ok(alerts)
}

/// How long a sync waits on the server work its rules queued.
const RULE_ACTIONS_BUDGET: Duration = Duration::from_secs(60);

pub(super) const DEFAULT_BACKGROUND_SYNC_TIMEOUT_SECS: u64 = 30;
pub(super) const MAX_BACKGROUND_SYNC_TIMEOUT_SECS: u64 = 24 * 60 * 60;

pub(super) fn parse_background_sync_timeout(value: Option<&str>) -> Duration {
    value
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|&secs| (1..=MAX_BACKGROUND_SYNC_TIMEOUT_SECS).contains(&secs))
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(DEFAULT_BACKGROUND_SYNC_TIMEOUT_SECS))
}

/// Per-folder sync budget, shared by background syncs and each piggybacked
/// companion sync so one slow mailbox can't hold the post-sync tail (and the
/// emit that follows it) indefinitely. The 30s default suits direct IMAP
/// servers; slow gateways (e.g. DavMail bridging Exchange EWS) can need more,
/// so it can be overridden with `MERON_SYNC_TIMEOUT` (seconds, up to one day).
pub fn background_sync_timeout() -> Duration {
    static TIMEOUT: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        let value = std::env::var("MERON_SYNC_TIMEOUT").ok();
        parse_background_sync_timeout(value.as_deref())
    })
}

/// The companion mailboxes (Sent, then Drafts) to piggyback onto a sync of
/// another folder, each paired with its role label for the caller's logs.
/// Callers resolve the names via `cached_sent_folder`/`cached_drafts_folder`,
/// which already exclude the folder being synced; this only guards against the
/// two roles resolving to the same mailbox.
pub(super) fn companion_folders(
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

/// Archive-mailbox copies whose mark names a Meron folder here but which the
/// sync didn't cache: read in batches of [`UNCACHED_MARKED_CAP`] (newest
/// first), stored, and their marks acted on. Answers whether every one was
/// read; when not, the sync keeps its old CONDSTORE position, so the next one
/// reports the rest again. Logged.
async fn fetch_uncached_marked(engine: &Arc<Engine>, account: &str, folder: &str, mut uids: Vec<u32>) -> bool {
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
async fn fetch_uncached_marked_batch(engine: &Arc<Engine>, account: &str, folder: &str, uids: &[u32]) -> bool {
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
            crate::mlog!(crate::log::Level::Warn, "mail.meronMarks", "account={account}: reading archived marked mail skipped: {err:#}");
            return false;
        }
    };
    for header in &mut headers {
        header.folder = folder.to_string();
    }
    let db = engine.db.lock().unwrap();
    if let Err(err) = store::upsert_read_messages(&db, account, folder, &headers) {
        crate::mlog!(crate::log::Level::Warn, "mail.meronMarks", "account={account}: storing archived marked mail skipped: {err:#}");
        return false;
    }
    let seen: Vec<store::MeronMarkSeen> = headers
        .iter()
        .filter(|header| header.server_tags.is_some())
        .filter_map(|header| {
            header
                .meron_mark
                .clone()
                .map(|mark| store::MeronMarkSeen { uid: header.uid, mark })
        })
        .collect();
    match store::meron_marks_observe(&db, account, folder, &seen, read_at, store::now_unix()) {
        // A mark this device wrote since the read began is read again later.
        Ok(applied) => !applied.skipped_stale,
        Err(err) => {
            crate::mlog!(crate::log::Level::Warn, "mail.meronMarks", "account={account}: archived marks skipped: {err:#}");
            false
        }
    }
}

#[cfg(test)]
mod owed_reread_tests {
    use super::*;

    // A bulk run puts off the re-read of the folder it moves mail into. What
    // it skipped has to be remembered, or the run's last re-read (sized for
    // one conversation) would leave the earlier ones out of the cache.
    #[test]
    fn a_put_off_reread_is_remembered_until_the_next_one_takes_it() {
        let (account, folder) = ("owed-test@example.com", "Trash");
        assert_eq!(take_owed_reread(account, folder), 0);
        owe_reread(account, folder, 12);
        owe_reread(account, folder, 30);
        owe_reread(account, "Archive", 5);
        assert_eq!(take_owed_reread(account, folder), 42);
        assert_eq!(take_owed_reread(account, folder), 0);
        assert_eq!(take_owed_reread(account, "Archive"), 5);
        owe_reread(account, folder, MAX_OWED_REREAD);
        owe_reread(account, folder, 10);
        assert_eq!(take_owed_reread(account, folder), MAX_OWED_REREAD);
    }
}

#[cfg(test)]
mod removal_guard_tests {
    use super::*;

    struct TestHost;

    impl EngineHost for TestHost {
        fn open_db(&self) -> anyhow::Result<rusqlite::Connection> {
            store::open_at(":memory:")
        }
        fn apply_secret(&self, _: &rusqlite::Connection, _: &str, _: &mut imap::Creds) {}
        fn store_secret(
            &self,
            _: &rusqlite::Connection,
            _: &str,
            _: &crate::secrets::Secrets,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    async fn metadata_test_server(
        total: u32,
        poison_uids: Vec<u32>,
    ) -> (
        Arc<Engine>,
        tokio::task::JoinHandle<()>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        metadata_test_server_with_stall(total, poison_uids, None).await
    }

    async fn metadata_test_server_with_stall(
        total: u32,
        poison_uids: Vec<u32>,
        stalled_uid: Option<u32>,
    ) -> (
        Arc<Engine>,
        tokio::task::JoinHandle<()>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
        use std::sync::atomic::AtomicUsize;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let commands_after_poison = Arc::new(AtomicUsize::new(0));
        let server_counter = commands_after_poison.clone();
        let server = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let counter = server_counter.clone();
                let poison_uids = poison_uids.clone();
                tokio::spawn(async move {
                    let (reader, mut writer) = socket.into_split();
                    writer.write_all(b"* OK IMAP4rev1 ready\r\n").await.unwrap();
                    let mut lines = tokio::io::BufReader::new(reader).lines();
                    let mut poisoned = false;
                    while let Ok(Some(line)) = lines.next_line().await {
                        if poisoned {
                            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                        let (tag, command) = line.split_once(' ').unwrap();
                        let mut reply = if command.starts_with("SELECT") {
                            format!(
                                "* {total} EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n* OK [UIDNEXT {}] next\r\n",
                                total + 1
                            )
                        } else if command.starts_with("CAPABILITY") {
                            "* CAPABILITY IMAP4rev1\r\n".to_string()
                        } else if command.starts_with("UID SEARCH") {
                            if command.contains("UNSEEN") {
                                "* SEARCH\r\n".to_string()
                            } else {
                                format!(
                                    "* SEARCH {}\r\n",
                                    (1..=total)
                                        .map(|uid| uid.to_string())
                                        .collect::<Vec<_>>()
                                        .join(" ")
                                )
                            }
                        } else if command.starts_with("UID FETCH") {
                            let set = command.split_whitespace().nth(2).unwrap();
                            let mut uids = Vec::new();
                            // `*` is the newest message, as in "1:*" (the whole folder).
                            let bound = |text: &str| match text {
                                "*" => total,
                                number => number.parse::<u32>().unwrap(),
                            };
                            for segment in set.split(',') {
                                if let Some((lo, hi)) = segment.split_once(':') {
                                    uids.extend(bound(lo)..=bound(hi));
                                } else {
                                    uids.push(bound(segment));
                                }
                            }
                            // Only a read that asks for BODYSTRUCTURE can carry the
                            // broken one: a flags-only read neither poisons nor stalls.
                            if !command.contains("BODYSTRUCTURE") {
                                uids.iter()
                                    .map(|uid| format!("* {uid} FETCH (UID {uid} FLAGS (\\Seen))\r\n"))
                                    .collect()
                            } else if let Some(uid) = uids.iter().find(|uid| poison_uids.contains(uid)) {
                                poisoned = true;
                                format!(
                                    "* {uid} FETCH (UID {uid} BODYSTRUCTURE (\"APPLICATION\" \"café\" NIL NIL NIL \"7BIT\" 1))\r\n"
                                )
                            } else if stalled_uid.is_some_and(|uid| uids.contains(&uid)) {
                                // Hold a partial FETCH open until cancellation drops
                                // the connection. No later command may reuse it.
                                writer.write_all(b"* 1 FETCH (UID 1 ").await.unwrap();
                                if let Ok(Some(_)) = lines.next_line().await {
                                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                }
                                break;
                            } else {
                                uids.iter().map(|uid| {
                                    if *uid == 2 { format!("* 2 FETCH (UID 2 FLAGS (\\Seen) BODYSTRUCTURE (\"APPLICATION\" \"PDF\" (\"NAME\" \"older.pdf\") NIL NIL \"BASE64\" 4))\r\n") }
                                    else if *uid == total { format!("* {uid} FETCH (UID {uid} FLAGS (\\Seen) BODYSTRUCTURE (\"TEXT\" \"PLAIN\" NIL NIL NIL \"7BIT\" 4 1))\r\n") }
                                    else { format!("* {uid} FETCH (UID {uid} FLAGS (\\Seen))\r\n") }
                                }).collect()
                            }
                        } else if command.starts_with("FETCH") {
                            format!(
                                "* {total} FETCH (UID {total} FLAGS (\\Seen) BODYSTRUCTURE (\"TEXT\" \"PLAIN\" NIL NIL NIL \"7BIT\" 4 1))\r\n"
                            )
                        } else {
                            String::new()
                        };
                        reply.push_str(&format!("{tag} OK done\r\n"));
                        if writer.write_all(reply.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        let engine = Arc::new(Engine::new(Box::new(TestHost)).unwrap());
        let mut creds = {
            let db = engine.db.lock().unwrap();
            let config = serde_json::json!({"host":"127.0.0.1", "port":port, "tls":false, "user":"u", "proxy":{"mode":"direct"}}).to_string();
            db.execute(
                "INSERT INTO accounts(id, config) VALUES('acct', ?1)",
                [&config],
            )
            .unwrap();
            store::set_folder_state(&db, "acct", "INBOX", 42, total + 1).unwrap();
            let headers: Vec<_> = (1..=total)
                .map(|uid| imap::MessageHeader {
                    uid,
                    date: uid as i64,
                    seen: true,
                    files: (uid == total).then(Vec::new),
                    ..Default::default()
                })
                .collect();
            store::upsert_messages(&db, "acct", "INBOX", &headers).unwrap();
            store::load_account(&db, "acct").unwrap().unwrap()
        };
        creds.password = "p".into();
        engine.accounts.lock().await.insert("acct".into(), creds);
        (engine, server, commands_after_poison)
    }

    /// UID 4 poisons every FETCH containing it, UID 3 omits BODYSTRUCTURE,
    /// and UID 2 carries a file. Catch-up must reach UID 2 on the next sync.
    #[tokio::test]
    async fn catchup_discards_poison_sessions_advances_and_preserves_sync_count() {
        use std::sync::atomic::Ordering;
        let (engine, server, commands_after_poison) = metadata_test_server(5, vec![4]).await;
        for (expected_missing, expected_read) in [(vec![2, 1], 1), (vec![], 0)] {
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                sync_messages(&engine, "acct", "INBOX", 2),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result.count, 2);
            assert_eq!(result.messages.len(), expected_read);
            let db = engine.db.lock().unwrap();
            assert_eq!(
                store::missing_attachment_metadata_uids(&db, "acct", "INBOX", 10).unwrap(),
                expected_missing
            );
            let count: i64 = db
                .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 5, "pruning must retain all live cached rows");
        }
        assert_eq!(commands_after_poison.load(Ordering::SeqCst), 0);
        assert_eq!(
            store::get_recent_page(
                &engine.db.lock().unwrap(),
                "acct",
                "INBOX",
                50,
                None,
                store::RecentFilter {
                    unread_only: false,
                    attachments_only: true
                }
            )
            .unwrap()
            .0[0]
                .uid,
            2
        );
        server.abort();
    }

    #[tokio::test]
    async fn catchup_leaves_unread_chunks_eligible_at_isolation_limit() {
        let (engine, server, commands_after_poison) =
            metadata_test_server(51, (26..=50).collect()).await;
        let sync = || {
            tokio::time::timeout(
                Duration::from_secs(15),
                sync_messages(&engine, "acct", "INBOX", 50),
            )
        };
        assert_eq!(sync().await.unwrap().unwrap().count, 50);
        let missing = store::missing_attachment_metadata_uids(
            &engine.db.lock().unwrap(),
            "acct",
            "INBOX",
            100,
        )
        .unwrap();
        assert!(
            missing.contains(&1),
            "healthy UID in an unread chunk must stay eligible"
        );
        assert!(
            missing.len() < 50,
            "isolated poison UIDs should advance progress"
        );
        // Further bounded syncs finish the poison range and reach the healthy file.
        for _ in 0..5 {
            if store::missing_attachment_metadata_uids(
                &engine.db.lock().unwrap(),
                "acct",
                "INBOX",
                100,
            )
            .unwrap()
            .is_empty()
            {
                break;
            }
            sync().await.unwrap().unwrap();
        }
        let db = engine.db.lock().unwrap();
        assert!(
            store::missing_attachment_metadata_uids(&db, "acct", "INBOX", 100)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store::get_recent_page(
                &db,
                "acct",
                "INBOX",
                50,
                None,
                store::RecentFilter {
                    unread_only: false,
                    attachments_only: true
                }
            )
            .unwrap()
            .0[0]
                .uid,
            2
        );
        assert_eq!(
            commands_after_poison.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        server.abort();
    }

    #[tokio::test]
    async fn catchup_timeout_preserves_main_sync_and_completed_progress() {
        let (engine, server, commands_after_failure) =
            metadata_test_server_with_stall(5, vec![4], Some(1)).await;
        // The poison UID is isolated and UID 3 completes before the healthy
        // [1,2] chunk stalls. Both completed rows must survive the deadline.
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            sync_messages_with_policy(&engine, "acct", "INBOX", 4, true, Duration::from_secs(2)),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.count, 4);
        assert_eq!(
            store::missing_attachment_metadata_uids(
                &engine.db.lock().unwrap(),
                "acct",
                "INBOX",
                10
            )
            .unwrap(),
            vec![1]
        );
        for _ in 0..2 {
            {
                let db = engine.db.lock().unwrap();
                db.execute("DELETE FROM messages WHERE uid = 5", [])
                    .unwrap();
                store::set_folder_state(&db, "acct", "INBOX", 42, 5).unwrap();
            }
            // Repeated catch-up timeouts must not hold new mail or sync state
            // hostage, nor mark cancelled UIDs as attempted.
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                sync_messages_with_policy(
                    &engine,
                    "acct",
                    "INBOX",
                    4,
                    true,
                    Duration::from_millis(150),
                ),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result.count, 4);
            let db = engine.db.lock().unwrap();
            assert_eq!(
                store::get_folder_state(&db, "acct", "INBOX").unwrap(),
                Some((42, 6))
            );
            let seen: bool = db
                .query_row("SELECT seen FROM messages WHERE uid = 5", [], |r| r.get(0))
                .unwrap();
            assert!(
                seen,
                "new recent headers must be persisted despite catch-up timeout"
            );
            assert_eq!(
                store::missing_attachment_metadata_uids(&db, "acct", "INBOX", 10).unwrap(),
                vec![1]
            );
        }
        assert_eq!(
            commands_after_failure.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        server.abort();
    }

    #[tokio::test]
    async fn post_write_refresh_skips_a_failed_guard_registration() {
        let engine = Arc::new(Engine::new(Box::new(TestHost)).unwrap());
        // Force the registration INSERT to fail before any network request.
        engine
            .db
            .lock()
            .unwrap()
            .execute_batch("CREATE TEMP TABLE active_message_syncs (invalid_column INTEGER);")
            .unwrap();
        assert!(
            refresh_written_folder(&engine, "acct", "Drafts", 20)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn cancelling_a_snapshot_releases_its_removal_markers() {
        let engine = Arc::new(Engine::new(Box::new(TestHost)).unwrap());
        let worker_engine = engine.clone();
        let (ready, started) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = MessageSyncGuard::begin(&worker_engine).unwrap();
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        started.await.unwrap();
        {
            let db = engine.db.lock().unwrap();
            store::delete_messages_by_uid(&db, "acct", "INBOX", &[7]).unwrap();
            let count: i64 = db
                .query_row("SELECT count(*) FROM removed_message_uids", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(count, 1);
        }
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let db = engine.db.lock().unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM removed_message_uids", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}
