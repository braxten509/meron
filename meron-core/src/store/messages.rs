//! Stores synchronized headers and reconciles their conversation identities.
//! The engine queries these rows for recent mail, arrivals and thread membership.
//! Starred pages, body storage and synchronization markers have dedicated modules.

mod thread_membership;
pub use thread_membership::{
    draft_thread_keys, get_thread_headers, newest_thread_uids, resolve_message_uids,
    thread_uids_by_folder,
};

mod starred;
pub use starred::{
    get_starred, get_starred_all_accounts, get_starred_page, get_starred_page_from,
    starred_thread_identity,
};
mod sync_state;
pub(crate) use sync_state::remember_removed_uids;
use sync_state::removed_uids_in;
pub use sync_state::{
    begin_message_sync, clear_folder_messages, end_message_sync, prune_missing_messages,
    release_removed_messages, removed_message_epoch,
};
mod bodies;
pub use bodies::{
    backfill_files_batch, get_cached_message, has_cached_body, has_message, save_cached_message,
};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::HashSet;
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

use crate::imap::MessageHeader;
use crate::parse::{Attachment, Message};

use super::*;

pub fn upsert_messages(
    conn: &Connection,
    account: &str,
    folder: &str,
    messages: &[MessageHeader],
) -> Result<()> {
    upsert_messages_marking(conn, account, folder, messages, &[])
}

/// [`upsert_messages`], marking the rows of `backfill` Inbox backfill (see
/// [`super::inbox_backfill_uids`]) in the same write, so a stop halfway can't
/// leave them stored but unmarked (a later sync sees them cached and would
/// never mark them).
pub fn upsert_messages_marking(
    conn: &Connection,
    account: &str,
    folder: &str,
    messages: &[MessageHeader],
    backfill: &[u32],
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let removed = removed_uids_in(&tx, account, folder)?;
    // The Message-IDs this batch cached: the only ids whose arrival can hand a
    // canonical thread key down to rows already in the cache.
    let mut upserted_ids: HashSet<String> = HashSet::new();
    for header in messages {
        // A move or delete already dropped this UID. A sync that fetched it
        // beforehand must not put the row back into the folder it left.
        if removed.contains(&header.uid) {
            continue;
        }
        // Store recipient lists as JSON. Skip empty lists so a later flag-only
        // resync (which carries no envelope) can't clobber recipients we already
        // cached with NULLs.
        let mut extra = serde_json::Map::new();
        if !header.to.is_empty() {
            extra.insert("to".to_string(), json!(header.to));
        }
        if !header.cc.is_empty() {
            extra.insert("cc".to_string(), json!(header.cc));
        }
        if !header.message_id.is_empty() {
            extra.insert("message_id".to_string(), json!(header.message_id));
        }
        if let Some(gmail_message_id) = header.gmail_msg_id {
            extra.insert("gmail_msg_id".to_string(), json!(gmail_message_id));
        }
        if !header.in_reply_to.is_empty() {
            extra.insert("in_reply_to".to_string(), json!(header.in_reply_to));
        }
        if header.files.is_some() {
            extra.insert("attachment_metadata".to_string(), json!(true));
        }
        if backfill.contains(&header.uid) {
            extra.insert(super::ai_sort::INBOX_BACKFILL_KEY.to_string(), json!(1));
        }
        let extra_json = Value::Object(extra).to_string();
        let thread_key = resolve_message_thread_key(&tx, account, &header.thread_key)?;
        let message_id = header.message_id.trim().to_lowercase();
        if !message_id.is_empty() {
            upserted_ids.insert(message_id);
        }
        // Cached: the statement carries every trigger it may fire (FTS,
        // contacts), so compiling it per message dominated a large sync.
        tx.prepare_cached(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, starred, thread_key, json, recipients, files)
             VALUES(?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(account, folder, msg_id) DO UPDATE SET
               subject    = excluded.subject,
               from_name  = excluded.from_name,
               from_addr  = excluded.from_addr,
               date       = excluded.date,
               seen       = excluded.seen,
               starred    = excluded.starred,
               thread_key = excluded.thread_key,
               json       = json_patch(messages.json, excluded.json),
               -- Same rule as the recipient lists in `json`: a flag-only resync
               -- carries no envelope, so it must not blank what we already indexed.
               recipients = COALESCE(excluded.recipients, messages.recipients),
               -- A fetched body is authoritative, especially for inline images.
               files = CASE WHEN messages.body IS NOT NULL OR ?14 = 0
                            THEN messages.files ELSE excluded.files END",
        )?
        .execute(params![
                account,
                folder,
                header.uid,
                header.subject,
                header.from_name,
                header.from_addr,
                header.date,
                header.seen as i64,
                header.starred as i64,
                thread_key,
                extra_json,
                recipients_index_text(&header.to, &header.cc),
                header.files.as_ref().filter(|files| !files.is_empty()).map(|files| json!(files).to_string()),
                header.files.is_some() as i64
        ])?;
        if let Some(server_tags) = &header.server_tags {
            observe_server_tags(&tx, account, folder, header.uid, server_tags)?;
        }
    }
    let rekeyed = reconcile_thread_keys_from(&tx, account, upserted_ids)?;
    // Meron folders follow their conversations: the rows just written and the
    // rows reconciliation re-keyed, in this same transaction.
    let mut adopt = meron_row_ids(
        &tx,
        account,
        folder,
        messages
            .iter()
            .filter(|header| !removed.contains(&header.uid))
            .map(|header| header.uid),
    )?;
    if !adopt.is_empty() {
        adopt.extend(rekeyed.into_iter().map(|(id, old)| (id, Some(old))));
        meron_adopt(&tx, account, &adopt)?;
    }
    tx.commit()?;
    Ok(())
}

