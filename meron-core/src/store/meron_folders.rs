//! Meron folders: folders that exist only on this device (and its backup).
//!
//! A conversation put in one is an *entry* (`meron_folder_entries`). The entry
//! finds its messages through *aliases* (`meron_folder_aliases`): the
//! conversation's thread root key (`lower(COALESCE(thread_key, ''))`, the
//! expression of `messages_thread_key_idx`) and every member message's tag key
//! ([`super::tags::TAG_KEY_SQL`]: its Message-ID, or `uid:<folder>:<uid>` for
//! mail without one, the expression of `messages_tag_key_idx`). A message is in
//! a Meron folder when either of its two keys is an alias; both checks are
//! primary-key probes. Listing a folder joins from the aliases into `messages`
//! over the same two indexes.
//!
//! The entry follows its conversation: [`meron_adopt`] runs inside every
//! `upsert_messages`, over the rows it wrote and the rows thread
//! reconciliation re-keyed, and adds aliases for any row one of whose keys is
//! already an alias. Message-ID aliases don't depend on cached rows, so they
//! survive a UIDVALIDITY reset's delete and reinsert.
//!
//! `meron_folder_uids` remembers the UIDs of member messages so the unread
//! counts can leave out server-unseen mail the cache doesn't hold. Cache
//! eviction never deletes from it; release does, and a UIDVALIDITY reset of
//! that folder.
//!
//! Nothing here touches the mail server.

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
        params![MERON_ARCHIVE_FOLDER_ID, MERON_ARCHIVE_FOLDER_NAME, super::now_unix()],
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
    Ok(meron_hides_role(&super::folder_role(conn, account, folder)?))
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
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))?;
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

/// The member rows of the entries `filter` (a condition on `e`, the entry)
/// picks: a UNION of the two indexed joins from the aliases into `messages`.
///
/// `+al.key` strips the column's TEXT affinity: compared with it as is, the
/// expression side would take that affinity, and SQLite only uses an
/// expression index (which has none) for a comparison without one, so it
/// scanned `messages`. CROSS JOIN keeps the order entries → aliases → mail.
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
    out.sort_by(|a, b| b.newest.cmp(&a.newest).then_with(|| b.entry_id.cmp(&a.entry_id)));
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
        .query_row(params![account, folder, uid], super::message_header_from_row)
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
            out.entry(account).or_default().entry(folder).or_default().insert(uid);
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

