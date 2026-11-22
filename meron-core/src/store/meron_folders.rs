//! Meron folders: folders that exist only on this device (and its backup).
//!
//! A conversation put in one is an *entry* (`meron_folder_entries`).
//!
//! The entry finds its messages through *aliases* (`meron_folder_aliases`): the conversation's
//! thread root key (`lower(COALESCE(thread_key, ''))`, the expression of `messages_thread_key_idx`)
//! and every member message's tag key ([`super::tags::TAG_KEY_SQL`]: its Message-ID, or
//! `uid:<folder>:<uid>` for mail without one, the expression of `messages_tag_key_idx`).
//!
//! A message is in a Meron folder when either of its two keys is an alias; both checks are
//! primary-key probes. Listing a folder joins from the aliases into `messages` over the same two
//! indexes.
//!
//! The entry follows its conversation: [`meron_adopt`] runs inside every `upsert_messages`, over
//! the rows it wrote and the rows thread reconciliation re-keyed, and adds aliases for any row one
//! of whose keys is already an alias. Message-ID aliases don't depend on cached rows, so they
//! survive a UIDVALIDITY reset's delete and reinsert.
//!
//! `meron_folder_uids` remembers the UIDs of member messages so the unread counts can leave out
//! server-unseen mail the cache doesn't hold. Cache eviction never deletes from it; release does,
//! and a UIDVALIDITY reset of that folder.
//!
//! Nothing here touches the mail server.

mod membership;
pub use membership::{
    meron_entry_members, meron_entry_of_key, meron_entry_of_row, meron_entry_of_thread,
    meron_folder_entries, meron_folder_members, meron_folder_of_rows, meron_folder_unseen_uids,
    meron_foldered_uids, meron_header_in_folder, meron_mark_seen, meron_row_header,
    meron_uid_in_folder, meron_unread_by_folder, retain_outside_meron_folders,
};
mod filing;
use filing::{conversation_rows, entry_of_alias, merge_entry, put_alias, put_uid};
pub use filing::{
    meron_conversation_has_inbox_copy, meron_conversation_inbox_rows, meron_move_in,
    meron_move_in_in,
};
mod release;
pub use release::{
    meron_card_release, meron_entry_folder_id, meron_queue_releases, meron_refile,
    meron_release_card, meron_release_entry, meron_release_entry_in, meron_release_old_ai_entries,
    meron_release_thread, meron_retry_pending_releases,
};
mod backup;
pub use backup::{meron_backup_collect, meron_backup_restore};

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::imap::MessageHeader;

use super::tags::{TAG_KEY_SQL, qualified_tag_key};

/// What every Meron folder id starts with.
pub const MERON_FOLDER_ID_PREFIX: &str = "mf_";
/// What every Meron folder entry id starts with.
pub const MERON_ENTRY_ID_PREFIX: &str = "mfe_";
/// Characters a Meron folder name keeps.
pub const MERON_FOLDER_MAX_NAME: usize = 80;
/// The built-in "Archive" Meron folder: archived whole conversations are
/// filed here. Always exists, can't be renamed or deleted. Its mark keyword
/// is `$MeronF_archive`, which no uuid id can clash with.
pub const MERON_ARCHIVE_FOLDER_ID: &str = "mf_archive";
/// The built-in Archive folder's stored name (the apps show their own,
/// translated, name).
const MERON_ARCHIVE_FOLDER_NAME: &str = "Archive";
/// What the built-in can't do.
const BUILTIN_REFUSAL: &str = "The Archive folder can't be changed.";

/// Whether `id` is a built-in Meron folder (only Archive).
pub fn meron_is_builtin(id: &str) -> bool {
    id == MERON_ARCHIVE_FOLDER_ID
}

