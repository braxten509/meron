//! Restores a local mail archive and reports its storage use.
//! The restore path repairs the index and tracks UID allocation in SQLite.
//! Statistics and reclamation helpers describe the resulting local account.

use super::*;

/// Add the mail of a passphrase-protected local folders backup (see
/// `store::backup_archive_to`) to the archive: its folders, and every message
/// not already in the same folder. Nothing here is overwritten or removed.
/// Returns how many messages were added.
pub fn restore_local_backup(
    conn: &Connection,
    source: &Path,
    passphrase: &str,
    media_root: &Path,
) -> Result<usize> {
    conn.execute(
        "ATTACH DATABASE ?1 AS restore KEY ?2",
        params![source.to_string_lossy(), passphrase],
    )
    .map_err(|error| match error.sqlite_error_code() {
        // A wrong key reads as a file that is not a database.
        Some(rusqlite::ErrorCode::NotADatabase) => anyhow!(WRONG_BACKUP_PASSPHRASE),
        _ => anyhow::Error::from(error),
    })?;
    let restored = restore_attached(conn, media_root);
    let _ = conn.execute_batch("DETACH DATABASE restore;");
    restored
}

fn restore_attached(conn: &Connection, media_root: &Path) -> Result<usize> {
    let readable = conn
        .query_row(
            "SELECT count(*) FROM restore.sqlite_master WHERE name = 'archive_raw'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0);
    if readable == 0 {
        bail!(WRONG_BACKUP_PASSPHRASE);
    }
    let folders = {
        let mut stmt = conn.prepare(
            "SELECT name FROM restore.folders WHERE account = ?1
             UNION SELECT DISTINCT folder FROM restore.archive_raw",
        )?;
        stmt.query_map(params![LOCAL_ACCOUNT_ID], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for folder in &folders {
        conn.execute(
            "INSERT OR IGNORE INTO main.folders(account, name, delimiter, special_use) VALUES(?1, ?2, ?3, ?4)",
            params![LOCAL_ACCOUNT_ID, folder, LOCAL_DELIMITER, LOCAL_SPECIAL_USE],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO main.folder_state(account, folder, uidvalidity, uid_next) VALUES(?1, ?2, 1, 1)",
            params![LOCAL_ACCOUNT_ID, folder],
        )?;
    }
    let rows = {
        let mut stmt = conn.prepare(
            "SELECT r.id, r.folder FROM restore.archive_raw r
              WHERE NOT EXISTS (
                SELECT 1 FROM main.archive_raw m WHERE m.folder = r.folder AND m.sha256 = r.sha256
              )
              ORDER BY r.id",
        )?;
        stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    // A backup from before tags has no such table. Rows keyed by location stay
    // behind: the restore hands out new UIDs.
    let has_tags: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM restore.sqlite_master WHERE name = 'message_tags')",
        [],
        |row| row.get(0),
    )?;
    if has_tags {
        conn.execute(
            &format!(
                "INSERT OR IGNORE INTO main.message_tags(account, tag_key, tag, state, updated_at)
                 SELECT account, tag_key, tag, 0, 0 FROM restore.message_tags
                  WHERE state IN {} AND tag_key NOT LIKE 'uid:%'",
                crate::store::tags::LIVE_STATES,
            ),
            [],
        )?;
    }
    for (id, folder) in &rows {
        let uid = allocate_uids(conn, folder, 1)?;
        conn.execute(
            "INSERT INTO main.archive_raw(folder, uid, sha256, size, seen, starred, thread_key, date_fallback, source, archived_at, data)
             SELECT folder, ?2, sha256, size, seen, starred, thread_key, date_fallback, source, archived_at, data
               FROM restore.archive_raw WHERE id = ?1",
            params![id, uid],
        )?;
        index_raw(conn, folder, uid, media_root)?;
    }
    Ok(rows.len())
}

/// Rebuild index rows for archived messages that have none: a crash between
/// storing a message and indexing it, or a migration that rebuilt `messages`.
/// Returns how many were indexed.
pub fn repair_local_index(conn: &Connection, media_root: &Path) -> Result<usize> {
    let missing = {
        let mut stmt = conn.prepare(
            "SELECT r.folder, r.uid FROM archive_raw r
              WHERE NOT EXISTS (
                SELECT 1 FROM messages m
                 WHERE m.account = ?1 AND m.folder = r.folder AND m.uid = r.uid
                   AND m.body IS NOT NULL
              )",
        )?;
        stmt.query_map(params![LOCAL_ACCOUNT_ID], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (folder, uid) in &missing {
        crate::store::ensure_folder(conn, LOCAL_ACCOUNT_ID, folder)?;
        index_raw(conn, folder, *uid, media_root)?;
    }
    Ok(missing.len())
}

/// How much mail the archive keeps: messages, their original size, and the
/// space the database file takes on disk.
pub fn local_storage_stats(conn: &Connection) -> Result<Value> {
    let (messages, size): (i64, i64) = conn.query_row(
        "SELECT count(*), COALESCE(sum(size), 0) FROM archive_raw",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let page_count = pragma_int(conn, "page_count")?;
    let page_size = pragma_int(conn, "page_size")?;
    let free_pages = pragma_int(conn, "freelist_count")?;
    Ok(json!({
        "messages": messages,
        "message_bytes": size,
        "file_bytes": (page_count - free_pages).max(0) * page_size,
    }))
}

/// An integer pragma. SQLCipher answers `page_size` on a keyed database as
/// its own `cipher_page_size`, which is text.
fn pragma_int(conn: &Connection, name: &str) -> Result<i64> {
    let value: rusqlite::types::Value =
        conn.query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))?;
    match value {
        rusqlite::types::Value::Integer(n) => Ok(n),
        rusqlite::types::Value::Text(text) => text
            .trim()
            .parse()
            .with_context(|| format!("PRAGMA {name} returned {text:?}")),
        other => bail!("PRAGMA {name} returned {other:?}"),
    }
}

/// Allocate `count` consecutive UIDs in a local folder from its `uid_next`.
pub(super) fn allocate_uids(conn: &Connection, folder: &str, count: u32) -> Result<u32> {
    let next: u32 = conn
        .query_row(
            "SELECT uid_next FROM folder_state WHERE account = ?1 AND folder = ?2",
            params![LOCAL_ACCOUNT_ID, folder],
            |row| row.get::<_, Option<u32>>(0),
        )
        .optional()?
        .flatten()
        .unwrap_or(1)
        .max(1);
    // Never reuse a UID still held by a row, whatever uid_next says.
    let held: u32 = conn.query_row(
        "SELECT COALESCE(MAX(uid), 0) + 1 FROM archive_raw WHERE folder = ?1",
        params![folder],
        |row| row.get(0),
    )?;
    let first = next.max(held);
    crate::store::set_folder_state(conn, LOCAL_ACCOUNT_ID, folder, 1, first + count)?;
    Ok(first)
}

/// Give freed pages back to the file system after a delete.
pub(super) fn reclaim_space(conn: &Connection) {
    let _ = conn.execute_batch("PRAGMA incremental_vacuum;");
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
