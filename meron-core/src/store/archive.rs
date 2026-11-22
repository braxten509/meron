//! Local folders: mail kept on this computer instead of on a server.
//!
//! There is one local account, [`LOCAL_ACCOUNT_ID`]. Its row lives in the store like any account's,
//! so it lists, orders and themes like one, but its folders and mail live in a separate database,
//! `archive.db` (see `db::open_archive_at`). The store is a cache that sync may clear and refill;
//! the archive holds the only copy of what it keeps, so no cache-management path can reach it.
//!
//! The archive shares the store's schema, so the message-list, search and thread code reads it
//! unchanged. `archive_raw` is the source of truth: the original RFC822 message, zstd-compressed,
//! with the few fields it does not carry itself (flags, thread key, a date fallback).
//!
//! The `messages` row of an archived message is derived from it by [`index_raw`] and can be rebuilt
//! at any time, which is how a move between local folders re-keys a message.

mod folders;
pub use folders::{
    canon_local_folder, create_local_folder, delete_local_folder, local_folder_subtree,
    normalize_local_folder_name, rename_local_folder,
};

mod restore;
use restore::{allocate_uids, reclaim_space, sha256_hex};
pub use restore::{local_storage_stats, repair_local_index, restore_local_backup};

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::imap::Folder;

/// The local account's id, in the store and in thread ids.
pub const LOCAL_ACCOUNT_ID: &str = "local";

/// Local folders nest with `/`, whatever the source servers used.
pub const LOCAL_DELIMITER: &str = "/";

/// The `special_use` every local folder carries, so a folder the user names
/// "Sent" or "Trash" is still an ordinary folder (see `classify_folder_role`).
pub const LOCAL_SPECIAL_USE: &str = "local";

const ZSTD_LEVEL: i32 = 9;

pub(super) const ARCHIVE_DDL: &str = "
CREATE TABLE IF NOT EXISTS archive_raw (
  id            INTEGER PRIMARY KEY,
  folder        TEXT NOT NULL,
  uid           INTEGER NOT NULL,
  sha256        TEXT NOT NULL,              -- of the uncompressed message
  size          INTEGER NOT NULL,           -- uncompressed bytes
  seen          INTEGER NOT NULL DEFAULT 0,
  starred       INTEGER NOT NULL DEFAULT 0,
  thread_key    TEXT NOT NULL DEFAULT '',   -- the source's key; '' = derive from headers
  date_fallback INTEGER NOT NULL DEFAULT 0, -- used when the Date header is unusable
  source        TEXT NOT NULL DEFAULT '',   -- where it came from, for the user
  archived_at   INTEGER NOT NULL DEFAULT 0,
  data          BLOB NOT NULL,              -- zstd
  UNIQUE (folder, uid)
);
CREATE INDEX IF NOT EXISTS archive_raw_sha_idx ON archive_raw(folder, sha256);
";

pub fn is_local_account(id: &str) -> bool {
    id == LOCAL_ACCOUNT_ID
}

/// Create the local account's row in the store. Returns false when it exists.
pub fn add_local_account(conn: &Connection, display_name: &str) -> Result<bool> {
    let now = super::now_unix();
    // Local folders have no inbox to merge, and nothing to send from.
    let prefs = json!({ "included_in_unified": false }).to_string();
    let inserted = conn.execute(
        "INSERT OR IGNORE INTO accounts(id, engine, provider, display_name, config, prefs, created_at, updated_at, sort_order)
         VALUES(?1, 'local', 'local', ?2, '{}', ?3, ?4, ?4, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts), 0))",
        params![LOCAL_ACCOUNT_ID, display_name, prefs, now],
    )?;
    Ok(inserted > 0)
}

pub fn has_local_account(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
        params![LOCAL_ACCOUNT_ID],
        |row| row.get(0),
    )?)
}