/// Make sure the built-in Archive folder exists. A user folder already named
/// "Archive" becomes "Archive 2" (3, 4, … when taken). Runs on every open of
/// the main store, after the migrations.
pub fn ensure_meron_archive_folder(conn: &Connection) -> Result<()> {
    if meron_folder_exists(conn, MERON_ARCHIVE_FOLDER_ID)? {
        return Ok(());
    }
    let clashing: Vec<String> = conn
        .prepare("SELECT id FROM meron_folders WHERE lower(name) = 'archive' AND id <> ?1")?
        .query_map(params![MERON_ARCHIVE_FOLDER_ID], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in clashing {
        let mut n = 2;
        loop {
            let name = format!("{MERON_ARCHIVE_FOLDER_NAME} {n}");
            if !name_taken(conn, &name, Some(&id))? {
                conn.execute(
                    "UPDATE OR IGNORE meron_folders SET name = ?2 WHERE id = ?1",
                    params![id, name],
                )?;
                break;
            }
            n += 1;
        }
    }
    conn.execute(
        "INSERT OR IGNORE INTO meron_folders(id, name, sort_order, created_at, notify)
         VALUES(?1, ?2, 0, ?3, 0)",
        params![
            MERON_ARCHIVE_FOLDER_ID,
            MERON_ARCHIVE_FOLDER_NAME,
            super::now_unix()
        ],
    )?;
    Ok(())
}

/// One Meron folder.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeronFolder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub created_at: i64,
    /// New mail AI sorting files here notifies, as an opted-in server folder's
    /// does. Off unless turned on; this device only.
    #[serde(default)]
    pub notify: bool,
}

/// `SELECT` columns of a [`MeronFolder`], in [`meron_folder_row`]'s order.
const MERON_FOLDER_COLUMNS: &str = "id, name, sort_order, created_at, notify";

fn meron_folder_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeronFolder> {
    Ok(MeronFolder {
        id: row.get(0)?,
        name: row.get(1)?,
        sort_order: row.get(2)?,
        created_at: row.get(3)?,
        notify: row.get::<_, i64>(4)? != 0,
    })
}

// ---- Keys ---------------------------------------------------------------------

/// The thread-root key of the `messages` row aliased `alias`. Character for
/// character the expression of `messages_thread_key_idx`.
pub fn meron_root_key_sql(alias: &str) -> String {
    format!("lower(COALESCE({alias}.thread_key, ''))")
}

/// The tag key of the `messages` row aliased `alias` (see
/// [`super::tags::TAG_KEY_SQL`]).
pub fn meron_tag_key_sql(alias: &str) -> String {
    qualified_tag_key(alias)
}

/// SQL that is true when the `messages` row aliased `alias` is in a Meron
/// folder: its root key or its tag key is an alias. Two separate primary-key
/// probes, no OR inside one lookup.
pub fn meron_hidden_sql(alias: &str) -> String {
    format!(
        "(EXISTS (SELECT 1 FROM meron_folder_aliases mfa
                   WHERE mfa.account = {alias}.account AND mfa.key = {root})
          OR EXISTS (SELECT 1 FROM meron_folder_aliases mfa
                   WHERE mfa.account = {alias}.account AND mfa.key = {tag}))",
        root = meron_root_key_sql(alias),
        tag = meron_tag_key_sql(alias),
    )
}

/// Whether `key` can be an alias. A bare `uid:<n>` thread key names nothing
/// outside its own mailbox, and an empty one nothing at all; sender rows are
/// not conversations.
fn is_alias_key(key: &str) -> bool {
    if key.is_empty() || key.starts_with(super::SENDER_KEY_PREFIX) {
        return false;
    }
    match key.strip_prefix("uid:") {
        Some(rest) => !rest.bytes().all(|byte| byte.is_ascii_digit()),
        None => true,
    }
}

/// A conversation key as aliases spell it: tag keys of mail without a
/// Message-ID (`uid:<folder>:<uid>`) keep their folder's case, everything
/// else is lowercased like the root-key index.
fn normalize_key(key: &str) -> String {
    let key = key.trim();
    if key.starts_with("uid:") {
        key.to_string()
    } else {
        key.to_lowercase()
    }
}

/// Whether Meron folders hide mail from a folder of this role. They never hide
/// from Drafts, Trash or Junk.
pub fn meron_hides_role(role: &str) -> bool {
    !matches!(role, "drafts" | "trash" | "junk")
}

