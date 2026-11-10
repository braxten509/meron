//! Sender grouping: one mailbox row per correspondent instead of per thread.
//!
//! A sender row's thread key is `sender:<lowercase address>` (see
//! [`SENDER_KEY_PREFIX`]), wrapped in the ordinary thread id, so every action
//! path that already passes thread keys through reaches the functions here.
//!
//! "The other person" of a message follows the thread card's identity rule
//! ([`super::apply_card_identity`]): an outgoing message (a Sent-like mailbox of
//! a server account, or From one of the account's own addresses) names its
//! first recipient, anything else its sender.

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{Connection, params};

use crate::imap::MessageHeader;

use super::*;

/// Thread-key prefix of a sender row.
pub const SENDER_KEY_PREFIX: &str = "sender:";

/// The address a `sender:` thread key names, lowercased; `None` for any other
/// key.
pub fn sender_key_addr(thread_key: &str) -> Option<String> {
    thread_key
        .strip_prefix(SENDER_KEY_PREFIX)
        .map(|addr| addr.trim().to_lowercase())
}

/// The `sender:` thread key for an address.
pub fn sender_thread_key(addr: &str) -> String {
    format!("{SENDER_KEY_PREFIX}{}", addr.trim().to_lowercase())
}

/// One sender row of a mailbox page.
#[derive(Clone)]
pub struct SenderGroup {
    /// Lowercased address of the correspondent.
    pub who: String,
    /// Their newest message in the folder (its date is the row's date).
    pub newest: MessageHeader,
    pub unread_count: u32,
    pub message_count: u32,
    pub starred: bool,
    pub has_attachments: bool,
}

/// Whether messages of `folder` count as outgoing by mailbox alone, as
/// [`super::is_outgoing`] decides it for a server account.
fn folder_is_outgoing(account: &str, folder: &str) -> bool {
    !is_local_account(account) && crate::imap::looks_like_sent(folder)
}

fn mine_json(conn: &Connection, account: &str) -> String {
    let mine: Vec<String> = self_addrs(conn, account).into_iter().collect();
    serde_json::to_string(&mine).unwrap_or_else(|_| "[]".to_string())
}

/// SQL for the correspondent of a `messages` row. `?sent` is 1 when the folder
/// is Sent-like, `?mine` a JSON array of the account's own addresses.
fn who_sql(sent: &str, mine: &str) -> String {
    format!(
        "CASE WHEN ({sent} = 1 OR lower(trim(COALESCE(from_addr, ''))) IN (SELECT value FROM json_each({mine})))
                AND lower(trim(COALESCE(json_extract(json, '$.to[0].addr'), ''))) <> ''
              THEN lower(trim(json_extract(json, '$.to[0].addr')))
              ELSE lower(trim(COALESCE(from_addr, ''))) END"
    )
}

/// The correspondent of one header in `folder`, by the same rule as
/// [`who_sql`]. Used to check headers read from the server.
pub fn sender_person(
    account: &str,
    mine: &HashSet<String>,
    folder: &str,
    header: &MessageHeader,
) -> String {
    if account_message_is_outgoing(account, mine, folder, &header.from_addr, false)
        && let Some(first) = header.to.first()
    {
        let addr = first.addr.trim().to_lowercase();
        if !addr.is_empty() {
            return addr;
        }
    }
    header.from_addr.trim().to_lowercase()
}

/// `sender:<last date>:<base64url(address)>` keyset cursor.
pub fn format_sender_cursor(last: i64, who: &str) -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    format!("sender:{last}:{}", URL_SAFE_NO_PAD.encode(who.as_bytes()))
}

pub fn parse_sender_cursor(cursor: &str) -> Option<(i64, String)> {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let rest = cursor.strip_prefix("sender:")?;
    let (last, who) = rest.split_once(':')?;
    let who = String::from_utf8(URL_SAFE_NO_PAD.decode(who).ok()?).ok()?;
    Some((last.parse().ok()?, who))
}

