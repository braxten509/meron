//! Removes or restores a local folder assignment and records unfinished releases.
//! The move and undo commands use card snapshots to recover the original folder.
//! Pending releases can be retried when a later sync supplies the missing rows.

use super::*;

/// The Meron-folder card a thread id names, read before an action moves its
/// mail (the rows go once it has). `None` when it isn't in a Meron folder.
pub fn meron_card_release(
    conn: &Connection,
    account: &str,
    folder: &str,
    root: &str,
    uid: Option<u32>,
) -> Result<Option<MeronCardRelease>> {
    let Some((entry_id, folder_id)) = meron_entry_of_thread(conn, account, folder, root, uid)?
    else {
        return Ok(None);
    };
    let Some(key) = meron_thread_key(conn, account, folder, root, uid)? else {
        return Ok(None);
    };
    let (rows, mut aliases) = conversation_rows(conn, account, &key, MeronFiling::User)?;
    // Only aliases of this entry: a key another entry holds isn't this card's.
    let mut theirs = conn.prepare_cached(
        "SELECT 1 FROM meron_folder_aliases WHERE account = ?1 AND key = ?2 AND entry_id = ?3",
    )?;
    let mut keep = BTreeSet::new();
    for alias in std::mem::take(&mut aliases) {
        if theirs.exists(params![account, alias, entry_id])? {
            keep.insert(alias);
        }
    }
    // Found only through a message re-keyed since it was filed: none of the
    // card's keys is the entry's, so only its remembered UIDs go; the entry's
    // other conversations stay.
    let source: String = conn
        .query_row(
            "SELECT source FROM meron_folder_entries WHERE id = ?1",
            params![entry_id],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or_else(|| "user".to_string());
    Ok(Some(MeronCardRelease {
        entry_id,
        folder_id,
        source,
        aliases: keep,
        rows: rows
            .into_values()
            .map(|(_, folder, uid, _, _, _)| (folder, uid))
            .collect(),
    }))
}

/// Take one card's conversation out of its entry: its aliases and remembered
/// UIDs go, the entry's other conversations stay. An entry left with no alias
/// goes too. One transaction.
pub fn meron_release_card(conn: &Connection, card: &MeronCardRelease) -> Result<MeronReleased> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let account: Option<String> = tx
        .query_row(
            "SELECT account FROM meron_folder_entries WHERE id = ?1",
            params![card.entry_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(account) = account else {
        return Ok(MeronReleased::default());
    };
    let mut touched: BTreeSet<(String, String)> = card
        .rows
        .iter()
        .map(|(folder, _)| (account.clone(), folder.clone()))
        .collect();
    let mut removed = 0;
    for alias in &card.aliases {
        removed += tx.execute(
            "DELETE FROM meron_folder_aliases WHERE account = ?1 AND key = ?2 AND entry_id = ?3",
            params![account, alias, card.entry_id],
        )?;
    }
    for (folder, uid) in &card.rows {
        removed += tx.execute(
            "DELETE FROM meron_folder_uids
              WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND entry_id = ?4",
            params![account, folder, uid, card.entry_id],
        )?;
    }
    let left: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM meron_folder_aliases WHERE entry_id = ?1)",
        params![card.entry_id],
        |row| row.get(0),
    )?;
    let mut released = usize::from(removed > 0);
    if !left {
        let whole = meron_release_entry_in(&tx, &card.entry_id)?;
        touched.extend(whole.touched);
        released = released.max(whole.released);
    }
    tx.commit()?;
    Ok(MeronReleased { released, touched })
}