pub(super) fn resolve_message_thread_key(
    conn: &rusqlite::Transaction<'_>,
    account: &str,
    thread_key: &str,
) -> Result<String> {
    let key = thread_key.trim();
    if key.is_empty() || key.starts_with("uid:") || key.starts_with("gmthrid:") {
        return Ok(thread_key.to_string());
    }

    // References chooses a Message-ID as the raw thread key. That message may
    // itself already have inherited an older canonical root. Following the
    // key through its cached Message-ID keeps later replies in that same root,
    // even when their immediate In-Reply-To names a different message. Proton
    // Bridge exposes exactly this shape after a reply round trip.
    Ok(cached_thread_key_of(conn, account, &key.to_lowercase())?
        .unwrap_or_else(|| thread_key.to_string()))
}

/// The thread key of the cached message with this (lowercased) Message-ID, or
/// `None` when we haven't cached it. Duplicate copies of one message — the same
/// id in several folders — resolve to the oldest row, so the answer doesn't
/// depend on which folder synced last.
pub(super) fn cached_thread_key_of(
    conn: &rusqlite::Transaction<'_>,
    account: &str,
    message_id: &str,
) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT thread_key FROM messages
             WHERE account = ?1
               AND lower(COALESCE(json_extract(json, '$.message_id'), '')) = ?2
               AND COALESCE(thread_key, '') <> ''
             ORDER BY date ASC, uid ASC
             LIMIT 1",
            params![account, message_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?)
}