/// Whether the account has any conversation in a Meron folder. Cheap (a
/// primary-key prefix probe), so list and count queries skip the hide rule
/// entirely for accounts that don't.
pub fn meron_account_has_entries(conn: &Connection, account: &str) -> Result<bool> {
    Ok(conn
        .prepare_cached("SELECT 1 FROM meron_folder_aliases WHERE account = ?1 LIMIT 1")?
        .exists(params![account])?)
}

/// Whether lists and counts of `folder` must leave Meron-folder mail out:
/// the folder's role hides it and the account has some.
pub fn meron_hides_folder(conn: &Connection, account: &str, folder: &str) -> Result<bool> {
    if super::is_local_account(account) || !meron_account_has_entries(conn, account)? {
        return Ok(false);
    }
    Ok(meron_hides_role(&super::folder_role(
        conn, account, folder,
    )?))
}

/// The conversation key of one cached message: its thread root, or its tag
/// key when it has no real root. `None` when the row isn't cached.
pub fn meron_conversation_key(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
) -> Result<Option<String>> {
    let row: Option<(String, String)> = conn
        .prepare_cached(&format!(
            "SELECT lower(COALESCE(thread_key, '')), {TAG_KEY_SQL} FROM messages
              WHERE account = ?1 AND folder = ?2 AND uid = ?3"
        ))?
        .query_row(params![account, folder, uid], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?;
    Ok(row.map(|(root, tag)| if is_alias_key(&root) { root } else { tag }))
}

/// The conversation key a thread id names: its root key, or for a `uid:`
/// thread (mail without threading headers) the message's own key, found
/// through `folder`. `uid` is the thread id's UID, when it has one.
pub fn meron_thread_key(
    conn: &Connection,
    account: &str,
    folder: &str,
    root: &str,
    uid: Option<u32>,
) -> Result<Option<String>> {
    let uid = uid.or_else(|| root.strip_prefix("uid:").and_then(|n| n.parse().ok()));
    if let Some(uid) = uid {
        return meron_conversation_key(conn, account, folder, uid);
    }
    let key = normalize_key(root);
    Ok(is_alias_key(&key).then_some(key))
}

// ---- Folders ------------------------------------------------------------------

fn clean_name(name: &str) -> Result<String> {
    let name: String = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MERON_FOLDER_MAX_NAME)
        .collect();
    if name.is_empty() {
        bail!("Enter a name");
    }
    Ok(name)
}

fn name_taken(conn: &Connection, name: &str, except: Option<&str>) -> Result<bool> {
    let wanted = name.to_lowercase();
    Ok(list_meron_folders(conn)?
        .into_iter()
        .any(|folder| Some(folder.id.as_str()) != except && folder.name.to_lowercase() == wanted))
}

/// Every Meron folder, A–Z, the built-in Archive last.
pub fn list_meron_folders(conn: &Connection) -> Result<Vec<MeronFolder>> {
    let mut stmt =
        conn.prepare_cached(&format!("SELECT {MERON_FOLDER_COLUMNS} FROM meron_folders"))?;
    let mut folders = stmt
        .query_map([], meron_folder_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    folders.sort_by(|a, b| {
        meron_is_builtin(&a.id)
            .cmp(&meron_is_builtin(&b.id))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(folders)
}

/// `(id, name)` of every Meron folder, A–Z: what AI sorting offers. The
/// built-in Archive isn't offered.
pub fn meron_folder_names(conn: &Connection) -> Result<Vec<(String, String)>> {
    Ok(list_meron_folders(conn)?
        .into_iter()
        .filter(|folder| !meron_is_builtin(&folder.id))
        .map(|folder| (folder.id, folder.name))
        .collect())
}

pub fn meron_folder_name(conn: &Connection, id: &str) -> Result<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT name FROM meron_folders WHERE id = ?1")?
        .query_row(params![id], |row| row.get(0))
        .optional()?)
}

pub fn meron_folder_exists(conn: &Connection, id: &str) -> Result<bool> {
    Ok(meron_folder_name(conn, id)?.is_some())
}

pub fn create_meron_folder(conn: &Connection, name: &str) -> Result<MeronFolder> {
    let name = clean_name(name)?;
    if name_taken(conn, &name, None)? {
        bail!("A Meron folder named “{name}” already exists");
    }
    let folder = MeronFolder {
        id: format!("{MERON_FOLDER_ID_PREFIX}{}", uuid::Uuid::new_v4().simple()),
        name,
        sort_order: 0,
        created_at: super::now_unix(),
        notify: false,
    };
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO meron_folders(id, name, sort_order, created_at)
         VALUES(?1, ?2, ?3, ?4)",
        params![folder.id, folder.name, folder.sort_order, folder.created_at],
    )?;
    if inserted == 0 {
        bail!("A Meron folder named “{}” already exists", folder.name);
    }
    Ok(folder)
}

