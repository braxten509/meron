//! Tracks mail tags across copies of the same message.
//! Baseline and pending rows distinguish observed server state from queued updates.
//! A sync pass records each push before clearing work that has settled.

use super::*;

pub(super) fn other_copy_has_tag(
    conn: &Connection,
    account: &str,
    key: &str,
    folder: &str,
    uid: u32,
    tag: &str,
) -> Result<bool> {
    Ok(other_copies(conn, account, key, folder, uid)?
        .iter()
        .any(|server_tags| parse_server_tags(server_tags.as_deref()).contains(tag)))
}

pub(super) fn other_copy_lacks_tag(
    conn: &Connection,
    account: &str,
    key: &str,
    folder: &str,
    uid: u32,
    tag: &str,
) -> Result<bool> {
    Ok(other_copies(conn, account, key, folder, uid)?
        .iter()
        .any(|server_tags| !parse_server_tags(server_tags.as_deref()).contains(tag)))
}

/// What the message's other mailbox copies last showed.
fn other_copies(
    conn: &Connection,
    account: &str,
    key: &str,
    folder: &str,
    uid: u32,
) -> Result<Vec<Option<String>>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT server_tags FROM messages
         WHERE account = ?1 AND uid <> 0 AND {TAG_KEY_SQL} = ?2
           AND NOT (folder = ?3 AND uid = ?4)"
    ))?;
    let rows = stmt.query_map(params![account, key, folder, uid], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Whether a mailbox holds mail whose keywords were never read, as all of it
/// was when tags arrived.
pub fn needs_tag_baseline(conn: &Connection, account: &str, folder: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages
                       WHERE account = ?1 AND folder = ?2 AND uid <> 0 AND server_tags IS NULL)",
        params![account, folder],
        |row| row.get(0),
    )?)
}

/// A full read of the mailbox is done. Rows it did not cover are gone from the
/// server, and must not ask for another one on every sync.
pub fn mark_tag_baseline(conn: &Connection, account: &str, folder: &str) -> Result<()> {
    conn.execute(
        "UPDATE messages SET server_tags = ''
         WHERE account = ?1 AND folder = ?2 AND uid <> 0 AND server_tags IS NULL",
        params![account, folder],
    )?;
    Ok(())
}

/// Forget tags keyed by location in a mailbox whose UIDs were reset: those UIDs
/// now name different messages.
pub fn clear_location_tags(conn: &Connection, account: &str, folder: &str) -> Result<()> {
    let prefix = format!("uid:{folder}:");
    conn.execute(
        "DELETE FROM message_tags
         WHERE account = ?1 AND substr(tag_key, 1, length(?2)) = ?2",
        params![account, prefix],
    )?;
    Ok(())
}

/// Whether the account has any tag at all, which most never will: the card
/// pages can then skip reading them.
pub(in crate::store) fn has_tags(conn: &Connection, account: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM message_tags WHERE account = ?1)",
        params![account],
        |row| row.get(0),
    )?)
}

/// Whether anything is waiting to reach the server.
pub fn has_pending_tags(conn: &Connection, account: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM message_tags WHERE account = ?1 AND state IN (1, 2))",
        params![account],
        |row| row.get(0),
    )?)
}

fn pending_tags(conn: &Connection, account: &str) -> Result<Vec<PendingTag>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT t.tag_key, t.tag, t.state, m.folder, m.uid, m.server_tags
         FROM message_tags t
         LEFT JOIN messages m ON m.account = t.account AND m.uid <> 0
              AND {} = t.tag_key
         WHERE t.account = ?1 AND t.state IN ({PENDING_ADD}, {PENDING_REMOVE})
         ORDER BY t.tag_key, t.tag",
        qualified_tag_key("m"),
    ))?;
    let rows = stmt.query_map(params![account], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<u32>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut pending: Vec<PendingTag> = Vec::new();
    for row in rows {
        let (key, tag, state, folder, uid, server_tags) = row?;
        if pending
            .last()
            .is_none_or(|last| last.key != key || last.tag != tag)
        {
            pending.push(PendingTag {
                key,
                tag,
                state,
                copies: Vec::new(),
            });
        }
        if let (Some(folder), Some(uid), Some(entry)) = (folder, uid, pending.last_mut()) {
            entry.copies.push((folder, uid, server_tags));
        }
    }
    Ok(pending)
}