/// Copy what thread reads need from the store into the archive's own account
/// row: the local account's preferences (remote images, HTML mode) and every
/// address the user owns, as aliases, so mail the user sent before archiving
/// still reads as outgoing.
pub fn sync_local_identity(store: &Connection, archive: &Connection) -> Result<()> {
    let prefs: Option<String> = store
        .query_row(
            "SELECT prefs FROM accounts WHERE id = ?1",
            params![LOCAL_ACCOUNT_ID],
            |row| row.get(0),
        )
        .optional()?;
    let mut prefs = prefs
        .and_then(|prefs| serde_json::from_str::<Value>(&prefs).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let mut addrs = super::all_self_addrs(store).into_iter().collect::<Vec<_>>();
    addrs.sort();
    prefs["aliases"] = Value::Array(
        addrs
            .into_iter()
            .map(|email| json!({ "email": email, "name": "" }))
            .collect(),
    );
    let prefs = prefs.to_string();
    // Reads call this; the archive syncs every commit to disk, so only write a change.
    let current: Option<String> = archive
        .query_row(
            "SELECT prefs FROM accounts WHERE id = ?1",
            params![LOCAL_ACCOUNT_ID],
            |row| row.get(0),
        )
        .optional()?;
    if current.as_deref() == Some(prefs.as_str()) {
        return Ok(());
    }
    archive.execute(
        "INSERT INTO accounts(id, engine, provider, prefs) VALUES(?1, 'local', 'local', ?2)
         ON CONFLICT(id) DO UPDATE SET prefs = excluded.prefs",
        params![LOCAL_ACCOUNT_ID, prefs],
    )?;
    Ok(())
}

/// One message to keep in a local folder.
pub struct ArchiveInput<'a> {
    pub raw: &'a [u8],
    pub seen: bool,
    pub starred: bool,
    /// The thread key the message had where it came from, so a conversation
    /// keeps grouping the way it did. Empty derives one from its headers.
    pub thread_key: String,
    /// The source's date, for a message whose Date header does not parse.
    pub date_fallback: i64,
    /// A short description of the origin ("user@example.com/INBOX").
    pub source: String,
}