// ---- Where a message is -------------------------------------------------------

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
pub fn meron_uid_in_folder(conn: &Connection, account: &str, folder: &str, uid: u32) -> Result<bool> {
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
        let tag = raw.trim_matches(|c| c == ' ' || c == '<' || c == '>').to_string();
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
        by_folder.entry(header.folder.clone()).or_default().push(header.uid);
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

fn rows_by_root(conn: &Connection, account: &str, root: &str) -> Result<Vec<KeyedRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT id, folder, uid, lower(COALESCE(thread_key, '')), {TAG_KEY_SQL}, COALESCE(date, 0)
           FROM messages
          WHERE account = ?1 AND lower(COALESCE(thread_key, '')) = ?2 AND uid <> 0"
    ))?;
    let rows = stmt.query_map(params![account, root], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
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
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `(entry id, added_at)` of the entry an alias key belongs to.
fn entry_of_alias(conn: &Connection, account: &str, key: &str) -> Result<Option<(String, i64)>> {
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
fn merge_entry(conn: &Connection, from: &str, into: &str) -> Result<()> {
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
    conn.execute("DELETE FROM meron_folder_entries WHERE id = ?1", params![from])?;
    Ok(())
}

fn put_alias(conn: &Connection, account: &str, key: &str, entry: &str) -> Result<()> {
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
fn put_uid(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
    tag_key: &str,
    entry: &str,
) -> Result<()> {
    let validity = super::get_folder_state(conn, account, folder)?
        .map(|(validity, _)| validity)
        .unwrap_or(0);
    conn.prepare_cached(
        "INSERT OR REPLACE INTO meron_folder_uids(account, folder, uidvalidity, uid, entry_id, tag_key)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
    )?
    .execute(params![account, folder, validity, uid, entry, tag_key])?;
    Ok(())
}

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

/// The messages of the conversation `key` names, and its aliases: those under
/// the key as a root or as a message key, then the rest of every real root
/// those carry. `filing` may leave rows out (see [`MeronFiling::leaves_out`]);
/// the key itself is always an alias.
fn conversation_rows(
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
                params![id, account, folder_id, super::now_unix(), filing.source()],
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
    super::ai_sort_inbox_drop(conn, account, &located)?;
    Ok(MeronMovedIn {
        entry_id,
        previous_folder,
        touched,
    })
}

/// Whether the conversation `key` names has a cached Inbox copy.
pub fn meron_conversation_has_inbox_copy(conn: &Connection, account: &str, key: &str) -> Result<bool> {
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
pub fn meron_conversation_inbox_rows(conn: &Connection, account: &str, key: &str) -> Result<Vec<(u32, i64)>> {
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

/// The Meron-folder card a thread id names, read before an action moves its
/// mail (the rows go once it has). `None` when it isn't in a Meron folder.
pub fn meron_card_release(
    conn: &Connection,
    account: &str,
    folder: &str,
    root: &str,
    uid: Option<u32>,
) -> Result<Option<MeronCardRelease>> {
    let Some((entry_id, folder_id)) = meron_entry_of_thread(conn, account, folder, root, uid)? else {
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

/// `meta` key set once [`meron_release_old_ai_entries`] has run.
const OLD_AI_ENTRIES_DONE_KEY: &str = "meron_old_ai_entries_released";

/// Seconds between an entry's `added_at` and the `ai_sort_done` row of one of
/// its messages for the AI to count as having filed it: the AI writes both in
/// one transaction.
const AI_FILED_WINDOW_SECS: i64 = 1;

/// Once per store: find the Meron-folder entries AI sorting filed (before
/// `source` existed) and mark them `source = 'ai'`; release those whose
/// newest dated message is past [`super::AI_SORT_MAX_AGE_DAYS`] and none of
/// whose messages is in the Inbox. Hand-filed entries are never touched. One
/// transaction. Returns how many were released.
pub fn meron_release_old_ai_entries(conn: &Connection, now: i64) -> Result<usize> {
    if super::db::meta_get(conn, OLD_AI_ENTRIES_DONE_KEY)?.is_some() {
        return Ok(0);
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let entries: Vec<(String, String, i64, String)> = tx
        .prepare("SELECT id, account, added_at, source FROM meron_folder_entries")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut identity_of = tx.prepare(&format!(
        "SELECT {} FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
        super::ai_sort::IDENTITY_SQL
    ))?;
    let mut done_near = tx.prepare(
        "SELECT 1 FROM ai_sort_done
          WHERE account = ?1 AND identity = ?2 AND at BETWEEN ?3 AND ?4",
    )?;
    let cutoff = super::ai_sort_age_cutoff(now);
    let mut released = 0;
    for (entry, account, added_at, source) in entries {
        let members = meron_entry_members(&tx, &entry)?;
        let mut by_ai = source == "ai";
        if !by_ai {
            for member in &members {
                let identity: Option<String> = identity_of
                    .query_row(params![account, member.folder, member.uid], |row| row.get(0))
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
    super::db::meta_set(&tx, OLD_AI_ENTRIES_DONE_KEY, "1")?;
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
    let removed = conn.execute("DELETE FROM meron_folder_entries WHERE id = ?1", params![entry_id])?;
    conn.execute("DELETE FROM meron_folder_aliases WHERE entry_id = ?1", params![entry_id])?;
    conn.execute("DELETE FROM meron_folder_uids WHERE entry_id = ?1", params![entry_id])?;
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

/// `meta` key holding releases that failed, as a JSON object of entry id to
/// the Meron folder the entry was in then.
const PENDING_RELEASES_KEY: &str = "meron_pending_releases";

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
    Ok(super::db::meta_get(conn, PENDING_RELEASES_KEY)?
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
    super::db::meta_set(&tx, PENDING_RELEASES_KEY, &serde_json::to_string(&pending)?)?;
    tx.commit()?;
    Ok(())
}

/// Release the entries [`meron_queue_releases`] remembered. An entry the user
/// has since moved to another Meron folder (or that is gone) is left alone:
/// the release was for where it was then. Returns how many were released.
pub fn meron_retry_pending_releases(conn: &Connection) -> Result<usize> {
    // The common case, nothing queued, without taking the write lock.
    if super::db::meta_get(conn, PENDING_RELEASES_KEY)?.is_none() {
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
    conn.execute("DELETE FROM meron_folder_uids WHERE account = ?1", params![account])?;
    conn.execute("DELETE FROM meron_marks WHERE account = ?1", params![account])?;
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

/// Adopt rows `upsert_messages` just wrote or re-keyed: any row whose old root
/// key, new root key or tag key is an alias gets aliases for its new root key
/// and tag key, and its UID is remembered. A row linking two entries (thread
/// reconciliation merged two conversations) folds them into the one added
/// most recently. `rows` pairs each row id with its root key from before a
/// re-key, if any. Runs inside the upsert's transaction.
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
fn uid_is_member(conn: &Connection, account: &str, folder: &str, uid: u32, entry_id: &str) -> Result<bool> {
    Ok(conn
        .prepare_cached(
            "SELECT 1 FROM meron_folder_uids
              WHERE account = ?1 AND folder = ?2 AND uid = ?3 AND entry_id = ?4",
        )?
        .exists(params![account, folder, uid, entry_id])?)
}

/// Before a built-in Archive entry is released inside an upsert: queue the
/// unmark of its cached copies in the Inbox and the archive mailbox (see
/// [`crate::meron_marks::queue_archive_unmark`]). All of them, not only those
/// this device marked: mail moved back by another app keeps the Archive's
/// mark, which this device never remembered.
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

/// Every Meron folder with its entries and aliases.
pub fn meron_backup_collect(conn: &Connection) -> Result<Vec<MeronBackupFolder>> {
    let mut out = Vec::new();
    let mut entries_stmt = conn.prepare(
        "SELECT id, account, added_at FROM meron_folder_entries WHERE folder_id = ?1 ORDER BY id",
    )?;
    let mut aliases_stmt =
        conn.prepare("SELECT key FROM meron_folder_aliases WHERE entry_id = ?1 ORDER BY key")?;
    for folder in list_meron_folders(conn)? {
        let entries = entries_stmt
            .query_map(params![folder.id], |row| {
                Ok(MeronBackupEntry {
                    id: row.get(0)?,
                    account: row.get(1)?,
                    added_at: row.get(2)?,
                    aliases: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut filled = Vec::with_capacity(entries.len());
        for mut entry in entries {
            entry.aliases = aliases_stmt
                .query_map(params![entry.id], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            filled.push(entry);
        }
        out.push(MeronBackupFolder {
            id: folder.id,
            name: folder.name,
            sort_order: folder.sort_order,
            created_at: folder.created_at,
            notify: folder.notify,
            entries: filled,
        });
    }
    Ok(out)
}

/// Restore Meron folders from a backup, never over what is here: a folder
/// whose id or name already exists is kept and the backup's conversations go
/// into it; an alias this device already files elsewhere stays there. Entries
/// of unknown accounts are skipped. Answers with how many folders were added
/// and the backup id → kept id map (for AI sorting's setting).
pub fn meron_backup_restore(
    conn: &Connection,
    folders: &[MeronBackupFolder],
) -> Result<(u32, HashMap<String, String>)> {
    let mut added = 0;
    let mut ids = HashMap::new();
    for folder in folders {
        let id = folder.id.trim();
        let Ok(name) = clean_name(&folder.name) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let kept = if meron_folder_exists(conn, id)? {
            id.to_string()
        } else if let Some(existing) = list_meron_folders(conn)?
            .into_iter()
            .find(|known| known.name.to_lowercase() == name.to_lowercase())
        {
            existing.id
        } else {
            // A folder already here keeps its own notification choice.
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO meron_folders(id, name, sort_order, created_at, notify)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    name,
                    folder.sort_order,
                    if folder.created_at > 0 { folder.created_at } else { super::now_unix() },
                    folder.notify as i64
                ],
            )?;
            if inserted == 0 {
                continue;
            }
            added += 1;
            id.to_string()
        };
        ids.insert(id.to_string(), kept.clone());
        for entry in &folder.entries {
            let known: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
                params![entry.account],
                |row| row.get(0),
            )?;
            if !known || entry.id.trim().is_empty() {
                continue;
            }
            let fresh = conn.execute(
                "INSERT OR IGNORE INTO meron_folder_entries(id, account, folder_id, added_at)
                 VALUES(?1, ?2, ?3, ?4)",
                params![entry.id, entry.account, kept, entry.added_at],
            )? > 0;
            if !fresh {
                continue;
            }
            let mut any = false;
            for alias in &entry.aliases {
                if is_alias_key(alias) {
                    any |= conn.execute(
                        "INSERT OR IGNORE INTO meron_folder_aliases(account, key, entry_id)
                         VALUES(?1, ?2, ?3)",
                        params![entry.account, alias, entry.id],
                    )? > 0;
                }
            }
            if !any {
                conn.execute(
                    "DELETE FROM meron_folder_entries WHERE id = ?1",
                    params![entry.id],
                )?;
            }
        }
    }
    Ok((added, ids))
}