/// [`TAG_KEY_SQL`] with its columns qualified, for joins.
pub(in crate::store) fn qualified_tag_key(alias: &str) -> String {
    TAG_KEY_SQL
        .replace("json_extract(json", &format!("json_extract({alias}.json"))
        .replace("|| folder ||", &format!("|| {alias}.folder ||"))
        .replace("|| uid)", &format!("|| {alias}.uid)"))
}

/// Whether a copy still needs a write for this pending tag. A copy never looked
/// at might hold a keyword being removed, so it is written to rather than
/// assumed clean.
fn copy_needs_push(state: i64, tag: &str, server_tags: Option<&str>) -> bool {
    let has = parse_server_tags(server_tags).contains(tag);
    if state == PENDING_ADD {
        !has
    } else {
        has || server_tags.is_none()
    }
}

/// The keyword writes that would bring the server up to date, per mailbox.
pub fn pending_tag_pushes(conn: &Connection, account: &str) -> Result<Vec<TagPush>> {
    let mut grouped: BTreeMap<(String, String, bool), BTreeSet<u32>> = BTreeMap::new();
    for pending in pending_tags(conn, account)? {
        for (folder, uid, server_tags) in pending.copies {
            if copy_needs_push(pending.state, &pending.tag, server_tags.as_deref()) {
                grouped
                    .entry((folder, pending.tag.clone(), pending.state == PENDING_ADD))
                    .or_default()
                    .insert(uid);
            }
        }
    }
    Ok(grouped
        .into_iter()
        .map(|((folder, tag, add), uids)| TagPush {
            folder,
            tag,
            add,
            uids: uids.into_iter().collect(),
        })
        .collect())
}

/// Record a keyword write the server accepted on `push`'s copies.
pub fn record_tag_push(conn: &Connection, account: &str, push: &TagPush) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for uid in &push.uids {
        let current: Option<Option<String>> = tx
            .prepare_cached(
                "SELECT server_tags FROM messages
                 WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            )?
            .query_row(params![account, push.folder, uid], |row| row.get(0))
            .optional()?;
        let Some(current) = current else { continue };
        let mut tags = parse_server_tags(current.as_deref());
        if push.add {
            tags.insert(push.tag.clone());
        } else {
            tags.remove(&push.tag);
        }
        set_server_tags(&tx, account, &push.folder, *uid, &tags)?;
    }
    tx.commit()?;
    Ok(())
}

/// Close out pending tags whose every copy now agrees. `refused` lists the
/// (mailbox, tag) pairs that turned a keyword down: those copies have no say in
/// an add, which stays pending while they are all that is left of it, in case
/// the mailbox learns keywords.
pub fn settle_pending_tags(
    conn: &Connection,
    account: &str,
    refused: &HashSet<(String, String)>,
) -> Result<()> {
    for pending in pending_tags(conn, account)? {
        let adding = pending.state == PENDING_ADD;
        let copies: Vec<_> = pending
            .copies
            .iter()
            .filter(|(folder, _, _)| {
                !(adding && refused.contains(&(folder.clone(), pending.tag.clone())))
            })
            .collect();
        let outstanding = copies.iter().any(|(_, _, server_tags)| {
            copy_needs_push(pending.state, &pending.tag, server_tags.as_deref())
        });
        if outstanding || copies.is_empty() {
            continue;
        }
        let settled = if adding { SYNCED } else { REMOVED };
        set_tag_state(conn, account, &pending.key, &pending.tag, settled)?;
    }
    conn.execute(
        "DELETE FROM message_tags WHERE account = ?1 AND state = ?2 AND updated_at < ?3",
        params![account, REMOVED, now_unix() - STALE_ECHO_SECS],
    )?;
    Ok(())
}