/// Store messages in a local folder. Each one is written, read back and
/// compared before the next, so a returned UID is a message that is safely on
/// disk; the caller may then remove the original. A message already in the
/// folder (same bytes) is not stored twice: its existing UID is returned.
pub fn archive_messages(
    conn: &Connection,
    folder: &str,
    messages: &[ArchiveInput<'_>],
    media_root: &Path,
) -> Result<Vec<u32>> {
    if !super::folder_exists(conn, LOCAL_ACCOUNT_ID, folder)? {
        bail!("Folder not found: {folder}");
    }
    let mut uids = Vec::with_capacity(messages.len());
    for message in messages {
        if message.raw.is_empty() {
            bail!("The server returned an empty message");
        }
        let sha256 = sha256_hex(message.raw);
        let existing: Option<u32> = conn
            .query_row(
                "SELECT uid FROM archive_raw WHERE folder = ?1 AND sha256 = ?2",
                params![folder, sha256],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(uid) = existing {
            uids.push(uid);
            continue;
        }
        let data = zstd::bulk::compress(message.raw, ZSTD_LEVEL).context("compress message")?;
        let uid = allocate_uids(conn, folder, 1)?;
        conn.execute(
            "INSERT INTO archive_raw(folder, uid, sha256, size, seen, starred, thread_key, date_fallback, source, archived_at, data)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                folder,
                uid,
                sha256,
                message.raw.len() as i64,
                message.seen as i64,
                message.starred as i64,
                message.thread_key,
                message.date_fallback,
                message.source,
                super::now_unix(),
                data,
            ],
        )?;
        // Read the stored copy back rather than trusting the write.
        let stored = raw_message(conn, folder, uid)?
            .ok_or_else(|| anyhow!("archived message {uid} could not be read back"))?;
        if stored.len() != message.raw.len() || sha256_hex(&stored) != sha256 {
            conn.execute(
                "DELETE FROM archive_raw WHERE folder = ?1 AND uid = ?2",
                params![folder, uid],
            )?;
            bail!("archived message {uid} did not read back intact");
        }
        index_raw(conn, folder, uid, media_root)?;
        uids.push(uid);
    }
    Ok(uids)
}

/// The original RFC822 bytes of an archived message.
pub fn raw_message(conn: &Connection, folder: &str, uid: u32) -> Result<Option<Vec<u8>>> {
    let data: Option<Vec<u8>> = conn
        .query_row(
            "SELECT data FROM archive_raw WHERE folder = ?1 AND uid = ?2",
            params![folder, uid],
            |row| row.get(0),
        )
        .optional()?;
    data.map(|data| zstd::stream::decode_all(data.as_slice()).context("decompress message"))
        .transpose()
}

/// Every UID kept in a local folder.
pub fn local_folder_uids(conn: &Connection, folder: &str) -> Result<Vec<u32>> {
    let mut stmt = conn.prepare("SELECT uid FROM archive_raw WHERE folder = ?1 ORDER BY uid")?;
    let rows = stmt.query_map(params![folder], |row| row.get(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Archived messages as they are kept: raw bytes plus flags, in `uids` order.
/// UIDs that are not in the folder are left out.
pub fn raw_messages_for_copy(
    conn: &Connection,
    folder: &str,
    uids: &[u32],
) -> Result<Vec<crate::imap::RawMessageCopy>> {
    let mut out = Vec::with_capacity(uids.len());
    for &uid in uids {
        let flags: Option<(bool, bool)> = conn
            .query_row(
                "SELECT seen, starred FROM archive_raw WHERE folder = ?1 AND uid = ?2",
                params![folder, uid],
                |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)? != 0)),
            )
            .optional()?;
        let (Some((seen, starred)), Some(raw)) = (flags, raw_message(conn, folder, uid)?) else {
            continue;
        };
        out.push(crate::imap::RawMessageCopy {
            uid,
            raw,
            seen,
            starred,
        });
    }
    Ok(out)
}

/// (Re)build the `messages` row of one archived message from `archive_raw`,
/// writing its attachments into the media cache under keys of its current
/// folder and UID.
pub fn index_raw(conn: &Connection, folder: &str, uid: u32, media_root: &Path) -> Result<()> {
    let row: Option<(bool, bool, String, i64)> = conn
        .query_row(
            "SELECT seen, starred, thread_key, date_fallback FROM archive_raw WHERE folder = ?1 AND uid = ?2",
            params![folder, uid],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? != 0,
                    row.get::<_, i64>(1)? != 0,
                    row.get(2)?,
                    row.get(3)?,
                ))
            },
        )
        .optional()?;
    let Some((seen, starred, thread_key, date_fallback)) = row else {
        bail!("archived message {uid} not found in {folder}");
    };
    let raw = raw_message(conn, folder, uid)?.unwrap_or_default();
    let mut header = crate::imap::header_from_raw(&raw, uid, seen, starred);
    let context = crate::parse::MediaCtx {
        root: media_root.to_path_buf(),
        account: LOCAL_ACCOUNT_ID.to_string(),
        folder: folder.to_string(),
        uid,
    };
    let mut message = crate::parse::parse_message(&raw, Some(&context));
    // Reads treat a row without a Message-ID as one cached before ids were
    // stored and go to the server for it, which a local folder has not got.
    if header.message_id.is_empty() {
        header.message_id = format!("{}@local.meron", &sha256_hex(&raw)[..32]);
    }
    message.message_id = header.message_id.clone();
    // A `uid:` key names the message's UID where it came from; here it has
    // another, so the key derived for this UID stands instead.
    if !thread_key.is_empty() && !thread_key.starts_with("uid:") {
        header.thread_key = thread_key;
    }
    if header.date == 0 {
        header.date = date_fallback;
    }
    if message.date == 0 {
        message.date = header.date;
    }
    header.files = Some(
        message
            .file_attachments()
            .map(|attachment| super::CardFile {
                filename: attachment.filename.clone(),
                mime: attachment.mime.clone(),
            })
            .collect(),
    );
    super::upsert_messages(
        conn,
        LOCAL_ACCOUNT_ID,
        folder,
        std::slice::from_ref(&header),
    )?;
    super::save_cached_message(conn, LOCAL_ACCOUNT_ID, folder, uid, &message)?;
    Ok(())
}