/// One page of sender rows for `folder`, newest correspondent first. Rows are
/// grouped before the LIMIT, so a person appears on exactly one page; the
/// keyset is (newest date desc, address asc). `filter` keeps people with at
/// least one unread message and/or one with attachments.
pub fn get_sender_page(
    conn: &Connection,
    account: &str,
    folder: &str,
    limit: u32,
    cursor: Option<(i64, String)>,
    filter: RecentFilter,
) -> Result<(Vec<SenderGroup>, Option<String>)> {
    let who = who_sql("?3", "?4");
    let hide_meron = meron_hides_folder(conn, account, folder)?;
    let hidden = meron_hidden_sql("messages");
    // As in the Inbox's thread list: only mail newer than the age limit, so a
    // person shows while one of their Inbox messages is (and their row's
    // counts and date come from those). Opening the row still shows it all.
    let recent = if inbox_hides_old(account, folder) {
        format!(" AND {}", inbox_listed_sql("messages", now_unix()))
    } else {
        String::new()
    };
    let narrow = format!(
        "uid <> 0
              AND NOT EXISTS (SELECT 1 FROM ai_sort_hidden h
                   WHERE h.account = messages.account AND h.folder = messages.folder
                     AND h.uid = messages.uid
                     AND h.hide_until > CAST(strftime('%s','now') AS INTEGER)){recent}
              AND (?10 = 0 OR NOT {hidden})"
    );
    let columns = format!(
        "uid, subject, from_name, from_addr, date, seen, starred, thread_key,
                  json_extract(json, '$.to') AS to_json, files, {who} AS who"
    );
    // Sender rows leave the AI's "shown in the Inbox" out: each person's row
    // lists, and its archive, delete and move act on, the mail really in this
    // folder. (An Inbox row listing mail from other folders acted on the Inbox
    // copies only, so that mail stayed put.) Mail shown in the Inbox from
    // another folder lists under its own folder's sender rows instead.
    let rows = format!(
        "SELECT {columns}, '' AS src FROM messages
          WHERE account = ?1 AND folder = ?2 AND {narrow}"
    );
    // A single max() makes SQLite take the bare columns from the newest row of
    // each group; every other aggregate is a SUM/COUNT for that reason.
    let sql = format!(
        "WITH p AS (
           {rows}
         )
         SELECT who, max(date) AS last, uid, subject, from_name, from_addr, seen, starred,
                thread_key, to_json,
                SUM(seen = 0), COUNT(*), SUM(starred <> 0), SUM(files IS NOT NULL), src
           FROM p
          GROUP BY who
         HAVING (?5 = 0 OR SUM(seen = 0) > 0)
            AND (?6 = 0 OR SUM(files IS NOT NULL) > 0)
            AND (?7 IS NULL OR max(date) < ?7 OR (max(date) = ?7 AND who > ?8))
          ORDER BY last DESC, who ASC
          LIMIT ?9"
    );
    let mut stmt = conn.prepare(&sql)?;
    let (cursor_last, cursor_who) = match &cursor {
        Some((last, who)) => (Some(*last), who.clone()),
        None => (None, String::new()),
    };
    let rows = stmt.query_map(
        params![
            account,
            folder,
            folder_is_outgoing(account, folder) as i64,
            mine_json(conn, account),
            filter.unread_only as i64,
            filter.attachments_only as i64,
            cursor_last,
            cursor_who,
            limit.saturating_add(1) as i64,
            hide_meron as i64,
        ],
        |row| {
            let uid: u32 = row.get(2)?;
            Ok(SenderGroup {
                who: row.get(0)?,
                newest: MessageHeader {
                    uid,
                    date: row.get(1)?,
                    subject: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    from_name: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    from_addr: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    seen: row.get::<_, i64>(6)? != 0,
                    starred: row.get::<_, i64>(7)? != 0,
                    thread_key: row
                        .get::<_, Option<String>>(8)?
                        .filter(|key| !key.is_empty())
                        .unwrap_or_else(|| format!("uid:{uid}")),
                    to: parse_recipients_json(row.get::<_, Option<String>>(9)?),
                    folder: Some(row.get::<_, String>(14)?)
                        .filter(|src| !src.is_empty())
                        .unwrap_or_else(|| folder.to_string()),
                    ..Default::default()
                },
                unread_count: row.get::<_, i64>(10)? as u32,
                message_count: row.get::<_, i64>(11)? as u32,
                starred: row.get::<_, i64>(12)? > 0,
                has_attachments: row.get::<_, i64>(13)? > 0,
            })
        },
    )?;
    let mut groups = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = groups.len() > limit as usize;
    if has_more {
        groups.truncate(limit as usize);
    }
    let next_cursor = has_more
        .then(|| groups.last())
        .flatten()
        .map(|group| format_sender_cursor(group.newest.date, &group.who));
    Ok((groups, next_cursor))
}

