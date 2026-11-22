//! Files conversations into local folders inside SQLite transactions.
//! Root keys, tag aliases and remembered UIDs keep the same entry across folder copies.
//! Callers can file a conversation directly or as part of a larger transaction.

use super::*;

fn rows_by_root(conn: &Connection, account: &str, root: &str) -> Result<Vec<KeyedRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT id, folder, uid, lower(COALESCE(thread_key, '')), {TAG_KEY_SQL}, COALESCE(date, 0)
           FROM messages
          WHERE account = ?1 AND lower(COALESCE(thread_key, '')) = ?2 AND uid <> 0"
    ))?;
    let rows = stmt.query_map(params![account, root], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn rows_by_tag(conn: &Connection, account: &str, tag: &str) -> Result<Vec<KeyedRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT id, folder, uid, lower(COALESCE(thread_key, '')), {TAG_KEY_SQL}, COALESCE(date, 0)
           FROM messages
          WHERE account = ?1 AND uid <> 0 AND {TAG_KEY_SQL} = ?2"
    ))?;
    let rows = stmt.query_map(params![account, tag], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `(entry id, added_at)` of the entry an alias key belongs to.
pub(super) fn entry_of_alias(
    conn: &Connection,
    account: &str,
    key: &str,
) -> Result<Option<(String, i64)>> {
    Ok(conn
        .prepare_cached(
            "SELECT e.id, e.added_at FROM meron_folder_aliases al
               JOIN meron_folder_entries e ON e.id = al.entry_id
              WHERE al.account = ?1 AND al.key = ?2",
        )?
        .query_row(params![account, key], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?)
}

/// Fold entry `from` into `into`: its aliases and UIDs move, it goes.
pub(super) fn merge_entry(conn: &Connection, from: &str, into: &str) -> Result<()> {
    if from == into {
        return Ok(());
    }
    conn.execute(
        "UPDATE meron_folder_aliases SET entry_id = ?2 WHERE entry_id = ?1",
        params![from, into],
    )?;
    conn.execute(
        "UPDATE meron_folder_uids SET entry_id = ?2 WHERE entry_id = ?1",
        params![from, into],
    )?;
    conn.execute(
        "DELETE FROM meron_folder_entries WHERE id = ?1",
        params![from],
    )?;
    Ok(())
}

pub(super) fn put_alias(conn: &Connection, account: &str, key: &str, entry: &str) -> Result<()> {
    if !is_alias_key(key) {
        return Ok(());
    }
    conn.prepare_cached(
        "INSERT INTO meron_folder_aliases(account, key, entry_id) VALUES(?1, ?2, ?3)
         ON CONFLICT(account, key) DO UPDATE SET entry_id = excluded.entry_id",
    )?
    .execute(params![account, key, entry])?;
    Ok(())
}

/// Remember a member's UID with its tag key (its identity across copies), so
/// unread counts can tell two copies of one message apart from two messages
/// once the cache no longer holds the row.
pub(super) fn put_uid(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
    tag_key: &str,
    entry: &str,
) -> Result<()> {
    let validity = crate::store::get_folder_state(conn, account, folder)?
        .map(|(validity, _)| validity)
        .unwrap_or(0);
    conn.prepare_cached(
        "INSERT OR REPLACE INTO meron_folder_uids(account, folder, uidvalidity, uid, entry_id, tag_key)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
    )?
    .execute(params![account, folder, validity, uid, entry, tag_key])?;
    Ok(())
}

/// The messages of the conversation `key` names, and its aliases: those under
/// the key as a root or as a message key, then the rest of every real root
/// those carry. `filing` may leave rows out (see [`MeronFiling::leaves_out`]);
/// the key itself is always an alias.
pub(super) fn conversation_rows(
    conn: &Connection,
    account: &str,
    key: &str,
    filing: MeronFiling<'_>,
) -> Result<(BTreeMap<i64, KeyedRow>, BTreeSet<String>)> {
    let mut rows: BTreeMap<i64, KeyedRow> = BTreeMap::new();
    for row in rows_by_root(conn, account, key)?
        .into_iter()
        .chain(rows_by_tag(conn, account, key)?)
    {
        if !filing.leaves_out(&row) {
            rows.insert(row.0, row);
        }
    }
    let roots: BTreeSet<String> = rows
        .values()
        .map(|row| row.3.clone())
        .filter(|root| is_alias_key(root) && *root != key)
        .collect();
    for root in &roots {
        for row in rows_by_root(conn, account, root)? {
            if !filing.leaves_out(&row) {
                rows.insert(row.0, row);
            }
        }
    }
    let mut aliases: BTreeSet<String> = BTreeSet::from([key.to_string()]);
    for row in rows.values() {
        aliases.insert(row.3.clone());
        aliases.insert(row.4.clone());
    }
    aliases.retain(|alias| is_alias_key(alias));
    Ok((rows, aliases))
}

/// Move the conversation `key` names (see [`meron_conversation_key`]) into
/// Meron folder `folder_id`, or from one Meron folder to another. Inside a
/// transaction the caller holds. A new entry records `filing`'s source; a
/// conversation the user files by hand becomes theirs.
pub fn meron_move_in_in(
    conn: &Connection,
    account: &str,
    key: &str,
    folder_id: &str,
    filing: MeronFiling<'_>,
) -> Result<MeronMovedIn> {
    if !meron_folder_exists(conn, folder_id)? {
        bail!("That Meron folder is gone.");
    }
    let key = normalize_key(key);
    if !is_alias_key(&key) {
        bail!("This conversation can't go in a Meron folder.");
    }
    let (rows, aliases) = conversation_rows(conn, account, &key, filing)?;

    // Already an entry (in this or another Meron folder): keep the newest,
    // fold the others into it.
    let mut found: Vec<(String, i64)> = Vec::new();
    for alias in &aliases {
        if let Some(entry) = entry_of_alias(conn, account, alias)?
            && !found.contains(&entry)
        {
            found.push(entry);
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    let (entry_id, previous_folder) = match found.first() {
        Some((winner, _)) => {
            for (other, _) in &found[1..] {
                merge_entry(conn, other, winner)?;
            }
            let previous: String = conn.query_row(
                "SELECT folder_id FROM meron_folder_entries WHERE id = ?1",
                params![winner],
                |row| row.get(0),
            )?;
            conn.execute(
                "UPDATE meron_folder_entries SET folder_id = ?2 WHERE id = ?1",
                params![winner, folder_id],
            )?;
            if filing == MeronFiling::User {
                conn.execute(
                    "UPDATE meron_folder_entries SET source = 'user' WHERE id = ?1",
                    params![winner],
                )?;
            }
            (winner.clone(), (previous != folder_id).then_some(previous))
        }
        None => {
            let id = format!("{MERON_ENTRY_ID_PREFIX}{}", uuid::Uuid::new_v4().simple());
            conn.execute(
                "INSERT INTO meron_folder_entries(id, account, folder_id, added_at, source)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    account,
                    folder_id,
                    crate::store::now_unix(),
                    filing.source()
                ],
            )?;
            (id, None)
        }
    };
    for alias in &aliases {
        put_alias(conn, account, alias, &entry_id)?;
    }
    let mut touched = BTreeSet::new();
    for (_, folder, uid, _, tag, _) in rows.values() {
        put_uid(conn, account, folder, *uid, tag, &entry_id)?;
        touched.insert(folder.clone());
    }
    // Filed by the user: mail the AI left shown in the Inbox no longer is, so
    // taking it out of the Meron folder later puts it back in its own folder.
    let located: BTreeSet<(String, u32)> = rows
        .values()
        .map(|(_, folder, uid, _, _, _)| (folder.clone(), *uid))
        .collect();
    crate::store::ai_sort_inbox_drop(conn, account, &located)?;
    Ok(MeronMovedIn {
        entry_id,
        previous_folder,
        touched,
    })
}

/// Whether the conversation `key` names has a cached Inbox copy.
pub fn meron_conversation_has_inbox_copy(
    conn: &Connection,
    account: &str,
    key: &str,
) -> Result<bool> {
    let key = normalize_key(key);
    if !is_alias_key(&key) {
        return Ok(false);
    }
    let (rows, _) = conversation_rows(conn, account, &key, MeronFiling::User)?;
    Ok(rows
        .values()
        .any(|(_, folder, _, _, _, _)| folder.eq_ignore_ascii_case("INBOX")))
}

/// The conversation `key` names' cached Inbox copies: (uid, date).
pub fn meron_conversation_inbox_rows(
    conn: &Connection,
    account: &str,
    key: &str,
) -> Result<Vec<(u32, i64)>> {
    let key = normalize_key(key);
    if !is_alias_key(&key) {
        return Ok(Vec::new());
    }
    let (rows, _) = conversation_rows(conn, account, &key, MeronFiling::User)?;
    Ok(rows
        .values()
        .filter(|(_, folder, _, _, _, _)| folder.eq_ignore_ascii_case("INBOX"))
        .map(|(_, _, uid, _, _, date)| (*uid, *date))
        .collect())
}

/// [`meron_move_in_in`] in its own transaction.
pub fn meron_move_in(
    conn: &Connection,
    account: &str,
    key: &str,
    folder_id: &str,
    filing: MeronFiling<'_>,
) -> Result<MeronMovedIn> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let moved = meron_move_in_in(&tx, account, key, folder_id, filing)?;
    tx.commit()?;
    Ok(moved)
}