/// Put back any attachment files of these archived messages that the media
/// cache has evicted. Rewrites the same keys, so stored HTML keeps pointing
/// at them.
pub fn restore_local_media(
    conn: &Connection,
    messages: &[(String, u32)],
    media_root: &Path,
) -> Result<()> {
    for (folder, uid) in messages {
        let Some(cached) = super::get_cached_message(conn, LOCAL_ACCOUNT_ID, folder, *uid)? else {
            continue;
        };
        if crate::parse::missing_media_count(media_root, &cached) == 0 {
            continue;
        }
        // Reindexing rewrites the files and the attachment list that names
        // them, which a write that failed the first time left without keys.
        index_raw(conn, folder, *uid, media_root)?;
    }
    Ok(())
}

/// Move archived messages to another local folder. They get new UIDs there,
/// returned in `uids` order; UIDs not found are skipped.
pub fn move_local_messages(
    conn: &Connection,
    folder: &str,
    target: &str,
    uids: &[u32],
    media_root: &Path,
) -> Result<Vec<u32>> {
    transfer_local_messages(conn, folder, target, uids, media_root, true)
}

/// Copy archived messages into another local folder.
pub fn copy_local_messages(
    conn: &Connection,
    folder: &str,
    target: &str,
    uids: &[u32],
    media_root: &Path,
) -> Result<Vec<u32>> {
    transfer_local_messages(conn, folder, target, uids, media_root, false)
}

fn transfer_local_messages(
    conn: &Connection,
    folder: &str,
    target: &str,
    uids: &[u32],
    media_root: &Path,
    remove_source: bool,
) -> Result<Vec<u32>> {
    if !super::folder_exists(conn, LOCAL_ACCOUNT_ID, target)? {
        bail!("Folder not found: {target}");
    }
    let mut out = Vec::with_capacity(uids.len());
    for &uid in uids {
        // Keep the flags the message has now, which the index row holds.
        let flags: Option<(i64, i64)> = conn
            .query_row(
                "SELECT seen, starred FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                params![LOCAL_ACCOUNT_ID, folder, uid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let new_uid = allocate_uids(conn, target, 1)?;
        let tx = conn.unchecked_transaction()?;
        let changed = if remove_source {
            tx.execute(
                "UPDATE archive_raw SET folder = ?3, uid = ?4 WHERE folder = ?1 AND uid = ?2",
                params![folder, uid, target, new_uid],
            )?
        } else {
            tx.execute(
                "INSERT INTO archive_raw(folder, uid, sha256, size, seen, starred, thread_key, date_fallback, source, archived_at, data)
                 SELECT ?3, ?4, sha256, size, seen, starred, thread_key, date_fallback, source, archived_at, data
                   FROM archive_raw WHERE folder = ?1 AND uid = ?2",
                params![folder, uid, target, new_uid],
            )?
        };
        if changed == 0 {
            continue;
        }
        if let Some((seen, starred)) = flags {
            tx.execute(
                "UPDATE archive_raw SET seen = ?3, starred = ?4 WHERE folder = ?1 AND uid = ?2",
                params![target, new_uid, seen, starred],
            )?;
        }
        if remove_source {
            tx.execute(
                "DELETE FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                params![LOCAL_ACCOUNT_ID, folder, uid],
            )?;
        }
        tx.commit()?;
        index_raw(conn, target, new_uid, media_root)?;
        out.push(new_uid);
    }
    Ok(out)
}

/// Permanently delete archived messages. Returns how many were removed.
pub fn delete_local_messages(conn: &Connection, folder: &str, uids: &[u32]) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut deleted = 0;
    for &uid in uids {
        deleted += tx.execute(
            "DELETE FROM archive_raw WHERE folder = ?1 AND uid = ?2",
            params![folder, uid],
        )?;
        tx.execute(
            "DELETE FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![LOCAL_ACCOUNT_ID, folder, uid],
        )?;
    }
    tx.commit()?;
    reclaim_space(conn);
    Ok(deleted)
}