pub fn rename_meron_folder(conn: &Connection, id: &str, name: &str) -> Result<MeronFolder> {
    if meron_is_builtin(id) {
        bail!("{BUILTIN_REFUSAL}");
    }
    let name = clean_name(name)?;
    if !meron_folder_exists(conn, id)? {
        bail!("That Meron folder is gone.");
    }
    if name_taken(conn, &name, Some(id))? {
        bail!("A Meron folder named “{name}” already exists");
    }
    let changed = conn.execute(
        "UPDATE OR IGNORE meron_folders SET name = ?2 WHERE id = ?1",
        params![id, name],
    )?;
    if changed == 0 {
        bail!("A Meron folder named “{name}” already exists");
    }
    Ok(conn.query_row(
        &format!("SELECT {MERON_FOLDER_COLUMNS} FROM meron_folders WHERE id = ?1"),
        params![id],
        meron_folder_row,
    )?)
}

/// Turn new-mail notifications for a Meron folder on or off. False when the
/// folder is gone.
pub fn set_meron_folder_notify(conn: &Connection, id: &str, enabled: bool) -> Result<bool> {
    if meron_is_builtin(id) {
        bail!("{BUILTIN_REFUSAL}");
    }
    Ok(conn.execute(
        "UPDATE meron_folders SET notify = ?2 WHERE id = ?1",
        params![id, enabled as i64],
    )? > 0)
}

/// Whether any Meron folder has notifications on.
pub fn meron_any_folder_notifies(conn: &Connection) -> Result<bool> {
    Ok(conn
        .prepare_cached("SELECT 1 FROM meron_folders WHERE notify <> 0 LIMIT 1")?
        .exists([])?)
}

/// Whether new mail AI sorting files into this Meron folder notifies.
pub fn meron_folder_notifies(conn: &Connection, id: &str) -> Result<bool> {
    Ok(conn
        .prepare_cached("SELECT notify FROM meron_folders WHERE id = ?1")?
        .query_row(params![id], |row| row.get::<_, i64>(0))
        .optional()?
        .is_some_and(|notify| notify != 0))
}

/// What releasing conversations from Meron folders changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeronReleased {
    /// Entries (conversations) released.
    pub released: usize,
    /// The `(account, folder)` mailboxes their messages sit in, whose unread
    /// counts changed.
    pub touched: BTreeSet<(String, String)>,
}

impl MeronReleased {
    fn add(&mut self, other: MeronReleased) {
        self.released += other.released;
        self.touched.extend(other.touched);
    }
}

