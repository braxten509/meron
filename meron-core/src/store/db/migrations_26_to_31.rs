//! Migrations 26 to 31 for the local mail database.
//!
//! The database opener runs migrations in schema-version order.
//! Existing profiles and fresh installations share the same upgrade path.

use super::*;

/// Meron folders: folders that exist only on this device. A conversation in one is an entry; its
/// aliases are the keys that find its messages (thread root keys and per-message tag keys, see
/// `store::meron_folders`); its UIDs are remembered so unread counts can leave out server-unseen
/// mail the cache no longer holds. The AI log learns which Meron folder it filed mail into.
pub(super) fn migrate_v26(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meron_folders (
           id         TEXT PRIMARY KEY,
           name       TEXT NOT NULL,
           sort_order INTEGER NOT NULL DEFAULT 0,
           created_at INTEGER NOT NULL
         );
         CREATE UNIQUE INDEX IF NOT EXISTS meron_folders_name_idx ON meron_folders(lower(name));
         CREATE TABLE IF NOT EXISTS meron_folder_entries (
           id        TEXT PRIMARY KEY,
           account   TEXT NOT NULL,
           folder_id TEXT NOT NULL,
           added_at  INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS meron_folder_entries_folder_idx
           ON meron_folder_entries(folder_id);
         CREATE TABLE IF NOT EXISTS meron_folder_aliases (
           account  TEXT NOT NULL,
           key      TEXT NOT NULL,
           entry_id TEXT NOT NULL,
           PRIMARY KEY (account, key)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS meron_folder_aliases_entry_idx
           ON meron_folder_aliases(entry_id);
         CREATE TABLE IF NOT EXISTS meron_folder_uids (
           account     TEXT NOT NULL,
           folder      TEXT NOT NULL,
           uidvalidity INTEGER NOT NULL DEFAULT 0,
           uid         INTEGER NOT NULL,
           entry_id    TEXT NOT NULL,
           tag_key     TEXT NOT NULL DEFAULT '',
           PRIMARY KEY (account, folder, uid)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS meron_folder_uids_entry_idx
           ON meron_folder_uids(entry_id);
         ALTER TABLE ai_sort_log ADD COLUMN meron_folder_id TEXT NOT NULL DEFAULT '';
         PRAGMA user_version = 26;",
    )?;
    Ok(())
}

/// AI sorting of mail server filters deliver straight into other folders: where each folder's
/// sorting starts (`ai_sort_watch`, so only mail arriving after the first look is sorted), and the
/// mail it left, shown in the Inbox on this device only (`ai_sort_inbox`, by location; the identity
/// lets a row follow its message across a UIDVALIDITY reset). Nothing here reaches the server.
pub(super) fn migrate_v27(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ai_sort_watch (
           account     TEXT NOT NULL,
           folder      TEXT NOT NULL,
           uidvalidity INTEGER NOT NULL,
           from_uid    INTEGER NOT NULL,
           PRIMARY KEY (account, folder)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS ai_sort_inbox (
           account     TEXT NOT NULL,
           folder      TEXT NOT NULL,
           uid         INTEGER NOT NULL,
           uidvalidity INTEGER NOT NULL DEFAULT 0,
           identity    TEXT NOT NULL DEFAULT '',
           at          INTEGER NOT NULL,
           PRIMARY KEY (account, folder, uid)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS ai_sort_inbox_identity_idx
           ON ai_sort_inbox(account, identity);
         ALTER TABLE ai_sort_log ADD COLUMN from_folder TEXT NOT NULL DEFAULT '';
         ALTER TABLE ai_sort_log ADD COLUMN server_target TEXT NOT NULL DEFAULT '';
         PRAGMA user_version = 27;",
    )?;
    Ok(())
}

/// New-mail notifications for Meron folders: off unless turned on, kept on
/// this device only.
pub(super) fn migrate_v28(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE meron_folders ADD COLUMN notify INTEGER NOT NULL DEFAULT 0;
         PRAGMA user_version = 28;",
    )?;
    Ok(())
}

/// Who put each conversation into its Meron folder: `'ai'` (AI sorting) or
/// `'user'` (by hand). Existing entries start as `'user'`; the one-time pass
/// `store::meron_release_old_ai_entries` marks those the AI filed.
pub(super) fn migrate_v29(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE meron_folder_entries ADD COLUMN source TEXT NOT NULL DEFAULT 'user';
         PRAGMA user_version = 29;",
    )?;
    Ok(())
}

/// The Meron-folder marks (`$MeronF_<id>` keywords) each Inbox copy carries on the server, as this
/// device last read or wrote them, so a mark another device added or took away is told apart from
/// one that was always there. `written_at` is when this device wrote it (0 when read), for the
/// grace period that ignores a read taken just before its own write.
pub(super) fn migrate_v30(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meron_marks (
           account    TEXT NOT NULL,
           folder     TEXT NOT NULL,
           uid        INTEGER NOT NULL,
           folder_id  TEXT NOT NULL,
           written_at INTEGER NOT NULL DEFAULT 0,
           PRIMARY KEY (account, folder, uid)
         ) WITHOUT ROWID;
         PRAGMA user_version = 30;",
    )?;
    Ok(())
}

/// What the reader taught AI sorting by moving mail by hand (see
/// `store::ai_lessons`): one row per email, keyed like its tags.
pub(super) fn migrate_v31(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ai_sort_lessons (
           id        INTEGER PRIMARY KEY AUTOINCREMENT,
           account   TEXT NOT NULL,
           tag_key   TEXT NOT NULL,
           from_name TEXT NOT NULL DEFAULT '',
           from_addr TEXT NOT NULL DEFAULT '',
           subject   TEXT NOT NULL DEFAULT '',
           place     TEXT NOT NULL,
           at        INTEGER NOT NULL,
           UNIQUE (account, tag_key)
         );
         PRAGMA user_version = 31;",
    )?;
    Ok(())
}

/// v26 gained `meron_folder_uids.tag_key` before it shipped; a development
/// store an earlier v26 build migrated lacks it. Adds it once; a no-op on
/// every other store.
pub(super) fn repair_v26_meron_uids(conn: &Connection) -> Result<()> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meron_folder_uids')",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(());
    }
    let has_column: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('meron_folder_uids') WHERE name = 'tag_key')",
        [],
        |row| row.get(0),
    )?;
    if !has_column {
        conn.execute_batch(
            "ALTER TABLE meron_folder_uids ADD COLUMN tag_key TEXT NOT NULL DEFAULT ''",
        )?;
    }
    Ok(())
}