/// Set the read and/or starred flag on archived messages, in both the index
/// row and the kept original.
pub fn set_local_flags(
    conn: &Connection,
    folder: &str,
    uids: &[u32],
    seen: Option<bool>,
    starred: Option<bool>,
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for &uid in uids {
        if let Some(seen) = seen {
            for table in ["archive_raw", "messages"] {
                tx.execute(
                    &format!(
                        "UPDATE {table} SET seen = ?3 WHERE folder = ?1 AND uid = ?2{}",
                        if table == "messages" {
                            " AND account = ?4"
                        } else {
                            ""
                        }
                    ),
                    rusqlite::params_from_iter(local_flag_params(folder, uid, seen, table)),
                )?;
            }
        }
        if let Some(starred) = starred {
            for table in ["archive_raw", "messages"] {
                tx.execute(
                    &format!(
                        "UPDATE {table} SET starred = ?3 WHERE folder = ?1 AND uid = ?2{}",
                        if table == "messages" {
                            " AND account = ?4"
                        } else {
                            ""
                        }
                    ),
                    rusqlite::params_from_iter(local_flag_params(folder, uid, starred, table)),
                )?;
            }
        }
    }
    tx.commit()?;
    Ok(())
}

fn local_flag_params(
    folder: &str,
    uid: u32,
    value: bool,
    table: &str,
) -> Vec<rusqlite::types::Value> {
    use rusqlite::types::Value as Sql;
    let mut values = vec![
        Sql::Text(folder.to_string()),
        Sql::Integer(uid as i64),
        Sql::Integer(value as i64),
    ];
    if table == "messages" {
        values.push(Sql::Text(LOCAL_ACCOUNT_ID.to_string()));
    }
    values
}

/// Mark every message in a local folder read or unread.
pub fn set_local_folder_seen(conn: &Connection, folder: &str, seen: bool) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let updated = tx.execute(
        "UPDATE archive_raw SET seen = ?2 WHERE folder = ?1 AND seen <> ?2",
        params![folder, seen as i64],
    )?;
    super::mark_folder_seen(&tx, LOCAL_ACCOUNT_ID, folder, seen)?;
    tx.commit()?;
    Ok(updated)
}

/// What a store row knows about a message that its raw bytes may not: the
/// thread key it grouped under (Gmail's thread id, an inherited root) and its
/// date with the INTERNALDATE fallback applied.
pub fn cached_thread_key_and_date(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
) -> Result<(String, i64)> {
    Ok(conn
        .query_row(
            "SELECT COALESCE(thread_key, ''), date FROM messages
              WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .unwrap_or_default())
}

/// "user@example.com/INBOX": where an archived message came from.
pub fn archive_source_label(conn: &Connection, account: &str, folder: &str) -> String {
    let email: Option<String> = conn
        .query_row(
            "SELECT COALESCE(NULLIF(email, ''), display_name) FROM accounts WHERE id = ?1",
            params![account],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    format!(
        "{}/{}",
        email.unwrap_or_else(|| account.to_string()),
        crate::utf7::decode(folder)
    )
}

/// Marks an error as a wrong passphrase (or a file that is no local folders
/// backup), so the UI can ask again instead of reporting a failure.
pub const WRONG_BACKUP_PASSPHRASE: &str = "wrong passphrase for this local folders backup";

#[cfg(test)]
mod tests;
