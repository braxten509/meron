//! Loads the headers that make up a conversation across mailboxes.
//! The reader asks for a complete thread or restricts the lookup to one folder.
//! Reference gaps identify ancestors the synchronization engine still needs to fetch.

use super::*;

/// Message-IDs a thread references but hasn't cached locally.
///
/// Across every cached row of `account` sharing `thread_key`, collect the union of ids they
/// reference (each row's `References` chain) plus the root id (`thread_key` itself), then subtract
/// the ids already present as cached messages. The remainder is the ancestry the thread links to
/// but that lies outside the synced window — the on-demand fetch target.
///
/// All ids are lowercased here so the set comparison is case-insensitive; stored ids preserve the
/// original header casing.
pub fn get_thread_reference_gaps(
    conn: &Connection,
    account: &str,
    thread_key: &str,
) -> Result<Vec<String>> {
    // A sender row names a person, not a conversation root.
    if thread_key.starts_with(crate::store::SENDER_KEY_PREFIX) {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT json FROM messages
         WHERE account = ?1 AND uid <> 0
           AND COALESCE(NULLIF(thread_key, ''), 'uid:' || uid) = ?2",
    )?;
    let rows = stmt.query_map(params![account, thread_key], |row| {
        row.get::<_, Option<String>>(0)
    })?;
    let mut referenced: std::collections::BTreeSet<String> = Default::default();
    let mut present: std::collections::HashSet<String> = Default::default();
    let normalize = |id: &str| {
        id.trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim()
            .to_ascii_lowercase()
    };
    for row in rows {
        let json = row?.unwrap_or_default();
        let parsed: serde_json::Value =
            serde_json::from_str(&json).unwrap_or(serde_json::Value::Null);
        if let Some(mid) = parsed.get("message_id").and_then(|v| v.as_str()) {
            let mid = normalize(mid);
            if !mid.is_empty() {
                present.insert(mid);
            }
        }
        if let Some(refs) = parsed.get("references").and_then(|v| v.as_str()) {
            for id in refs.split_whitespace() {
                let id = normalize(id);
                if !id.is_empty() {
                    referenced.insert(id);
                }
            }
        }
    }
    // The non-synthetic `thread_key` is the thread's root Message-ID.
    if !thread_key.starts_with("uid:") {
        let root = normalize(thread_key);
        if !root.is_empty() {
            referenced.insert(root);
        }
    }
    Ok(referenced
        .into_iter()
        .filter(|id| !present.contains(id))
        .collect())
}

/// Like `get_thread_headers`, but spans any folder of `account` whose row shares the `thread_key`.
///
/// Used by the cross-folder thread view so a reply stored in Sent appears alongside the inbox
/// messages it threads with. Each header carries its source `folder` so the reader can fetch the
/// body from the right mailbox. Ordered by `date` ASC (UIDs are folder-scoped, so they aren't
/// comparable across folders).
///
/// A self-addressed message is delivered to both Sent and the Inbox, so the same RFC `Message-ID`
/// lands as two rows under one `thread_key` (distinct per-folder UIDs sidestep the
/// `UNIQUE(account, folder, msg_id)` constraint). We collapse those to a single bubble by keeping,
/// per non-empty `Message-ID`, the unread copy if any, otherwise the lowest-`id` row.
///
/// Rows without a `Message-ID` (e.g. drafts) are never collapsed.
///
/// This is every cached copy, thrown-away mail included; a conversation on screen is read with
/// [`get_thread_headers_from_folder`].
pub fn get_thread_headers_all_folders(
    conn: &Connection,
    account: &str,
    thread_key: &str,
) -> Result<Vec<MessageHeader>> {
    thread_headers_across_folders(conn, account, thread_key, None)
}

/// [`get_thread_headers_all_folders`] as a conversation read from `folder`
/// shows it: without the mail a [`ChatScope`] leaves out (copies in Trash or
/// Junk, and the reader's own replies to a conversation they deleted).
pub fn get_thread_headers_from_folder(
    conn: &Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
) -> Result<Vec<MessageHeader>> {
    let scope = ChatScope::for_folder(conn, account, folder)?;
    thread_headers_across_folders(conn, account, thread_key, Some(&scope))
}

fn thread_headers_across_folders(
    conn: &Connection,
    account: &str,
    thread_key: &str,
    scope: Option<&ChatScope>,
) -> Result<Vec<MessageHeader>> {
    // `date` is an epoch integer, so the ORDER BY sorts chronologically.
    // Unknown dates (0) sort first.
    let mut stmt = conn.prepare(&format!(
        "SELECT uid, folder, subject, from_name, from_addr, date, seen, starred, thread_key,
                json_extract(json, '$.in_reply_to'),
                id, COALESCE(json_extract(json, '$.message_id'), ''), {TAG_KEY_SQL}
         FROM messages
         WHERE account = ?1 AND uid <> 0
           AND COALESCE(NULLIF(thread_key, ''), 'uid:' || uid) = ?2
         ORDER BY date ASC, uid ASC"
    ))?;
    let rows = stmt.query_map(params![account, thread_key], |row| {
        let uid: u32 = row.get(0)?;
        Ok((
            MessageHeader {
                uid,
                folder: row.get(1)?,
                subject: row.get(2)?,
                from_name: row.get(3)?,
                from_addr: row.get(4)?,
                date: row.get(5)?,
                seen: row.get::<_, i64>(6)? != 0,
                starred: row.get::<_, i64>(7)? != 0,
                thread_key: row
                    .get::<_, Option<String>>(8)?
                    .filter(|key| !key.is_empty())
                    .unwrap_or_else(|| format!("uid:{uid}")),
                in_reply_to: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
                ..Default::default()
            },
            row.get::<_, i64>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, String>(12)?,
        ))
    })?;
    // Left-out copies go first, so one never stands in for a shown copy of the
    // same Message-ID.
    let mut shown: Vec<(MessageHeader, i64, String)> = Vec::new();
    for row in rows {
        let (header, id, message_id, tag_key) = row?;
        if scope.is_some_and(|scope| scope.leaves_out(&header.folder, &tag_key)) {
            continue;
        }
        shown.push((header, id, message_id));
    }
    // Per Message-ID: the unread copy if any, otherwise the lowest row id.
    let mut kept: std::collections::HashMap<&str, (bool, i64)> = Default::default();
    for (header, id, message_id) in &shown {
        if message_id.is_empty() {
            continue;
        }
        let rank = (header.seen, *id);
        kept.entry(message_id.as_str())
            .and_modify(|best| *best = (*best).min(rank))
            .or_insert(rank);
    }
    let keep: Vec<bool> = shown
        .iter()
        .map(|(header, id, message_id)| {
            message_id.is_empty() || kept[message_id.as_str()] == (header.seen, *id)
        })
        .collect();
    Ok(shown
        .into_iter()
        .zip(keep)
        .filter_map(|((header, _, _), keep)| keep.then_some(header))
        .collect())
}
