//! Transfers in the mail synchronization engine.
//!
//! Coordinates the corresponding IMAP and store operations during folder updates.
//! The parent module exposes these operations to foreground and background sync.

use super::*;

/// Where [`delete_to_trash`] moved messages, and the Trash folder's UIDNEXT
/// from just before the move (see [`refresh_moved_copies`]).
#[derive(Clone)]
pub struct Trashed {
    pub folder: String,
    pub uid_next_before: Option<u32>,
}

/// Delete messages on the server.
///
/// Returns `None` for a permanent expunge (Drafts, or items already in Trash), or `Some` when moved
/// to Trash.
///
/// Inside a bulk run this is the move step every member shares (see [`crate::engine::bulk`]): one
/// command moves the mail of all of them.
pub async fn delete_to_trash(
    engine: &Engine,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> anyhow::Result<Option<Trashed>> {
    let Some(seat) = bulk::seat() else {
        return delete_to_trash_alone(engine, account, folder, uids).await;
    };
    let ask = bulk::MoveAsk {
        account: account.to_string(),
        folder: folder.to_string(),
        to: bulk::MoveTo::Trash,
        uids: uids.to_vec(),
    };
    match seat.moved(ask).await? {
        bulk::Moved::Trashed(trashed) => Ok(trashed),
        bulk::Moved::Folder(_) => anyhow::bail!("a delete was answered as a move"),
    }
}

/// [`delete_to_trash`] in one command of its own.
pub(crate) async fn delete_to_trash_alone(
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
        crate::engine::folders::find_role_folder(folders, special_use, looks_like, "")
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
                    None => imap::find_trash_folder(session).await?.ok_or_else(|| {
                        anyhow::anyhow!("Trash folder not found for this account")
                    })?,
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
    pub(super) _snapshot_guard: Option<MessageSyncGuard>,
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
    let uid_next_before = move_on_server(engine, account, folder, target, uids).await?;
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

/// Move `uids` from `folder` to `target` on the server.
///
/// Gives the target's UIDNEXT from just before the move (see [`imap::move_to_folder`]). Mutating,
/// so it never auto-retries.
///
/// Inside a bulk run this is the move step every member shares (see [`crate::engine::bulk`]): one
/// command moves the mail of all of them.
pub async fn move_on_server(
    engine: &Engine,
    account: &str,
    folder: &str,
    target: &str,
    uids: &[u32],
) -> anyhow::Result<Option<u32>> {
    let Some(seat) = bulk::seat() else {
        return move_on_server_alone(engine, account, folder, target, uids).await;
    };
    let ask = bulk::MoveAsk {
        account: account.to_string(),
        folder: folder.to_string(),
        to: bulk::MoveTo::Folder(target.to_string()),
        uids: uids.to_vec(),
    };
    match seat.moved(ask).await? {
        bulk::Moved::Folder(uid_next_before) => Ok(uid_next_before),
        bulk::Moved::Trashed(_) => anyhow::bail!("a move was answered as a delete"),
    }
}

/// [`move_on_server`] in one command of its own.
pub(crate) async fn move_on_server_alone(
    engine: &Engine,
    account: &str,
    folder: &str,
    target: &str,
    uids: &[u32],
) -> anyhow::Result<Option<u32>> {
    engine
        .with_write_session(account, |session| {
            let folder = folder.to_string();
            let target = target.to_string();
            let uids = uids.to_vec();
            Box::pin(async move { imap::move_to_folder(session, &folder, &target, &uids).await })
        })
        .await
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
        // In a bulk run only a few members read on their own at a time.
        let _turn = bulk::own_read_turn().await;
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
    // In a bulk run only a few members read on their own at a time.
    let _turn = bulk::own_read_turn().await;
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
