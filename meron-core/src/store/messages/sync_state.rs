//! Reconciles cached headers with the UIDs seen during synchronization.
//! Active sync owners retain removal markers while concurrent readers finish.
//! Folder clearing and pruning update the same stored message state.

use super::*;

/// Delete locally cached messages whose UIDs are no longer on the server.
///
/// `server_uids` must be the complete UID set for the folder, listed no earlier than `uid_next` was
/// read. Rows at or above `uid_next` arrived after that snapshot (a move's copy cached meanwhile,
/// say), so the list cannot speak for them and they are kept. With `0` (no UIDNEXT) the newest
/// listed UID bounds it instead; an empty listing prunes everything.
///
/// Returns the number of rows removed.
pub fn prune_missing_messages(
    conn: &Connection,
    account: &str,
    folder: &str,
    server_uids: &std::collections::HashSet<u32>,
    uid_next: u32,
) -> Result<usize> {
    let mut stmt = conn.prepare("SELECT uid FROM messages WHERE account = ?1 AND folder = ?2")?;
    let local_uids: Vec<u32> = stmt
        .query_map(params![account, folder], |row| row.get::<_, u32>(0))?
        .filter_map(|r| r.ok())
        .collect();
    drop(stmt);

    // Without a UIDNEXT from the server, the newest listed UID still bounds
    // the snapshot: anything above it was cached after the listing.
    let bound = if uid_next == 0 {
        server_uids
            .iter()
            .max()
            .map_or(0, |max| max.saturating_add(1))
    } else {
        uid_next
    };
    let mut removed = 0usize;
    let mut gone = Vec::new();
    for uid in local_uids {
        if (bound == 0 || uid < bound) && !server_uids.contains(&uid) {
            conn.execute(
                "DELETE FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                params![account, folder, uid],
            )?;
            gone.push(uid);
            removed += 1;
        }
    }
    remember_removed_uids(conn, account, folder, &gone)?;
    Ok(removed)
}

/// Epoch of the latest UID removal. A folder sync reads this before it talks
/// to the server; removals that happen while that fetch is in flight get a
/// higher epoch and stay hidden from that snapshot.
pub fn removed_message_epoch(conn: &Connection) -> Result<i64> {
    Ok(conn
        .query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'removed_uid_epoch'",
            [],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

/// Forget removals at or before `fetched_at_epoch` for these UIDs. The sync
/// that passes the epoch it read before fetching may store a UID only when
/// nothing removed it afterwards — the snapshot and the removal agree.
pub fn release_removed_messages(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
    fetched_at_epoch: i64,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "DELETE FROM removed_message_uids
         WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND epoch <= ?4",
    )?;
    for uid in uids {
        stmt.execute(params![account, folder, *uid, fetched_at_epoch])?;
    }
    Ok(())
}

/// Names this process in `active_message_syncs`. Rows carrying another name
/// were left by a process that died mid-sync.
fn message_sync_owner() -> &'static str {
    static OWNER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    OWNER.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// Register a folder sync before network I/O. The registry is an ordinary table because engines in
/// one process each hold their own connection (the mobile foreground engine and a transient
/// background one), and a removal on one must stay hidden from a snapshot another is still
/// fetching. Registration and cleanup run under the engine's database lock.
pub fn begin_message_sync(conn: &Connection) -> Result<(i64, i64)> {
    // Take the write lock before reading the epoch. A deferred transaction that
    // reads first fails with SQLITE_BUSY_SNAPSHOT when another connection
    // commits in between, and the busy timeout cannot wait that out.
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let epoch = removed_message_epoch(&tx)?;
    tx.execute(
        "INSERT INTO active_message_syncs(epoch, owner) VALUES(?1, ?2)",
        params![epoch, message_sync_owner()],
    )?;
    let id = tx.last_insert_rowid();
    prune_removed_message_markers(&tx)?;
    tx.commit()?;
    Ok((id, epoch))
}

/// A finished, failed or cancelled sync no longer needs its removal markers.
pub fn end_message_sync(conn: &Connection, id: i64) -> Result<()> {
    conn.execute(
        "DELETE FROM active_message_syncs WHERE id = ?1",
        params![id],
    )?;
    prune_removed_message_markers(conn)
}

fn prune_removed_message_markers(conn: &Connection) -> Result<()> {
    // Abandoned snapshots must not survive a restart and pin markers forever.
    conn.execute(
        "DELETE FROM active_message_syncs WHERE owner != ?1",
        params![message_sync_owner()],
    )?;
    // A snapshot begun at E cannot contain a UID removed before or at E.
    // Keep all later removals until the oldest active snapshot has finished.
    conn.execute(
        "DELETE FROM removed_message_uids
         WHERE epoch <= COALESCE((SELECT MIN(epoch) FROM active_message_syncs), ?1)",
        params![removed_message_epoch(conn)?],
    )?;
    Ok(())
}

pub(super) fn removed_uids_in(
    conn: &Connection,
    account: &str,
    folder: &str,
) -> Result<HashSet<u32>> {
    let mut stmt =
        conn.prepare("SELECT uid FROM removed_message_uids WHERE account = ?1 AND folder = ?2")?;
    let rows = stmt.query_map(params![account, folder], |row| row.get(0))?;
    Ok(rows.collect::<std::result::Result<HashSet<_>, _>>()?)
}

pub(crate) fn remember_removed_uids(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> Result<()> {
    let uids: Vec<u32> = uids.iter().copied().filter(|uid| *uid != 0).collect();
    if uids.is_empty() {
        return Ok(());
    }
    let epoch = removed_message_epoch(conn)? + 1;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES('removed_uid_epoch', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![epoch.to_string()],
    )?;
    let mut stmt = conn.prepare(
        "INSERT INTO removed_message_uids(account, folder, uid, epoch)
         VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(account, folder, uid) DO UPDATE SET epoch = excluded.epoch",
    )?;
    for uid in uids {
        stmt.execute(params![account, folder, uid, epoch])?;
    }
    Ok(())
}

pub fn clear_folder_messages(conn: &Connection, account: &str, folder: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM messages WHERE account = ?1 AND folder = ?2",
        params![account, folder],
    )?;
    clear_location_tags(conn, account, folder)?;
    conn.execute(
        "DELETE FROM uncached_unseen WHERE account = ?1 AND folder = ?2",
        params![account, folder],
    )?;
    // UIDVALIDITY changed: these UID numbers no longer name the messages that
    // were removed, so they must not block the folder's new contents.
    conn.execute(
        "DELETE FROM removed_message_uids WHERE account = ?1 AND folder = ?2",
        params![account, folder],
    )?;
    // Nor name Meron-folder members; sync re-adopts the mail by Message-ID.
    meron_clear_folder_uids(conn, account, folder)?;
    Ok(())
}