/// Undo of an action that took a card out of Meron folder `folder_id`: put
/// the conversations of the copies `(folder, uids)` (just moved back) into it
/// again. Falls back to `root` when none of the copies is cached. Copies
/// already back in a Meron folder are left where they are.
pub fn meron_refile(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
    root: &str,
    folder_id: &str,
    source: &str,
) -> Result<BTreeSet<String>> {
    if !meron_folder_exists(conn, folder_id)? {
        return Ok(BTreeSet::new());
    }
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for uid in uids {
        if let Some(key) = meron_conversation_key(conn, account, folder, *uid)? {
            keys.insert(key);
        }
    }
    // A `uid:` root names a UID of the folder the mail was in, not of this one.
    if keys.is_empty()
        && !root.starts_with("uid:")
        && let Some(key) = meron_thread_key(conn, account, folder, root, None)?
    {
        keys.insert(key);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut touched = BTreeSet::new();
    for key in keys {
        if !is_alias_key(&key) || entry_of_alias(&tx, account, &key)?.is_some() {
            continue;
        }
        let moved = meron_move_in_in(&tx, account, &key, folder_id, MeronFiling::User)?;
        if source == "ai" {
            tx.execute(
                "UPDATE meron_folder_entries SET source = 'ai' WHERE id = ?1",
                params![moved.entry_id],
            )?;
        }
        touched.extend(moved.touched);
    }
    tx.commit()?;
    Ok(touched)
}

/// Once per store: find the Meron-folder entries AI sorting filed (before `source` existed) and
/// mark them `source = 'ai'`; release those whose newest dated message is past
/// [`crate::store::AI_SORT_MAX_AGE_DAYS`] and none of whose messages is in the Inbox. Hand-filed entries
/// are never touched. One transaction. Returns how many were released.
pub fn meron_release_old_ai_entries(conn: &Connection, now: i64) -> Result<usize> {
    if crate::store::db::meta_get(conn, OLD_AI_ENTRIES_DONE_KEY)?.is_some() {
        return Ok(0);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let entries: Vec<(String, String, i64, String)> = tx
        .prepare("SELECT id, account, added_at, source FROM meron_folder_entries")?
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut identity_of = tx.prepare(&format!(
        "SELECT {} FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
        crate::store::ai_sort::IDENTITY_SQL
    ))?;
    let mut done_near = tx.prepare(
        "SELECT 1 FROM ai_sort_done
          WHERE account = ?1 AND identity = ?2 AND at BETWEEN ?3 AND ?4",
    )?;
    let cutoff = crate::store::ai_sort_age_cutoff(now);
    let mut released = 0;
    for (entry, account, added_at, source) in entries {
        let members = meron_entry_members(&tx, &entry)?;
        let mut by_ai = source == "ai";
        if !by_ai {
            for member in &members {
                let identity: Option<String> = identity_of
                    .query_row(params![account, member.folder, member.uid], |row| {
                        row.get(0)
                    })
                    .optional()?;
                if let Some(identity) = identity
                    && done_near.exists(params![
                        account,
                        identity,
                        added_at - AI_FILED_WINDOW_SECS,
                        added_at + AI_FILED_WINDOW_SECS
                    ])?
                {
                    by_ai = true;
                    break;
                }
            }
            if by_ai {
                tx.execute(
                    "UPDATE meron_folder_entries SET source = 'ai' WHERE id = ?1",
                    params![entry],
                )?;
            }
        }
        if !by_ai {
            continue;
        }
        let newest = members
            .iter()
            .map(|member| member.date)
            // Future dates count: a clock-skewed message keeps its entry.
            .filter(|date| *date > 0)
            .max();
        let in_inbox = members
            .iter()
            .any(|member| member.folder.eq_ignore_ascii_case("INBOX"));
        if newest.is_some_and(|newest| newest < cutoff) && !in_inbox {
            released += meron_release_entry_in(&tx, &entry)?.released;
        }
    }
    drop(identity_of);
    drop(done_near);
    crate::store::db::meta_set(&tx, OLD_AI_ENTRIES_DONE_KEY, "1")?;
    tx.commit()?;
    Ok(released)
}

/// Take one entry out of its Meron folder: the entry and all its aliases and
/// UIDs go. Inside a transaction the caller holds.
pub fn meron_release_entry_in(conn: &Connection, entry_id: &str) -> Result<MeronReleased> {
    let mut touched = BTreeSet::new();
    for member in meron_entry_members(conn, entry_id)? {
        touched.insert((member.account, member.folder));
    }
    let removed = conn.execute(
        "DELETE FROM meron_folder_entries WHERE id = ?1",
        params![entry_id],
    )?;
    conn.execute(
        "DELETE FROM meron_folder_aliases WHERE entry_id = ?1",
        params![entry_id],
    )?;
    conn.execute(
        "DELETE FROM meron_folder_uids WHERE entry_id = ?1",
        params![entry_id],
    )?;
    Ok(MeronReleased {
        released: removed,
        touched,
    })
}

/// [`meron_release_entry_in`] in its own transaction.
pub fn meron_release_entry(conn: &Connection, entry_id: &str) -> Result<MeronReleased> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let released = meron_release_entry_in(&tx, entry_id)?;
    tx.commit()?;
    Ok(released)
}

/// The Meron folder an entry is in, `None` once the entry is gone.
pub fn meron_entry_folder_id(conn: &Connection, entry_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT folder_id FROM meron_folder_entries WHERE id = ?1",
            params![entry_id],
            |row| row.get(0),
        )
        .optional()?)
}

fn pending_releases_in(conn: &Connection) -> Result<BTreeMap<String, String>> {
    Ok(crate::store::db::meta_get(conn, PENDING_RELEASES_KEY)?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}

/// Remember releases that failed (entry id to its folder then), for
/// [`meron_retry_pending_releases`].
pub fn meron_queue_releases(conn: &Connection, releases: &BTreeMap<String, String>) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut pending = pending_releases_in(&tx)?;
    pending.extend(
        releases
            .iter()
            .map(|(entry, folder)| (entry.clone(), folder.clone())),
    );
    crate::store::db::meta_set(&tx, PENDING_RELEASES_KEY, &serde_json::to_string(&pending)?)?;
    tx.commit()?;
    Ok(())
}

/// Release the entries [`meron_queue_releases`] remembered. An entry the user
/// has since moved to another Meron folder (or that is gone) is left alone:
/// the release was for where it was then. Returns how many were released.
pub fn meron_retry_pending_releases(conn: &Connection) -> Result<usize> {
    // The common case, nothing queued, without taking the write lock.
    if crate::store::db::meta_get(conn, PENDING_RELEASES_KEY)?.is_none() {
        return Ok(0);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut released = 0;
    for (entry, folder) in pending_releases_in(&tx)? {
        if meron_entry_folder_id(&tx, &entry)?.as_deref() == Some(folder.as_str()) {
            released += meron_release_entry_in(&tx, &entry)?.released;
        }
    }
    tx.execute(
        "DELETE FROM meta WHERE key = ?1",
        params![PENDING_RELEASES_KEY],
    )?;
    tx.commit()?;
    Ok(released)
}

/// Take the conversation a thread id names out of its Meron folder, if it is
/// in one.
pub fn meron_release_thread(
    conn: &Connection,
    account: &str,
    folder: &str,
    root: &str,
    uid: Option<u32>,
) -> Result<MeronReleased> {
    match meron_entry_of_thread(conn, account, folder, root, uid)? {
        Some((entry, _)) => meron_release_entry(conn, &entry),
        None => Ok(MeronReleased::default()),
    }
}