/// Delete a Meron folder: its conversations go back where they are, the AI
/// stops sorting into it. One transaction.
pub fn delete_meron_folder(conn: &Connection, id: &str) -> Result<MeronReleased> {
    if meron_is_builtin(id) {
        bail!("{BUILTIN_REFUSAL}");
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let entries: Vec<String> = tx
        .prepare("SELECT id FROM meron_folder_entries WHERE folder_id = ?1")?
        .query_map(params![id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut out = MeronReleased::default();
    for entry in entries {
        out.add(meron_release_entry_in(&tx, &entry)?);
    }
    tx.execute("DELETE FROM meron_folders WHERE id = ?1", params![id])?;
    super::ai_sort_forget_meron_folder(&tx, id)?;
    tx.commit()?;
    Ok(out)
}

/// Entries (conversations) per Meron folder.
pub fn meron_folder_counts(conn: &Connection) -> Result<HashMap<String, u32>> {
    let mut stmt =
        conn.prepare("SELECT folder_id, COUNT(*) FROM meron_folder_entries GROUP BY folder_id")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (folder, count) = row?;
        out.insert(folder, count as u32);
    }
    Ok(out)
}

// ---- Members ------------------------------------------------------------------

/// One cached message of a Meron-folder conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct MeronMember {
    pub folder_id: String,
    pub entry_id: String,
    pub account: String,
    pub folder: String,
    pub uid: u32,
    pub date: i64,
    pub seen: bool,
    /// Its tag key: one message's identity across its copies.
    pub tag_key: String,
}

/// Caches folder roles while a query walks many rows.
#[derive(Default)]
struct Roles(HashMap<(String, String), String>);

impl Roles {
    fn hides(&mut self, conn: &Connection, account: &str, folder: &str) -> Result<bool> {
        let key = (account.to_string(), folder.to_string());
        if !self.0.contains_key(&key) {
            let role = super::folder_role(conn, account, folder)?;
            self.0.insert(key.clone(), role);
        }
        Ok(meron_hides_role(&self.0[&key]))
    }
}

/// One conversation of a Meron folder, for paging: its newest message's date
/// and where its messages are (Drafts, Trash and Junk left out).
#[derive(Clone, Debug, PartialEq)]
pub struct MeronEntryRows {
    pub entry_id: String,
    pub account: String,
    pub newest: i64,
    /// `(folder, uid)` of each cached message.
    pub rows: Vec<(String, u32)>,
}

// ---- Where a message is -------------------------------------------------------

// ---- Moving in and out --------------------------------------------------------

/// What moving a conversation into a Meron folder did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeronMovedIn {
    pub entry_id: String,
    /// The Meron folder it was in before, when it moved between two.
    pub previous_folder: Option<String>,
    /// The mailboxes its cached messages sit in.
    pub touched: BTreeSet<String>,
}

/// One cached message: row id, folder, uid, root key, tag key, date.
type KeyedRow = (i64, String, u32, String, String, i64);

/// Who put a conversation into a Meron folder: `meron_folder_entries.source`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MeronFiling<'a> {
    /// The user, by hand (or an Undo putting back what the user had).
    User,
    /// AI sorting, filing new mail of `folder` at `now`.
    Ai { folder: &'a str, now: i64 },
}

impl MeronFiling<'_> {
    pub fn source(&self) -> &'static str {
        match self {
            MeronFiling::User => "user",
            MeronFiling::Ai { .. } => "ai",
        }
    }

    /// Whether this filing leaves `row` out: the AI pulls in no mail past
    /// [`super::AI_SORT_MAX_AGE_DAYS`] from folders other than the one it
    /// sorted, which would otherwise come along as old cards.
    fn leaves_out(&self, row: &KeyedRow) -> bool {
        match self {
            MeronFiling::User => false,
            MeronFiling::Ai { folder, now } => {
                row.1 != *folder && super::ai_sort_too_old(row.5, *now)
            }
        }
    }
}

/// What taking one card out of its Meron folder removes: the card's
/// conversation (as [`meron_move_in_in`] gathers it for the card's key), not
/// the whole entry, which can hold more conversations than this one card.
#[derive(Clone, Debug, PartialEq)]
pub struct MeronCardRelease {
    pub entry_id: String,
    /// The Meron folder the card was in, for an Undo to put it back.
    pub folder_id: String,
    /// The entry's `source` ("ai" or "user"), for that Undo too.
    pub source: String,
    /// The entry's aliases that are this card's.
    pub aliases: BTreeSet<String>,
    /// `(folder, uid)` of the card's cached messages.
    pub rows: BTreeSet<(String, u32)>,
}