const CHAT_COLUMNS: &str = "uid, folder, subject, from_name, from_addr, date, seen, starred,
     thread_key, json_extract(json, '$.in_reply_to'),
     lower(trim(COALESCE(json_extract(json, '$.message_id'), ''), ' <>'))";

fn chat_header_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageHeader> {
    let uid: u32 = row.get(0)?;
    Ok(MessageHeader {
        uid,
        folder: row.get(1)?,
        subject: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        from_name: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        from_addr: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        date: row.get(5)?,
        seen: row.get::<_, i64>(6)? != 0,
        starred: row.get::<_, i64>(7)? != 0,
        thread_key: row
            .get::<_, Option<String>>(8)?
            .filter(|key| !key.is_empty())
            .unwrap_or_else(|| format!("uid:{uid}")),
        in_reply_to: row.get::<_, Option<String>>(9)?.unwrap_or_default(),
        message_id: row.get::<_, Option<String>>(10)?.unwrap_or_default(),
        ..Default::default()
    })
}

/// The messages in `folder` whose correspondent is `addr`, oldest first, each
/// carrying its folder. This is what a sender row's actions (delete, move,
/// mark read, tags…) act on.
pub fn sender_folder_headers(
    conn: &Connection,
    account: &str,
    folder: &str,
    addr: &str,
) -> Result<Vec<MessageHeader>> {
    let addr = addr.trim().to_lowercase();
    let who = who_sql("?3", "?4");
    let hidden = meron_hidden_sql("messages");
    // Mail the AI shows in the Inbox from here is this folder's row's: sender
    // rows list each folder's own mail (see `get_sender_page`).
    let mut stmt = conn.prepare(&format!(
        "SELECT {CHAT_COLUMNS} FROM messages
          WHERE account = ?1 AND folder = ?2 AND uid <> 0 AND {who} = ?5
            AND (?6 = 0 OR NOT {hidden})
          ORDER BY date ASC, uid ASC"
    ))?;
    let rows = stmt.query_map(
        params![
            account,
            folder,
            folder_is_outgoing(account, folder) as i64,
            mine_json(conn, account),
            addr,
            meron_hides_folder(conn, account, folder)? as i64
        ],
        chat_header_from_row,
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// [`sender_folder_headers`] as UIDs.
pub fn sender_folder_uids(
    conn: &Connection,
    account: &str,
    folder: &str,
    addr: &str,
) -> Result<Vec<u32>> {
    Ok(sender_folder_headers(conn, account, folder, addr)?
        .into_iter()
        .map(|header| header.uid)
        .collect())
}

/// Messages of `folder` addressed (To) to `addr`.
pub(super) fn addressed_to(
    conn: &Connection,
    account: &str,
    folder: &str,
    addr: &str,
) -> Result<Vec<MessageHeader>> {
    let hidden = meron_hidden_sql("messages");
    let mut stmt = conn.prepare(&format!(
        "SELECT {CHAT_COLUMNS} FROM messages
          WHERE account = ?1 AND folder = ?2 AND uid <> 0
            AND EXISTS (SELECT 1 FROM json_each(messages.json, '$.to') r
                         WHERE lower(trim(COALESCE(json_extract(r.value, '$.addr'), ''))) = ?3)
            AND (?4 = 0 OR NOT {hidden})
          ORDER BY date ASC, uid ASC"
    ))?;
    let hide_meron = meron_hides_folder(conn, account, folder)?;
    let rows = stmt.query_map(
        params![account, folder, addr, hide_meron as i64],
        chat_header_from_row,
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The chat a sender row opens, oldest first: their messages in `folder`, the
/// account's Sent mail to them, and drafts to them. Other folders stay out,
/// and so do replies to a conversation the reader deleted.
/// Two copies of one Message-ID (mail to yourself) show once, preferring the
/// copy in `folder` so the chat's read state follows the row.
pub fn sender_chat_headers(
    conn: &Connection,
    account: &str,
    folder: &str,
    addr: &str,
) -> Result<Vec<MessageHeader>> {
    let addr = addr.trim().to_lowercase();
    let mut headers = sender_folder_headers(conn, account, folder, &addr)?;
    if !addr.is_empty() {
        // Replies to a conversation the reader deleted stay out (see
        // `ChatScope`); `CHAT_COLUMNS` reads the Message-ID as tags key it.
        let scope = ChatScope::for_folder(conn, account, folder)?;
        for role in ["sent", "drafts"] {
            if let Some(other) = folder_for_role(conn, account, role)?
                && other != folder
            {
                headers.extend(
                    addressed_to(conn, account, &other, &addr)?
                        .into_iter()
                        .filter(|header| {
                            let tag_key = if header.message_id.is_empty() {
                                format!("uid:{}:{}", header.folder, header.uid)
                            } else {
                                header.message_id.clone()
                            };
                            !scope.leaves_out(&header.folder, &tag_key)
                        }),
                );
            }
        }
    }
    // Keep the first copy of each Message-ID: this folder's copies come first.
    let mut seen: HashSet<String> = HashSet::new();
    headers.retain(|header| header.message_id.is_empty() || seen.insert(header.message_id.clone()));
    headers.sort_by(|a, b| (a.date, a.uid).cmp(&(b.date, b.uid)));
    Ok(headers)
}

/// The UIDs among `uids` that have a cached row in `folder`.
pub fn cached_uids_in_folder(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> Result<HashSet<u32>> {
    let mut stmt =
        conn.prepare("SELECT 1 FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3")?;
    let mut out = HashSet::new();
    for uid in uids {
        if stmt.exists(params![account, folder, *uid])? {
            out.insert(*uid);
        }
    }
    Ok(out)
}

/// For each correspondent in `addrs`, whether the account has a draft to them.
pub fn sender_draft_addrs(
    conn: &Connection,
    account: &str,
    addrs: &[String],
) -> Result<HashSet<String>> {
    let Some(drafts) = folder_for_role(conn, account, "drafts")? else {
        return Ok(HashSet::new());
    };
    let wanted: HashSet<&str> = addrs.iter().map(String::as_str).collect();
    let mut stmt = conn.prepare(
        "SELECT DISTINCT lower(trim(COALESCE(json_extract(r.value, '$.addr'), '')))
           FROM messages m, json_each(m.json, '$.to') r
          WHERE m.account = ?1 AND m.folder = ?2 AND m.uid <> 0",
    )?;
    let rows = stmt.query_map(params![account, drafts], |row| row.get::<_, String>(0))?;
    let mut out = HashSet::new();
    for addr in rows {
        let addr = addr?;
        if wanted.contains(addr.as_str()) {
            out.insert(addr);
        }
    }
    Ok(out)
}

/// A sender row's messages in `folder`, grouped like
/// [`thread_uids_by_folder`]: only this folder.
pub fn sender_uids_by_folder(
    conn: &Connection,
    account: &str,
    folder: &str,
    addr: &str,
    unseen_only: bool,
) -> Result<std::collections::BTreeMap<String, Vec<u32>>> {
    let uids: Vec<u32> = sender_folder_headers(conn, account, folder, addr)?
        .into_iter()
        .filter(|header| !unseen_only || !header.seen)
        .map(|header| header.uid)
        .collect();
    let mut out = std::collections::BTreeMap::new();
    if !uids.is_empty() {
        out.insert(folder.to_string(), uids);
    }
    Ok(out)
}
