//! Resolves the cached messages addressed by a conversation or UID selection.
//! Reader and mutation requests share the same folder and thread-key lookup.
//! Draft keys, newest UIDs and per-folder membership come from the stored headers.

use super::*;

pub fn get_thread_headers(
    conn: &Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
) -> Result<Vec<MessageHeader>> {
    // A sender row acts on that person's messages in this folder only; their
    // Sent copies and drafts are in the chat but not in its actions.
    if let Some(addr) = sender_key_addr(thread_key) {
        let mut headers = sender_folder_headers(conn, account, folder, &addr)?;
        for header in &mut headers {
            header.folder = String::new();
        }
        return Ok(headers);
    }
    let mut stmt = conn.prepare(
        "SELECT uid, subject, from_name, from_addr, date, seen, starred, thread_key,
                json_extract(json, '$.in_reply_to') FROM messages
         WHERE account = ?1 AND folder = ?2 AND COALESCE(NULLIF(thread_key, ''), 'uid:' || uid) = ?3
         ORDER BY date ASC, uid ASC",
    )?;
    let rows = stmt.query_map(params![account, folder, thread_key], |row| {
        let uid = row.get(0)?;
        Ok(MessageHeader {
            uid,
            subject: row.get(1)?,
            from_name: row.get(2)?,
            from_addr: row.get(3)?,
            date: row.get(4)?,
            seen: row.get::<_, i64>(5)? != 0,
            starred: row.get::<_, i64>(6)? != 0,
            thread_key: row
                .get::<_, Option<String>>(7)?
                .filter(|key| !key.is_empty())
                .unwrap_or_else(|| format!("uid:{}", uid)),
            in_reply_to: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
            folder: String::new(),
            ..Default::default()
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The newest message of a thread (or of one subject branch of it), as a single-element uid list — empty when the thread has no cached messages.
///
/// Marking a whole thread unread uses this: the gesture means "bring this back to me", not "I read
/// none of these", so only the newest message carries the flag. The thread then shows one unread
/// message and opening it lands on that message, instead of reopening at the oldest one and
/// shedding the count again as the reader scrolls down.
pub fn newest_thread_uids(
    conn: &Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
    subject_filter: Option<&str>,
) -> Result<Vec<u32>> {
    // get_thread_headers orders by date ascending, so the newest is last.
    let newest = get_thread_headers(conn, account, folder, thread_key)?
        .into_iter()
        .filter(|header| match subject_filter {
            Some(filter) => thread_grouping_subject(&header.subject) == filter,
            None => true,
        })
        .next_back();
    Ok(newest.map(|header| header.uid).into_iter().collect())
}

/// A thread's messages grouped by mailbox, for a whole-thread "mark read".
///
/// Covers every folder the reader shows the thread from (see `thread_read`), not just the card's
/// own, so an unread copy filed elsewhere is not left behind a card that now reads as read. A
/// synthetic `uid:N` key stays folder-local: UIDs are only unique within one mailbox.
///
/// `unseen_only` narrows it to messages the store still has unread. Only a caller that updates the
/// store after the server write may use it: one that marks the store first would leave a retry of a
/// failed write nothing to send.
pub fn thread_uids_by_folder(
    conn: &Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
    subject_filter: Option<&str>,
    unseen_only: bool,
) -> Result<std::collections::BTreeMap<String, Vec<u32>>> {
    if let Some(addr) = sender_key_addr(thread_key) {
        return sender_uids_by_folder(conn, account, folder, &addr, unseen_only);
    }
    let only_folder = thread_key.starts_with("uid:").then_some(folder);
    let mut stmt = conn.prepare(
        "SELECT folder, uid, subject FROM messages
         WHERE account = ?1 AND uid <> 0 AND (?4 = 0 OR seen = 0)
           AND COALESCE(NULLIF(thread_key, ''), 'uid:' || uid) = ?2
           AND (?3 IS NULL OR folder = ?3)
         ORDER BY folder, uid",
    )?;
    let rows = stmt.query_map(
        params![account, thread_key, only_folder, unseen_only],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        },
    )?;
    let mut by_folder = std::collections::BTreeMap::<String, Vec<u32>>::new();
    for row in rows {
        let (folder, uid, subject) = row?;
        if subject_filter.is_some_and(|filter| thread_grouping_subject(&subject) != filter) {
            continue;
        }
        by_folder.entry(folder).or_default().push(uid);
    }
    Ok(by_folder)
}

/// Which of the account's threads have a draft waiting in them — the keys the list's Draft badge is
/// decided by. Only real thread keys: a draft with no threading headers falls back to
/// `uid:<its own uid>`, which says nothing about any thread and would collide with an unrelated
/// message that happens to hold that UID in another mailbox (UIDs are unique only within one).
pub fn draft_thread_keys(conn: &Connection, account: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT m.thread_key, m.folder, f.special_use
           FROM messages m
           LEFT JOIN folders f ON f.account = m.account AND f.name = m.folder
          WHERE m.account = ?1 AND m.uid <> 0 AND m.thread_key <> ''",
    )?;
    let rows = stmt.query_map(params![account], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut out = HashSet::new();
    for row in rows {
        let (thread_key, folder, special_use) = row?;
        if classify_folder_role(&folder, special_use.as_deref()) == "drafts" {
            out.insert(thread_key);
        }
    }
    Ok(out)
}

pub fn resolve_message_uids(
    conn: &Connection,
    account: &str,
    folder: &str,
    thread_key: &str,
    subject_filter: Option<&str>,
    uid: Option<u32>,
    explicit_uids: &[u32],
) -> Result<Vec<u32>> {
    if !explicit_uids.is_empty() {
        return Ok(explicit_uids.to_vec());
    }
    if thread_key.is_empty() {
        return Ok(uid.into_iter().collect());
    }
    let mut headers = get_thread_headers(conn, account, folder, thread_key)?;
    if let Some(filter) = subject_filter {
        headers.retain(|header| thread_grouping_subject(&header.subject) == filter);
    }
    Ok(headers.into_iter().map(|header| header.uid).collect())
}