/// `meta` key set once [`meron_release_old_ai_entries`] has run.
const OLD_AI_ENTRIES_DONE_KEY: &str = "meron_old_ai_entries_released";

/// Seconds between an entry's `added_at` and the `ai_sort_done` row of one of
/// its messages for the AI to count as having filed it: the AI writes both in
/// one transaction.
const AI_FILED_WINDOW_SECS: i64 = 1;

/// `meta` key holding releases that failed, as a JSON object of entry id to
/// the Meron folder the entry was in then.
const PENDING_RELEASES_KEY: &str = "meron_pending_releases";

/// Forget a removed account's Meron-folder entries.
pub fn meron_forget_account(conn: &Connection, account: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM meron_folder_entries WHERE account = ?1",
        params![account],
    )?;
    conn.execute(
        "DELETE FROM meron_folder_aliases WHERE account = ?1",
        params![account],
    )?;
    conn.execute(
        "DELETE FROM meron_folder_uids WHERE account = ?1",
        params![account],
    )?;
    conn.execute(
        "DELETE FROM meron_marks WHERE account = ?1",
        params![account],
    )?;
    Ok(())
}

/// A folder's UIDs were reset: its remembered member UIDs name other mail now.
///
/// So do its location aliases (`uid:<folder>:<uid>`, the key of mail without
/// a Message-ID): a reused UID would otherwise file or hide an unrelated
/// message. An entry left with no alias at all is gone with them. Message-ID
/// and root aliases stay, so sync re-adopts the rest of the mail.
pub fn meron_clear_folder_uids(conn: &Connection, account: &str, folder: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM meron_folder_uids WHERE account = ?1 AND folder = ?2",
        params![account, folder],
    )?;
    conn.execute(
        "DELETE FROM meron_marks WHERE account = ?1 AND folder = ?2",
        params![account, folder],
    )?;
    let prefix = format!("uid:{folder}:");
    let removed = conn.execute(
        "DELETE FROM meron_folder_aliases
          WHERE account = ?1 AND substr(key, 1, length(?2)) = ?2",
        params![account, prefix],
    )?;
    if removed > 0 {
        conn.execute(
            "DELETE FROM meron_folder_uids
              WHERE account = ?1 AND entry_id IN (
                SELECT e.id FROM meron_folder_entries e
                 WHERE e.account = ?1
                   AND NOT EXISTS (SELECT 1 FROM meron_folder_aliases al WHERE al.entry_id = e.id))",
            params![account],
        )?;
        conn.execute(
            "DELETE FROM meron_folder_entries
              WHERE account = ?1
                AND NOT EXISTS (SELECT 1 FROM meron_folder_aliases al WHERE al.entry_id = meron_folder_entries.id)",
            params![account],
        )?;
    }
    Ok(())
}

// ---- Following the conversation -----------------------------------------------

