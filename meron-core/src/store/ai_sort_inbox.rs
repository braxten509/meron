//! AI sorting of folders other than the Inbox: mail a server filter delivered
//! straight into a folder (Runbox's "Notifications", say) goes through the same
//! sorting as new Inbox mail. What the AI files goes into a Meron folder; what
//! it leaves is *shown in the Inbox*: listed and counted there, and left out of
//! its own folder's list, on this device only. The server copy never moves, so
//! opening, replying or deleting acts on the real message where it is.
//!
//! `ai_sort_watch` says where each folder's sorting starts: only mail that
//! arrives after Meron first looked at the folder is sorted. `ai_sort_inbox`
//! holds the shown mail by location; a row goes when its message leaves the
//! folder (deleted or moved, here or on the server), when the user files it
//! into a Meron folder, or when the same message turns up in the Inbox itself
//! (Gmail shows one message in INBOX and in each of its labels).

use std::collections::BTreeSet;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use crate::imap::MessageHeader;

use super::AiSortCandidate;

/// Whether `folder` of `account` is one AI sorting watches besides the Inbox: a
/// plain mail folder. Sent, Drafts, Trash, Junk, Archive and All Mail are left
/// alone, as are Gmail's own folders (Important, Starred, All Mail…) and local
/// folders. The AI's own server targets are left alone too: mail a server
/// filter put there is already where the AI would put it.
pub fn ai_sort_watches_folder(
    conn: &Connection,
    config: &super::AiSortConfig,
    account: &str,
    folder: &str,
) -> Result<bool> {
    if folder.eq_ignore_ascii_case("INBOX") || super::is_local_account(account) {
        return Ok(false);
    }
    let special_use: Option<String> = conn
        .query_row(
            "SELECT special_use FROM folders WHERE account = ?1 AND name = ?2",
            params![account, folder],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    // Starred (\Flagged) has no role of its own, so the role alone would let it in.
    if special_use.as_deref().is_some_and(|use_| !use_.is_empty()) {
        return Ok(false);
    }
    if super::classify_folder_role(folder, None) != "folder" {
        return Ok(false);
    }
    let gmail = ["[gmail]", "[google mail]"];
    let lower = folder.to_lowercase();
    if gmail
        .iter()
        .any(|root| lower == *root || lower.starts_with(&format!("{root}/")))
    {
        return Ok(false);
    }
    Ok(!config
        .folders_of(account)
        .iter()
        .any(|target| target.folder == folder))
}

/// One folder the AI-sorting check looks at.
#[derive(Clone, Debug, PartialEq)]
pub struct AiSortCheckFolder {
    pub name: String,
    /// Its stored `(UIDVALIDITY, UIDNEXT)`, if it was synced before.
    pub state: Option<(u32, u32)>,
    /// Its watch: `(UIDVALIDITY, first UID not yet looked at for sorting)`.
    pub watch: Option<(u32, u32)>,
}

impl AiSortCheckFolder {
    /// Whether mail below the folder's UIDNEXT is still waiting to be looked
    /// at: more arrived at once than one sync's window holds.
    pub fn behind(&self) -> bool {
        matches!((self.state, self.watch), (Some((validity, uid_next)), Some((watched, from)))
            if validity == watched && from < uid_next)
    }
}

/// The folders of `account` the AI-sorting check looks at besides the Inbox.
/// Folders opted in to notifications are left out: their own watch or poll
/// syncs them (and so sorts them), and a sync here would take their alerts.
pub fn ai_sort_check_folders(
    conn: &Connection,
    config: &super::AiSortConfig,
    account: &str,
) -> Result<Vec<AiSortCheckFolder>> {
    let names: Vec<String> = conn
        .prepare("SELECT name FROM folders WHERE account = ?1 AND notify = 0 ORDER BY name")?
        .query_map(params![account], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = Vec::new();
    for name in names {
        if ai_sort_watches_folder(conn, config, account, &name)? {
            let state = super::get_folder_state(conn, account, &name)?;
            let watch = ai_sort_watch(conn, account, &name)?;
            out.push(AiSortCheckFolder { name, state, watch });
        }
    }
    Ok(out)
}

/// The folder's watch: `(UIDVALIDITY, first UID not yet looked at)`.
pub fn ai_sort_watch(conn: &Connection, account: &str, folder: &str) -> Result<Option<(u32, u32)>> {
    Ok(conn
        .query_row(
            "SELECT uidvalidity, from_uid FROM ai_sort_watch WHERE account = ?1 AND folder = ?2",
            params![account, folder],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

/// A sync of `folder` while AI sorting is off for it: its watch (if any)
/// moves past what it stored, so turning sorting back on sorts only mail that
/// arrives from then on, as in the Inbox.
pub fn ai_sort_watch_skip(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    uid_next: u32,
) -> Result<()> {
    ai_sort_watch_advance(conn, account, folder, uidvalidity, uid_next)
}

/// Move the watch on to `next`, the first UID not yet looked at, never back.
pub(super) fn ai_sort_watch_advance(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    next: u32,
) -> Result<()> {
    conn.execute(
        "UPDATE ai_sort_watch SET from_uid = MAX(from_uid, ?4)
          WHERE account = ?1 AND folder = ?2 AND uidvalidity = ?3",
        params![account, folder, uidvalidity, next],
    )?;
    Ok(())
}

/// Where sorting of `folder` stands, under `uidvalidity`: the first UID not yet
/// looked at (every UID below it was, or came before the first look). The first
/// look at a folder (or after its UIDs were reset) starts it at `uid_next` and
/// answers `None`: nothing there is new yet.
pub fn ai_sort_watch_from(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    uid_next: u32,
) -> Result<Option<u32>> {
    let stored: Option<(u32, u32)> = conn
        .query_row(
            "SELECT uidvalidity, from_uid FROM ai_sort_watch WHERE account = ?1 AND folder = ?2",
            params![account, folder],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match stored {
        Some((validity, from)) if validity == uidvalidity => Ok(Some(from)),
        _ => {
            conn.execute(
                "INSERT OR REPLACE INTO ai_sort_watch(account, folder, uidvalidity, from_uid)
                 VALUES(?1, ?2, ?3, ?4)",
                params![account, folder, uidvalidity, uid_next],
            )?;
            Ok(None)
        }
    }
}

/// Whether `account` has any mail shown in the Inbox from another folder. A
/// primary-key prefix probe, so lists and counts skip the rest without it.
pub fn ai_sort_inbox_has_rows(conn: &Connection, account: &str) -> Result<bool> {
    Ok(conn
        .prepare_cached("SELECT 1 FROM ai_sort_inbox WHERE account = ?1 LIMIT 1")?
        .exists(params![account])?)
}

/// SQL that is true when the `messages` row aliased `alias` is shown in the
/// Inbox from its own folder (and so is left out of that folder's lists).
pub fn ai_sort_inbox_sql(alias: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM ai_sort_inbox x
                  WHERE x.account = {alias}.account AND x.folder = {alias}.folder
                    AND x.uid = {alias}.uid)"
    )
}

/// SQL for the row ids of the account (`account`, an SQL expression) whose mail
/// is shown in its Inbox from other folders: what an Inbox list adds.
pub fn ai_sort_inbox_ids_sql(account: &str) -> String {
    format!(
        "(SELECT m.id FROM ai_sort_inbox x
            JOIN messages m ON m.account = x.account AND m.folder = x.folder AND m.uid = x.uid
           WHERE x.account = {account})"
    )
}

/// Unread mail shown in the Inbox from other folders, of the account `account`
/// names, or only of `folder` when given (both SQL expressions): what the
/// Inbox count adds and the folder's count takes off. Mail in a Meron folder,
/// or held while the AI decides, counts in neither, as in the folder counts.
/// The Inbox (`folder` `None`) adds only mail it lists: newer than the age
/// limit ([`super::inbox_listed_sql`]); its own folder still leaves it out.
pub fn ai_sort_inbox_unread_sql(account: &str, folder: Option<&str>) -> String {
    let in_folder = match folder {
        Some(folder) => format!(" AND x.folder = {folder}"),
        None => format!(" AND {}", super::inbox_listed_sql("m", super::now_unix())),
    };
    format!(
        "(CASE WHEN EXISTS (SELECT 1 FROM ai_sort_inbox WHERE account = {account})
          THEN (SELECT COUNT(*) FROM ai_sort_inbox x
                  JOIN messages m ON m.account = x.account AND m.folder = x.folder AND m.uid = x.uid
                 WHERE x.account = {account}{in_folder} AND m.seen = 0
                   AND NOT EXISTS (SELECT 1 FROM ai_sort_hidden h
                        WHERE h.account = m.account AND h.folder = m.folder AND h.uid = m.uid
                          AND h.hide_until > CAST(strftime('%s','now') AS INTEGER))
                   AND NOT {hidden})
          ELSE 0 END)",
        hidden = super::meron_hidden_sql("m"),
    )
}

/// Show mail the AI left (or couldn't decide on) in the Inbox. Mail whose copy
/// is in the Inbox already (Gmail labels) needs nothing: it shows there.
pub fn ai_sort_show_in_inbox(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    candidates: &[AiSortCandidate],
) -> Result<()> {
    if folder.eq_ignore_ascii_case("INBOX") {
        return Ok(());
    }
    let now = super::now_unix();
    let mut insert = conn.prepare_cached(
        "INSERT OR REPLACE INTO ai_sort_inbox(account, folder, uid, uidvalidity, identity, at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for candidate in candidates {
        if in_inbox(conn, account, candidate)? {
            continue;
        }
        insert.execute(params![
            account,
            folder,
            candidate.uid,
            uidvalidity,
            candidate.identity,
            now
        ])?;
    }
    Ok(())
}

/// Move back for mail the AI filed from another folder into a Meron folder:
/// it shows in the Inbox again, as if the AI had left it. Only while the copy
/// it saw is still cached there under the same UIDVALIDITY, and not for mail
/// past [`super::AI_SORT_MAX_AGE_DAYS`], which the AI never shows there.
pub fn ai_sort_inbox_show_again(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
    uidvalidity: u32,
) -> Result<()> {
    show_again(conn, account, folder, uid, Some(uidvalidity), true)
}

/// Undo of the user's own archive, delete or move of mail that was shown in
/// the Inbox from `folder`: its copies, just moved back there under new UIDs,
/// show in the Inbox again. Old mail too: it was there before the action.
pub fn ai_sort_inbox_show_again_after_undo(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> Result<()> {
    for uid in uids {
        show_again(conn, account, folder, *uid, None, false)?;
    }
    Ok(())
}

fn show_again(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
    uidvalidity: Option<u32>,
    check_age: bool,
) -> Result<()> {
    if folder.is_empty() || folder.eq_ignore_ascii_case("INBOX") || uid == 0 {
        return Ok(());
    }
    let Some(current) = super::get_folder_state(conn, account, folder)?.map(|(validity, _)| validity)
    else {
        return Ok(());
    };
    if uidvalidity.is_some_and(|uidvalidity| uidvalidity != current) {
        return Ok(());
    }
    let row: Option<(String, i64)> = conn
        .query_row(
            &format!(
                "SELECT {}, COALESCE(date, 0) FROM messages
                  WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                super::ai_sort::IDENTITY_SQL
            ),
            params![account, folder, uid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((identity, date)) = row else {
        return Ok(());
    };
    if check_age && super::ai_sort_too_old(date, super::now_unix()) {
        return Ok(());
    }
    conn.execute(
        "INSERT OR REPLACE INTO ai_sort_inbox(account, folder, uid, uidvalidity, identity, at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
        params![account, folder, uid, current, identity, super::now_unix()],
    )?;
    Ok(())
}

/// Whether the candidate's message is cached in the Inbox too.
fn in_inbox(conn: &Connection, account: &str, candidate: &AiSortCandidate) -> Result<bool> {
    let message_id = candidate.message_id.trim();
    if message_id.is_empty() {
        return Ok(false);
    }
    Ok(super::ai_sort_find_cached(conn, account, "INBOX", message_id)?.is_some())
}

/// The UIDs of `folder` shown in the Inbox. Empty for INBOX itself.
pub fn ai_sort_inbox_uids(conn: &Connection, account: &str, folder: &str) -> Result<BTreeSet<u32>> {
    if folder.eq_ignore_ascii_case("INBOX") {
        return Ok(BTreeSet::new());
    }
    let mut stmt = conn
        .prepare_cached("SELECT uid FROM ai_sort_inbox WHERE account = ?1 AND folder = ?2")?;
    let rows = stmt.query_map(params![account, folder], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The unread mail shown in the Inbox from other folders, per folder: what the
/// Inbox's "Mark all read" marks read too, where each message really is.
pub fn ai_sort_inbox_unseen(
    conn: &Connection,
    account: &str,
) -> Result<std::collections::BTreeMap<String, Vec<u32>>> {
    let mut out: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
    if !ai_sort_inbox_has_rows(conn, account)? {
        return Ok(out);
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT m.folder, m.uid FROM ai_sort_inbox x
           JOIN messages m ON m.account = x.account AND m.folder = x.folder AND m.uid = x.uid
          WHERE x.account = ?1 AND m.seen = 0 AND NOT {hidden}",
        hidden = super::meron_hidden_sql("m"),
    ))?;
    let rows = stmt.query_map(params![account], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    })?;
    for row in rows {
        let (folder, uid) = row?;
        out.entry(folder).or_default().push(uid);
    }
    Ok(out)
}

/// Whether a copy of this message (same Message-ID) is cached in the Inbox: the
/// Inbox's own sync holds it, sorts it or shows it, so its copy in another
/// folder (a Gmail label) isn't sorted, or paid for, a second time.
pub(super) fn ai_sort_seen_in_inbox(
    conn: &Connection,
    account: &str,
    header: &MessageHeader,
) -> Result<bool> {
    let message_id = header.message_id.trim();
    if message_id.is_empty() {
        return Ok(false);
    }
    Ok(super::ai_sort_find_cached(conn, account, "INBOX", message_id)?.is_some())
}

/// Keep the shown rows of `folder` true to what a sync just stored:
/// - INBOX: what now shows in the Inbox itself is no longer shown from its
///   label folder (Gmail), so it doesn't list twice.
/// - Another folder: after a UIDVALIDITY reset each row finds its message
///   again by identity; a row whose message is gone from the folder (deleted
///   or moved, here or elsewhere) goes.
/// Stop showing in the Inbox the account's mail from other folders that is
/// past [`super::AI_SORT_MAX_AGE_DAYS`] (shown before that rule existed, or
/// grown old since). Its cached row's date decides; uncached rows stay.
pub fn ai_sort_inbox_drop_old(conn: &Connection, account: &str, now: i64) -> Result<usize> {
    Ok(conn
        .prepare_cached(
            "DELETE FROM ai_sort_inbox
              WHERE account = ?1
                AND EXISTS (SELECT 1 FROM messages m
                     WHERE m.account = ai_sort_inbox.account
                       AND m.folder = ai_sort_inbox.folder
                       AND m.uid = ai_sort_inbox.uid
                       AND COALESCE(m.date, 0) > 0 AND m.date <= ?2 AND m.date < ?3)",
        )?
        .execute(params![account, now, super::ai_sort_age_cutoff(now)])?)
}

/// At app start, before any list is read: stop showing old mail in the Inbox
/// for every account ([`ai_sort_inbox_drop_old`], which otherwise only runs
/// when a folder syncs), and the one-time release of old AI-filed Meron-folder
/// conversations ([`super::meron_release_old_ai_entries`]). Best effort: a
/// failure is logged and the app goes on.
pub fn tidy_at_start(conn: &Connection) {
    let now = super::now_unix();
    let accounts: Result<Vec<String>> = (|| {
        Ok(conn
            .prepare("SELECT DISTINCT account FROM ai_sort_inbox")?
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    })();
    match accounts {
        Ok(accounts) => {
            for account in accounts {
                if let Err(err) = ai_sort_inbox_drop_old(conn, &account, now) {
                    crate::mlog!(
                        crate::log::Level::Warn,
                        "store",
                        "dropping old shown-in-Inbox mail of {account}: {err:#}"
                    );
                }
            }
        }
        Err(err) => crate::mlog!(crate::log::Level::Warn, "store", "shown-in-Inbox accounts: {err:#}"),
    }
    match super::meron_release_old_ai_entries(conn, now) {
        Ok(0) => {}
        Ok(released) => crate::mlog!(
            crate::log::Level::Info,
            "store",
            "released {released} old AI-filed Meron folder conversation(s)"
        ),
        Err(err) => crate::mlog!(
            crate::log::Level::Warn,
            "store",
            "releasing old AI-filed Meron folder conversations: {err:#}"
        ),
    }
}

pub fn ai_sort_inbox_after_sync(
    conn: &Connection,
    account: &str,
    folder: &str,
    uidvalidity: u32,
    uid_next: u32,
    headers: &[MessageHeader],
) -> Result<()> {
    if !ai_sort_inbox_has_rows(conn, account)? {
        return Ok(());
    }
    ai_sort_inbox_drop_old(conn, account, super::now_unix())?;
    if folder.eq_ignore_ascii_case("INBOX") {
        let mut forget = conn.prepare_cached(
            "DELETE FROM ai_sort_inbox WHERE account = ?1 AND identity = ?2",
        )?;
        for header in headers {
            forget.execute(params![account, super::messages::message_identity(header, folder)])?;
            // A copy read without Gmail's id names itself by its Message-ID.
            let message_id = header.message_id.trim();
            if !message_id.is_empty() {
                forget.execute(params![
                    account,
                    format!("message-id:{}", message_id.to_lowercase())
                ])?;
            }
        }
        return Ok(());
    }
    let stale: Vec<(u32, String)> = conn
        .prepare(
            "SELECT uid, identity FROM ai_sort_inbox
              WHERE account = ?1 AND folder = ?2 AND uidvalidity <> ?3",
        )?
        .query_map(params![account, folder, uidvalidity], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (uid, identity) in stale {
        conn.execute(
            "DELETE FROM ai_sort_inbox WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid],
        )?;
        if identity.is_empty() || identity.starts_with("uid:") {
            continue;
        }
        let found: Option<u32> = conn
            .query_row(
                &format!(
                    "SELECT uid FROM messages
                      WHERE account = ?1 AND folder = ?2 AND uid <> 0 AND ({}) = ?3
                      LIMIT 1",
                    super::ai_sort::IDENTITY_SQL
                ),
                params![account, folder, identity],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(found) = found {
            conn.execute(
                "INSERT OR REPLACE INTO ai_sort_inbox(account, folder, uid, uidvalidity, identity, at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![account, folder, found, uidvalidity, identity, super::now_unix()],
            )?;
        }
    }
    // Rows below this sync's UIDNEXT whose message the cache no longer holds
    // here: the sync's prune (or a move or delete in Meron) took it out.
    conn.execute(
        "DELETE FROM ai_sort_inbox
          WHERE account = ?1 AND folder = ?2 AND uid < ?3
            AND NOT EXISTS (SELECT 1 FROM messages m
                 WHERE m.account = ai_sort_inbox.account AND m.folder = ai_sort_inbox.folder
                   AND m.uid = ai_sort_inbox.uid)",
        params![account, folder, uid_next],
    )?;
    Ok(())
}

/// Stop showing `(folder, uid)` rows in the Inbox: the user filed the
/// conversation into a Meron folder.
pub fn ai_sort_inbox_drop(
    conn: &Connection,
    account: &str,
    rows: &BTreeSet<(String, u32)>,
) -> Result<()> {
    if rows.is_empty() || !ai_sort_inbox_has_rows(conn, account)? {
        return Ok(());
    }
    let mut forget = conn.prepare_cached(
        "DELETE FROM ai_sort_inbox WHERE account = ?1 AND folder = ?2 AND uid = ?3",
    )?;
    for (folder, uid) in rows {
        forget.execute(params![account, folder, uid])?;
    }
    Ok(())
}

/// A folder deleted or renamed: its watch and its shown rows go.
pub(super) fn ai_sort_inbox_forget_folder(conn: &Connection, account: &str, folder: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM ai_sort_inbox WHERE account = ?1 AND folder = ?2",
        params![account, folder],
    )?;
    conn.execute(
        "DELETE FROM ai_sort_watch WHERE account = ?1 AND folder = ?2",
        params![account, folder],
    )?;
    Ok(())
}

/// A removed account: its watches and shown rows go.
pub(super) fn ai_sort_inbox_forget_account(conn: &Connection, account: &str) -> Result<()> {
    conn.execute("DELETE FROM ai_sort_inbox WHERE account = ?1", params![account])?;
    conn.execute("DELETE FROM ai_sort_watch WHERE account = ?1", params![account])?;
    Ok(())
}