/// Hand the canonical thread key down to rows that named a message we only just cached.
///
/// A fresh folder sync may upsert a reply before the referenced row whose Message-ID reveals the
/// root key, so `resolve_message_thread_key` alone can't settle those.
///
/// Walking out from the ids this batch wrote — rather than re-scanning the account cache — keeps
/// the work proportional to the batch: each newly cached id fixes the rows that name it, and each
/// row so fixed becomes the next id to walk out from, since its own children inherited through it.
///
/// `seen` stops the walk from revisiting an id, so a reference cycle (two messages naming each
/// other) terminates instead of flip-flopping.
///
/// Answers with every row it re-keyed and that row's thread key from before, lowercased, so Meron
/// folders can follow the conversation.
pub(super) fn reconcile_thread_keys_from(
    conn: &rusqlite::Transaction<'_>,
    account: &str,
    seeds: HashSet<String>,
) -> Result<Vec<(i64, String)>> {
    let mut rekeyed = Vec::new();
    let mut seen = seeds;
    let mut pending: Vec<String> = seen.iter().cloned().collect();

    while !pending.is_empty() {
        let mut next: Vec<String> = Vec::new();
        for parent_id in pending.drain(..) {
            let Some(canonical) = cached_thread_key_of(conn, account, &parent_id)? else {
                continue;
            };
            let mut stmt = conn.prepare_cached(
                "SELECT id, lower(COALESCE(json_extract(json, '$.message_id'), '')),
                        lower(COALESCE(thread_key, ''))
                   FROM messages
                  WHERE account = ?1
                    AND uid <> 0
                    AND lower(COALESCE(thread_key, '')) = ?2
                    AND thread_key <> ?3
                    AND thread_key NOT LIKE 'uid:%'
                    AND thread_key NOT LIKE 'gmthrid:%'",
            )?;
            let children = stmt
                .query_map(params![account, parent_id, canonical], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (id, child_message_id, old_key) in children {
                conn.execute(
                    "UPDATE messages SET thread_key = ?1 WHERE id = ?2",
                    params![canonical, id],
                )?;
                rekeyed.push((id, old_key));
                if !child_message_id.is_empty() && seen.insert(child_message_id.clone()) {
                    next.push(child_message_id);
                }
            }
        }
        pending = next;
    }
    Ok(rekeyed)
}

/// UIDs in `folder` whose row a header sync wrote (it always sets a thread
/// key). A sync reads only the window's other UIDs in full; a row some other
/// path left without its header fields is read again.
pub fn header_synced_uids(conn: &Connection, account: &str, folder: &str) -> Result<HashSet<u32>> {
    let mut stmt = conn.prepare(
        "SELECT uid FROM messages WHERE account = ?1 AND folder = ?2 AND uid <> 0
         AND COALESCE(thread_key, '') <> ''",
    )?;
    let rows = stmt.query_map(params![account, folder], |row| row.get::<_, u32>(0))?;
    Ok(rows.collect::<rusqlite::Result<HashSet<_>>>()?)
}

/// Older header-only rows from before BODYSTRUCTURE was indexed. Bound each
/// sync's catch-up work; later syncs continue after the rows marked complete.
pub fn missing_attachment_metadata_uids(
    conn: &Connection,
    account: &str,
    folder: &str,
    limit: u32,
) -> Result<Vec<u32>> {
    let mut stmt = conn.prepare(
        "SELECT uid FROM messages WHERE account = ?1 AND folder = ?2 AND body IS NULL
         AND COALESCE(json_extract(json, '$.attachment_metadata'), 0) = 0
         AND COALESCE(json_extract(json, '$.attachment_metadata_attempted'), 0) = 0
         ORDER BY date DESC, uid DESC LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![account, folder, limit], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Mark bounded catch-up work even when a server omitted BODYSTRUCTURE or a
/// malformed response was isolated. Fresh header sync can still supply metadata.
pub fn mark_attachment_metadata_attempted(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for uid in uids {
        tx.execute(
            "UPDATE messages SET json = json_set(json, '$.attachment_metadata_attempted', 1)
                    WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Which cached rows a newest-first page keeps. The two narrowings combine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecentFilter {
    pub unread_only: bool,
    /// Select conversations containing a cached message with attachments.
    ///
    /// Messages whose *thread* has a cached message with a real attachment — thread-wide, like the card's
    /// paperclip, so an unread plain reply to a read message that carried the file still lists its thread under
    /// "unread with attachments". A subject-branched thread shares one key, so this is a superset; the card-level
    /// narrowing (`ThreadListQuery::retain_attachment_threads`) trims it to the branch.
    ///
    /// Header sync supplies BODYSTRUCTURE metadata even before a body is cached.
    pub attachments_only: bool,
}

/// The keyset cursor of a page that ended at `(date, uid)`: `date:<date>:<uid>`,
/// or, for a page that mixes folders, `date:<date>:<uid>:<base64url(folder)>`
/// with the last row's folder as the tie-breaker of equal (date, uid) (UIDs
/// repeat across folders). See `thread_list::parse_mail_cursor`.
pub fn format_mail_cursor(date: i64, uid: u32, folder: Option<&str>) -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    match folder {
        Some(folder) => format!(
            "date:{date}:{uid}:{}",
            URL_SAFE_NO_PAD.encode(folder.as_bytes())
        ),
        None => format!("date:{date}:{uid}"),
    }
}

pub fn get_recent_page(
    conn: &Connection,
    account: &str,
    folder: &str,
    limit: u32,
    before_cursor: Option<(i64, u32)>,
    filter: RecentFilter,
) -> Result<(Vec<MessageHeader>, Option<String>)> {
    get_recent_page_from(conn, account, folder, limit, before_cursor, None, filter)
}

/// [`get_recent_page`] continuing a cursor that carries its folder
/// tie-breaker (`before_folder`, see [`format_mail_cursor`]).
pub fn get_recent_page_from(
    conn: &Connection,
    account: &str,
    folder: &str,
    limit: u32,
    before_cursor: Option<(i64, u32)>,
    before_folder: Option<&str>,
    filter: RecentFilter,
) -> Result<(Vec<MessageHeader>, Option<String>)> {
    let probe = limit.saturating_add(1);
    // Conversations moved into a Meron folder show there instead (never hidden
    // from Drafts, Trash or Junk).
    let hide_meron = meron_hides_folder(conn, account, folder)?;
    // Mail the AI left in another folder: listed in the Inbox (each row
    // carrying its own folder, so opening it acts on the real message) and
    // left out of its own folder. Without any, the query is as it always was.
    let shown = super::ai_sort_inbox_has_rows(conn, account)?;
    let mixed = shown && folder.eq_ignore_ascii_case("INBOX");
    // Newest-first by send time. The cursor is the (date, uid) of the last row of
    // the previous page; uid is the keyset tiebreaker because `date` is not unique
    // (two messages can share a second), so it gives a stable, gap-free walk.
    // A page mixing folders breaks (date, uid) ties by folder too, as UIDs
    // repeat across folders; an older cursor without it cuts ties as before.
    let after = if mixed {
        "(?3 IS NULL
                OR date < ?3
                OR (date = ?3 AND (uid < ?4
                     OR (?9 IS NOT NULL AND uid = ?4 AND folder < ?9))))"
    } else {
        "(?3 IS NULL
                OR date < ?3
                OR (date = ?3 AND uid < ?4))"
    };
    // The Inbox lists only mail newer than the AI sorting age limit (both its
    // own rows and mail shown there from other folders). Filtered here, before
    // the LIMIT, so pages stay full and the cursor exact; a card shows while
    // any of its Inbox messages is new enough, since its newest row decides.
    let recent = if super::inbox_hides_old(account, folder) {
        format!(
            " AND {}",
            super::inbox_listed_sql("messages", super::now_unix())
        )
    } else {
        String::new()
    };
    let narrow = format!(
        "NOT EXISTS (SELECT 1 FROM ai_sort_hidden h
                WHERE h.account = messages.account AND h.folder = messages.folder
                  AND h.uid = messages.uid
                  AND h.hide_until > CAST(strftime('%s','now') AS INTEGER)){recent}
           AND (?8 = 0 OR NOT {hidden})
           AND (?6 = 0 OR seen = 0)
           AND (?7 = 0 OR files IS NOT NULL
                OR (COALESCE(thread_key, '') <> '' AND EXISTS (
                      SELECT 1 FROM messages t
                      WHERE t.account = messages.account
                        AND lower(COALESCE(t.thread_key, '')) = lower(COALESCE(messages.thread_key, ''))
                        AND t.files IS NOT NULL)))
           AND {after}",
        hidden = meron_hidden_sql("messages"),
    );
    let columns = "uid, subject, from_name, from_addr, date, seen, starred, thread_key,
                json_extract(json, '$.to')";
    let sql = if mixed {
        format!(
            "SELECT {columns}, '', folder FROM messages
              WHERE account = ?1 AND folder = ?2 AND {narrow}
             UNION ALL
             SELECT {columns}, folder, folder FROM messages
              WHERE id IN {ids} AND {narrow}
             ORDER BY 5 DESC, 1 DESC, 11 DESC LIMIT ?5",
            ids = super::ai_sort_inbox_ids_sql("?1"),
        )
    } else if shown {
        format!(
            "SELECT {columns}, '', folder FROM messages
              WHERE account = ?1 AND folder = ?2 AND {narrow} AND NOT {shown_here}
             ORDER BY date DESC, uid DESC LIMIT ?5",
            shown_here = super::ai_sort_inbox_sql("messages"),
        )
    } else {
        format!(
            "SELECT {columns}, '', folder FROM messages
              WHERE account = ?1 AND folder = ?2 AND {narrow}
             ORDER BY date DESC, uid DESC LIMIT ?5"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let cursor_date = before_cursor.map(|(date, _)| date);
    let cursor_uid = before_cursor.map(|(_, uid)| uid as i64).unwrap_or(0);
    // Each row, with its real folder (the tie-breaker of a mixed page).
    let map = |row: &rusqlite::Row<'_>| {
        let uid = row.get(0)?;
        let mut header = MessageHeader {
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
            to: parse_recipients_json(row.get::<_, Option<String>>(8)?),
            // Empty for the folder's own mail; another folder's name for
            // mail shown in the Inbox from there.
            folder: row.get(9)?,
            ..Default::default()
        };
        // Only the mixed Inbox read fills the folder column, and only for
        // mail shown from elsewhere: those rows make cards of their own.
        header.ai_shown = mixed && !header.folder.is_empty();
        Ok((header, row.get::<_, String>(10)?))
    };
    let base = params![
        account,
        folder,
        cursor_date,
        cursor_uid,
        probe as i64,
        filter.unread_only as i64,
        filter.attachments_only as i64,
        hide_meron as i64
    ];
    let mut out: Vec<(MessageHeader, String)> = if mixed {
        let mut all: Vec<&dyn rusqlite::ToSql> = base.to_vec();
        all.push(&before_folder);
        stmt.query_map(all.as_slice(), map)?
            .collect::<rusqlite::Result<_>>()?
    } else {
        stmt.query_map(base, map)?
            .collect::<rusqlite::Result<_>>()?
    };
    let has_more = out.len() > limit as usize;
    if has_more {
        out.truncate(limit as usize);
    }
    let next_cursor = if has_more {
        out.last().map(|(header, real)| {
            format_mail_cursor(header.date, header.uid, mixed.then_some(real.as_str()))
        })
    } else {
        None
    };
    Ok((
        out.into_iter().map(|(header, _)| header).collect(),
        next_cursor,
    ))
}

#[cfg(test)]
pub(super) fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub(super) fn mail_identity_from_parts(
    folder: &str,
    uid: u32,
    gmail_message_id: Option<u64>,
    message_id: &str,
) -> String {
    if let Some(gmail_message_id) = gmail_message_id {
        return format!("gmail:{gmail_message_id}");
    }
    let message_id = message_id.trim();
    if !message_id.is_empty() {
        return format!("message-id:{}", message_id.to_lowercase());
    }
    format!("uid:{}:{uid}", folder.to_lowercase())
}

pub(super) fn message_identity(header: &MessageHeader, folder: &str) -> String {
    mail_identity_from_parts(folder, header.uid, header.gmail_msg_id, &header.message_id)
}

#[cfg(test)]
pub(super) fn gmail_message_id_from_json(value: Option<String>) -> Option<u64> {
    value.and_then(|value| value.parse::<u64>().ok())
}

/// Record message identities and return the subset that had not been observed
/// before this call.
#[cfg(test)]
pub(super) fn record_observed_mail_identities(
    conn: &Connection,
    account: &str,
    folder: &str,
    messages: &[MessageHeader],
) -> Result<std::collections::HashSet<String>> {
    let now = now_epoch_seconds();
    let tx = conn.unchecked_transaction()?;
    let mut new_identities = std::collections::HashSet::new();
    {
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO observed_mail_identities(account, identity, first_seen_at)
             VALUES(?1, ?2, ?3)",
        )?;
        for message in messages {
            let identity = message_identity(message, folder);
            let inserted = stmt.execute(params![account, &identity, now])?;
            if inserted > 0 {
                new_identities.insert(identity);
            }
        }
    }
    tx.commit()?;
    Ok(new_identities)
}

/// [`classify_arrivals`] for INBOX.
pub fn classify_inbox_arrivals(
    conn: &Connection,
    account: &str,
    before: u32,
    after: u32,
    messages: &[MessageHeader],
) -> Result<Vec<MessageHeader>> {
    classify_arrivals(conn, account, "INBOX", before, after, messages)
}

/// Unread messages in `folder` in the UID range that appeared during the last sync, newest first.
/// Returns the whole batch rather than just its latest message so notifications can post one entry
/// per arrival; empty when nothing new and unread landed. A message already observed anywhere in
/// the account (one the user filed here from INBOX, say) is not an arrival.
pub fn classify_arrivals(
    conn: &Connection,
    account: &str,
    folder: &str,
    before: u32,
    after: u32,
    messages: &[MessageHeader],
) -> Result<Vec<MessageHeader>> {
    let unread = messages
        .iter()
        .filter(|message| !message.seen)
        .cloned()
        .collect::<Vec<_>>();
    first_seen_messages(conn, account, folder, before, after, &unread)
}

/// Messages in the UID range of the last sync that the account has not observed
/// anywhere before, newest first, read or not. Mail filed here from another
/// folder got a new UID, but it has not arrived.
pub fn first_seen_messages(
    conn: &Connection,
    account: &str,
    folder: &str,
    before: u32,
    after: u32,
    messages: &[MessageHeader],
) -> Result<Vec<MessageHeader>> {
    let mut arrivals = Vec::new();
    if before == 0 || after <= before {
        return Ok(arrivals);
    }
    let mut stmt = conn
        .prepare("SELECT 1 FROM observed_mail_identities WHERE account = ?1 AND identity = ?2")?;
    let mut identities = HashSet::new();
    for message in messages {
        if message.uid < before || message.uid >= after {
            continue;
        }
        let identity = message_identity(message, folder);
        if stmt
            .query_row(params![account, &identity], |_| Ok(()))
            .optional()?
            .is_none()
            && identities.insert(identity)
        {
            arrivals.push(message.clone());
        }
    }
    arrivals.sort_by_key(|message| std::cmp::Reverse(message.uid));
    Ok(arrivals)
}

#[cfg(test)]
pub fn new_unread_inbox_messages(
    conn: &Connection,
    account: &str,
    uid_next_before: u32,
    uid_next_after: u32,
    synced_messages: &[MessageHeader],
) -> Result<Option<Vec<MessageHeader>>> {
    if uid_next_before == 0 || uid_next_after <= uid_next_before {
        return Ok(None);
    }
    let newly_observed = record_observed_mail_identities(conn, account, "INBOX", synced_messages)?;
    if newly_observed.is_empty() {
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT uid, subject, from_name, from_addr, date, seen, starred, thread_key,
                json_extract(json, '$.to'),
                CAST(json_extract(json, '$.gmail_msg_id') AS TEXT),
                json_extract(json, '$.message_id') FROM messages
         WHERE account = ?1 AND folder = 'INBOX'
           AND uid >= ?2 AND uid < ?3 AND seen = 0
         ORDER BY uid DESC",
    )?;
    let rows = stmt.query_map(
        params![account, uid_next_before as i64, uid_next_after as i64],
        |row| {
            let mut header = message_header_from_row(row)?;
            header.gmail_msg_id = gmail_message_id_from_json(row.get::<_, Option<String>>(9)?);
            header.message_id = row.get::<_, Option<String>>(10)?.unwrap_or_default();
            Ok(header)
        },
    )?;
    let headers = rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|header| newly_observed.contains(&message_identity(header, "INBOX")))
        .collect::<Vec<_>>();
    if headers.is_empty() {
        return Ok(None);
    }
    Ok(Some(headers))
}

/// One-line body snippet for a cached message, or `None` when the body hasn't
/// been fetched yet. Notifications use it to show the mail itself rather than
/// the subject alone; a miss degrades to a subject-only notification.
pub fn cached_body_preview(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
) -> Option<String> {
    let message = get_cached_message(conn, account, folder, uid).ok()??;
    let preview = crate::parse::preview_of(&message.body);
    (!preview.trim().is_empty()).then_some(preview)
}

/// Progress of [`backfill_files_batch`] in `meta`: the last `messages.id`
/// done, or `done`.
const FILES_BACKFILL_KEY: &str = "files_backfill";