/// Attach newly upserted or re-keyed rows to an existing local folder entry.
///
/// Match aliases against the old root, new root and tag key. For a match, remember the new
/// keys and UID. A row that joins two entries merges them into the most recently added entry.
///
/// Each row ID carries its previous root when available. All writes use the upsert transaction.
pub(super) fn meron_adopt(
    conn: &Connection,
    account: &str,
    rows: &[(i64, Option<String>)],
) -> Result<()> {
    if rows.is_empty() || !meron_account_has_entries(conn, account)? {
        return Ok(());
    }
    let mut read = conn.prepare_cached(&format!(
        "SELECT folder, uid, lower(COALESCE(thread_key, '')), {TAG_KEY_SQL}
           FROM messages WHERE id = ?1"
    ))?;
    for (id, old_root) in rows {
        let Some((folder, uid, root, tag)) = read
            .query_row(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .optional()?
        else {
            continue;
        };
        if uid == 0 {
            continue;
        }
        let mut found: Vec<(String, i64)> = Vec::new();
        let old = old_root.as_deref().map(str::to_lowercase);
        for key in old.iter().chain([&root, &tag]) {
            if is_alias_key(key)
                && let Some(entry) = entry_of_alias(conn, account, key)?
                && !found.contains(&entry)
            {
                found.push(entry);
            }
        }
        let Some(winner) = found
            .iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)))
            .map(|(id, _)| id.clone())
        else {
            continue;
        };
        for (other, _) in &found {
            merge_entry(conn, other, &winner)?;
        }
        // Never in the built-in Archive while it has Inbox mail (a new reply,
        // an un-archive in webmail or another app, an Undo): the conversation
        // leaves Archive, and its marks go once the transaction is over.
        // Not for the Inbox copies that were archived themselves (archiving is
        // Meron-only on an account with no archive mailbox, and they stay in
        // the Inbox): those are remembered members already, a new one isn't.
        if folder.eq_ignore_ascii_case("INBOX")
            && meron_entry_folder_id(conn, &winner)?.as_deref() == Some(MERON_ARCHIVE_FOLDER_ID)
            && !uid_is_member(conn, account, &folder, uid, &winner)?
        {
            queue_archive_entry_unmarks(conn, account, &winner)?;
            meron_release_entry_in(conn, &winner)?;
            continue;
        }
        put_alias(conn, account, &root, &winner)?;
        put_alias(conn, account, &tag, &winner)?;
        put_uid(conn, account, &folder, uid, &tag, &winner)?;
    }
    Ok(())
}

/// Whether `(folder, uid)` is remembered as a member of entry `entry_id`.
fn uid_is_member(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
    entry_id: &str,
) -> Result<bool> {
    Ok(conn
        .prepare_cached(
            "SELECT 1 FROM meron_folder_uids
              WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND entry_id = ?4",
        )?
        .exists(params![account, folder, uid, entry_id])?)
}

/// Before a built-in Archive entry is released inside an upsert: queue the unmark of its cached
/// copies in the Inbox and the archive mailbox (see [`crate::meron_marks::queue_archive_unmark`]).
/// All of them, not only those this device marked: mail moved back by another app keeps the
/// Archive's mark, which this device never remembered.
fn queue_archive_entry_unmarks(conn: &Connection, account: &str, entry_id: &str) -> Result<()> {
    let archive = super::meron_archive_mailbox(conn, account);
    let mut by_folder: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for member in meron_entry_members(conn, entry_id)? {
        let marks_here = member.folder.eq_ignore_ascii_case("INBOX")
            || archive.as_deref() == Some(member.folder.as_str());
        if marks_here {
            by_folder.entry(member.folder).or_default().push(member.uid);
        }
    }
    for (folder, mut uids) in by_folder {
        uids.sort_unstable();
        uids.dedup();
        crate::meron_marks::queue_archive_unmark(crate::meron_marks::MarkJob {
            account: account.to_string(),
            folder,
            uids,
            target: None,
        });
    }
    Ok(())
}

/// Row ids of `uids` in `folder`, for [`meron_adopt`]; empty when the account
/// has nothing in Meron folders.
pub(super) fn meron_row_ids(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: impl IntoIterator<Item = u32>,
) -> Result<Vec<(i64, Option<String>)>> {
    if !meron_account_has_entries(conn, account)? {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare_cached(
        "SELECT id FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
    )?;
    let mut out = Vec::new();
    for uid in uids {
        if let Some(id) = stmt
            .query_row(params![account, folder, uid], |row| row.get::<_, i64>(0))
            .optional()?
        {
            out.push((id, None));
        }
    }
    Ok(out)
}

// ---- Backup -------------------------------------------------------------------

/// One conversation of a Meron folder, as a backup carries it: the entry and
/// every alias that finds its messages.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeronBackupEntry {
    pub id: String,
    pub account: String,
    #[serde(default)]
    pub added_at: i64,
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// One Meron folder and its conversations, as a backup carries it.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeronBackupFolder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub created_at: i64,
    /// Older backups don't carry it: off.
    #[serde(default)]
    pub notify: bool,
    #[serde(default)]
    pub entries: Vec<MeronBackupEntry>,
}
