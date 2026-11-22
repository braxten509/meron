//! Resolves which cached messages belong to each local folder entry.
//! Folder lists and unread counts use the same stored aliases and remembered UIDs.
//! The membership queries also filter ordinary IMAP folder pages.

use super::*;

/// The member rows of the entries `filter` (a condition on `e`, the entry) picks: a UNION of the two indexed joins from the aliases into `messages`.
///
/// `+al.key` strips the column's TEXT affinity: compared with it as is, the expression side would
/// take that affinity, and SQLite only uses an expression index (which has none) for a comparison
/// without one, so it scanned `messages`. CROSS JOIN keeps the order entries → aliases → mail.
fn members(
    conn: &Connection,
    filter: &str,
    args: &[&dyn rusqlite::ToSql],
    unseen_only: bool,
) -> Result<Vec<MeronMember>> {
    let seen = if unseen_only { " AND m.seen = 0" } else { "" };
    let columns = format!(
        "e.folder_id, e.id, m.account, m.folder, m.uid, m.date, m.seen, {}",
        meron_tag_key_sql("m")
    );
    let sql = format!(
        "SELECT {columns}
           FROM meron_folder_entries e
           CROSS JOIN meron_folder_aliases al
           CROSS JOIN messages m
          WHERE {filter} AND al.entry_id = e.id
            AND m.account = al.account AND {root} = +al.key AND m.uid <> 0{seen}
         UNION
         SELECT {columns}
           FROM meron_folder_entries e
           CROSS JOIN meron_folder_aliases al
           CROSS JOIN messages m
          WHERE {filter} AND al.entry_id = e.id
            AND m.account = al.account AND m.uid <> 0 AND {tag} = +al.key{seen}",
        root = meron_root_key_sql("m"),
        tag = meron_tag_key_sql("m"),
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(args, |row| {
        Ok(MeronMember {
            folder_id: row.get(0)?,
            entry_id: row.get(1)?,
            account: row.get(2)?,
            folder: row.get(3)?,
            uid: row.get(4)?,
            date: row.get(5)?,
            seen: row.get::<_, i64>(6)? != 0,
            tag_key: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The cached messages of every conversation in Meron folder `folder_id`.
pub fn meron_folder_members(conn: &Connection, folder_id: &str) -> Result<Vec<MeronMember>> {
    members(conn, "e.folder_id = ?1", &[&folder_id], false)
}

/// The cached messages of one entry.
pub fn meron_entry_members(conn: &Connection, entry_id: &str) -> Result<Vec<MeronMember>> {
    members(conn, "e.id = ?1", &[&entry_id], false)
}

/// Every conversation of Meron folder `folder_id` with something to show,
/// newest first (ties by entry id, descending), for keyset paging.
pub fn meron_folder_entries(conn: &Connection, folder_id: &str) -> Result<Vec<MeronEntryRows>> {
    let mut roles = Roles::default();
    let mut by_entry: BTreeMap<String, MeronEntryRows> = BTreeMap::new();
    for member in meron_folder_members(conn, folder_id)? {
        if !roles.hides(conn, &member.account, &member.folder)? {
            continue;
        }
        let entry = by_entry
            .entry(member.entry_id.clone())
            .or_insert_with(|| MeronEntryRows {
                entry_id: member.entry_id.clone(),
                account: member.account.clone(),
                newest: i64::MIN,
                rows: Vec::new(),
            });
        entry.newest = entry.newest.max(member.date);
        if !entry.rows.contains(&(member.folder.clone(), member.uid)) {
            entry.rows.push((member.folder, member.uid));
        }
    }
    let mut out: Vec<MeronEntryRows> = by_entry.into_values().collect();
    out.sort_by(|a, b| {
        b.newest
            .cmp(&a.newest)
            .then_with(|| b.entry_id.cmp(&a.entry_id))
    });
    Ok(out)
}

/// The cached header of one message, its folder filled in.
pub fn meron_row_header(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
) -> Result<Option<MessageHeader>> {
    let header = conn
        .prepare_cached(
            "SELECT uid, subject, from_name, from_addr, date, seen, starred, thread_key,
                    json_extract(json, '$.to') FROM messages
              WHERE account = ?1 AND folder = ?2 AND uid = ?3",
        )?
        .query_row(
            params![account, folder, uid],
            crate::store::message_header_from_row,
        )
        .optional()?;
    Ok(header.map(|mut header| {
        header.folder = folder.to_string();
        header
    }))
}

/// Unread messages per Meron folder: distinct messages (a copy in two folders
/// counts once) not seen, outside Drafts, Trash and Junk, plus server-unseen
/// mail the cache doesn't hold whose UID is known to be a member.
pub fn meron_unread_by_folder(conn: &Connection) -> Result<HashMap<String, u32>> {
    let mut roles = Roles::default();
    let mut unread: HashSet<(String, String, String)> = HashSet::new();
    for member in members(conn, "1 = 1", &[], true)? {
        if roles.hides(conn, &member.account, &member.folder)? {
            unread.insert((member.folder_id, member.account, member.tag_key));
        }
    }
    // Uncached copies count by the identity remembered with their UID, so a
    // message unread here in one folder and uncached in another counts once.
    let mut stmt = conn.prepare(
        "SELECT e.folder_id, u.account, u.folder, u.uid, x.tag_key
           FROM uncached_unseen u
           JOIN meron_folder_uids x
             ON x.account = u.account AND x.folder = u.folder AND x.uid = u.uid
           JOIN meron_folder_entries e ON e.id = x.entry_id",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u32>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (folder_id, account, folder, uid, tag_key) in rows {
        if roles.hides(conn, &account, &folder)? {
            // Rows remembered before identities were kept fall back to the
            // location key, which is what a message without a Message-ID has.
            let identity = if tag_key.is_empty() {
                format!("uid:{folder}:{uid}")
            } else {
                tag_key
            };
            unread.insert((folder_id, account, identity));
        }
    }
    let mut out = HashMap::new();
    for (folder_id, _, _) in unread {
        *out.entry(folder_id).or_insert(0) += 1;
    }
    Ok(out)
}

/// The unseen UIDs of Meron folder `folder_id`, per account and mailbox, cached
/// or not: what its "Mark all read" sets `\Seen` on.
pub fn meron_folder_unseen_uids(
    conn: &Connection,
    folder_id: &str,
) -> Result<BTreeMap<String, BTreeMap<String, Vec<u32>>>> {
    let mut roles = Roles::default();
    let mut out: BTreeMap<String, BTreeMap<String, BTreeSet<u32>>> = BTreeMap::new();
    for member in members(conn, "e.folder_id = ?1", &[&folder_id], true)? {
        if roles.hides(conn, &member.account, &member.folder)? {
            out.entry(member.account)
                .or_default()
                .entry(member.folder)
                .or_default()
                .insert(member.uid);
        }
    }
    let mut stmt = conn.prepare(
        "SELECT u.account, u.folder, u.uid
           FROM uncached_unseen u
           JOIN meron_folder_uids x
             ON x.account = u.account AND x.folder = u.folder AND x.uid = u.uid
           JOIN meron_folder_entries e ON e.id = x.entry_id
          WHERE e.folder_id = ?1",
    )?;
    let rows = stmt
        .query_map(params![folder_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (account, folder, uid) in rows {
        if roles.hides(conn, &account, &folder)? {
            out.entry(account)
                .or_default()
                .entry(folder)
                .or_default()
                .insert(uid);
        }
    }
    Ok(out
        .into_iter()
        .map(|(account, folders)| {
            (
                account,
                folders
                    .into_iter()
                    .map(|(folder, uids)| (folder, uids.into_iter().collect()))
                    .collect(),
            )
        })
        .collect())
}

/// After the server took `\Seen` for `uids` of `folder`: the store follows.
pub fn meron_mark_seen(conn: &Connection, account: &str, folder: &str, uids: &[u32]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for uid in uids {
        tx.execute(
            "UPDATE messages SET seen = 1 WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid],
        )?;
        tx.execute(
            "DELETE FROM uncached_unseen WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// The entry and Meron folder the cached message `(folder, uid)` is in.
pub fn meron_entry_of_row(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
) -> Result<Option<(String, String)>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT e.id, e.folder_id FROM messages m
               CROSS JOIN meron_folder_aliases al
               CROSS JOIN meron_folder_entries e
              WHERE m.account = ?1 AND m.folder = ?2 AND m.uid = ?3
                AND al.account = m.account AND al.key = {root} AND e.id = al.entry_id
             UNION ALL
             SELECT e.id, e.folder_id FROM messages m
               CROSS JOIN meron_folder_aliases al
               CROSS JOIN meron_folder_entries e
              WHERE m.account = ?1 AND m.folder = ?2 AND m.uid = ?3
                AND al.account = m.account AND al.key = {tag} AND e.id = al.entry_id
             LIMIT 1",
            root = meron_root_key_sql("m"),
            tag = meron_tag_key_sql("m"),
        ))?
        .query_row(params![account, folder, uid], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?)
}

/// The entry and Meron folder an alias key belongs to.
pub fn meron_entry_of_key(
    conn: &Connection,
    account: &str,
    key: &str,
) -> Result<Option<(String, String)>> {
    Ok(conn
        .prepare_cached(
            "SELECT e.id, e.folder_id FROM meron_folder_aliases al
               JOIN meron_folder_entries e ON e.id = al.entry_id
              WHERE al.account = ?1 AND al.key = ?2",
        )?
        .query_row(params![account, normalize_key(key)], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?)
}

/// The entry a thread id's conversation is in, if any: found by its key, else
/// through any of its cached messages (one whose Message-ID was aliased before
/// a reconciliation re-keyed it).
pub fn meron_entry_of_thread(
    conn: &Connection,
    account: &str,
    folder: &str,
    root: &str,
    uid: Option<u32>,
) -> Result<Option<(String, String)>> {
    let Some(key) = meron_thread_key(conn, account, folder, root, uid)? else {
        return Ok(None);
    };
    if let Some(found) = meron_entry_of_key(conn, account, &key)? {
        return Ok(Some(found));
    }
    let rows: Vec<(String, u32)> = conn
        .prepare_cached(
            "SELECT folder, uid FROM messages
              WHERE account = ?1 AND lower(COALESCE(thread_key, '')) = ?2 AND uid <> 0",
        )?
        .query_map(params![account, key], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (folder, uid) in rows {
        if let Some(found) = meron_entry_of_row(conn, account, &folder, uid)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// The Meron folder of each `(folder, uid)`, for the cards' `meron_folder_id`.
pub fn meron_folder_of_rows(
    conn: &Connection,
    account: &str,
    rows: &[(String, u32)],
) -> Result<HashMap<(String, u32), String>> {
    let mut out = HashMap::new();
    if rows.is_empty() || !meron_account_has_entries(conn, account)? {
        return Ok(out);
    }
    for (folder, uid) in rows {
        if let Some((_, folder_id)) = meron_entry_of_row(conn, account, folder, *uid)? {
            out.insert((folder.clone(), *uid), folder_id);
        }
    }
    Ok(out)
}

/// Of `uids` in `folder`, those in a Meron folder: a cached row whose key is
/// an alias, or a UID remembered as a member.
pub fn meron_foldered_uids(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> Result<HashSet<u32>> {
    let mut out = HashSet::new();
    if uids.is_empty() || !meron_account_has_entries(conn, account)? {
        return Ok(out);
    }
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT 1 FROM messages m
          WHERE m.account = ?1 AND m.folder = ?2 AND m.uid = ?3 AND {hidden}
         UNION ALL
         SELECT 1 FROM meron_folder_uids
          WHERE account = ?1 AND folder = ?2 AND uid = ?3
         LIMIT 1",
        hidden = meron_hidden_sql("m"),
    ))?;
    for uid in uids {
        if stmt.exists(params![account, folder, uid])? {
            out.insert(*uid);
        }
    }
    Ok(out)
}

/// Whether the cached message `(folder, uid)` is in a Meron folder.
pub fn meron_uid_in_folder(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
) -> Result<bool> {
    Ok(meron_foldered_uids(conn, account, folder, &[uid])?.contains(&uid))
}

/// Whether a header read from the server, not stored yet, belongs to a
/// conversation in a Meron folder: by its root key (as read, and as the cache
/// resolves it), its Message-ID or the message it replies to.
pub fn meron_header_in_folder(
    conn: &Connection,
    account: &str,
    header: &MessageHeader,
) -> Result<bool> {
    if !meron_account_has_entries(conn, account)? {
        return Ok(false);
    }
    let mut keys: Vec<String> = Vec::new();
    let root = header.thread_key.trim().to_lowercase();
    if is_alias_key(&root) {
        if !root.starts_with("gmthrid:") {
            let resolved: Option<String> = conn
                .prepare_cached(
                    "SELECT lower(thread_key) FROM messages
                      WHERE account = ?1
                        AND lower(COALESCE(json_extract(json, '$.message_id'), '')) = ?2
                        AND COALESCE(thread_key, '') <> ''
                      ORDER BY date ASC, uid ASC LIMIT 1",
                )?
                .query_row(params![account, root], |row| row.get(0))
                .optional()?;
            keys.extend(resolved.filter(|key| is_alias_key(key)));
        }
        keys.push(root);
    }
    for id in [&header.message_id, &header.in_reply_to] {
        let raw = id.trim().to_lowercase();
        let tag = raw
            .trim_matches(|c| c == ' ' || c == '<' || c == '>')
            .to_string();
        for key in [raw, tag] {
            if is_alias_key(&key) {
                keys.push(key);
            }
        }
    }
    let mut stmt =
        conn.prepare_cached("SELECT 1 FROM meron_folder_aliases WHERE account = ?1 AND key = ?2")?;
    for key in keys {
        if stmt.exists(params![account, key])? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Drop from `headers` (each carrying its folder) the mail of conversations in
/// a Meron folder, where its folder hides them. For lists built from server
/// reads, like the desktop's live per-folder Starred filter.
pub fn retain_outside_meron_folders(
    conn: &Connection,
    account: &str,
    headers: &mut Vec<MessageHeader>,
) -> Result<()> {
    if headers.is_empty() || !meron_account_has_entries(conn, account)? {
        return Ok(());
    }
    let mut by_folder: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for header in headers.iter() {
        by_folder
            .entry(header.folder.clone())
            .or_default()
            .push(header.uid);
    }
    let mut hidden: HashSet<(String, u32)> = HashSet::new();
    for (folder, uids) in by_folder {
        if !meron_hides_folder(conn, account, &folder)? {
            continue;
        }
        for uid in meron_foldered_uids(conn, account, &folder, &uids)? {
            hidden.insert((folder.clone(), uid));
        }
    }
    let mut keep = Vec::with_capacity(headers.len());
    for header in std::mem::take(headers) {
        let gone = hidden.contains(&(header.folder.clone(), header.uid))
            || (meron_hides_folder(conn, account, &header.folder)?
                && meron_header_in_folder(conn, account, &header)?);
        if !gone {
            keep.push(header);
        }
    }
    *headers = keep;
    Ok(())
}
