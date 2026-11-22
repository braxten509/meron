//! Open, key and upgrade the local mail database.
//!
//! Store operations share the same connection setup and transaction handling.
//! Schema definitions and ordered migration steps live in the adjacent modules.

use anyhow::{Context, Result};
use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior, params,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod schema;
pub(super) use schema::{ACCOUNTS_DDL, MESSAGES_DDL, TASKS_DDL};
use schema::{
    MAIL_SEARCH_HITS_DDL, MESSAGES_RECIPIENTS_FTS_DDL, MESSAGES_THREAD_KEY_INDEXES_DDL,
    OBSERVED_MAIL_IDENTITIES_DDL, SCHEMA,
};
mod migrations_01_to_16;
use migrations_01_to_16::{
    migrate_v1, migrate_v2, migrate_v3, migrate_v4, migrate_v5, migrate_v6, migrate_v7, migrate_v8,
    migrate_v9, migrate_v10, migrate_v11, migrate_v12, migrate_v13, migrate_v14, migrate_v15,
    migrate_v16,
};
mod contact_migrations;
use contact_migrations::{
    contact_occurrences, contacts_triggers, migrate_v17, migrate_v18, migrate_v19,
};
mod migrations_20_to_25;
#[cfg(test)]
mod tests;
use migrations_20_to_25::{
    migrate_v20, migrate_v21, migrate_v22, migrate_v23, migrate_v24, migrate_v25,
};
mod migrations_26_to_31;
use migrations_26_to_31::{
    migrate_v26, migrate_v27, migrate_v28, migrate_v29, migrate_v30, migrate_v31,
    repair_v26_meron_uids,
};

const BODY_CACHE_VERSION: &str = "1";

pub fn open() -> Result<Connection> {
    open_notifying(|| {})
}

/// [`open`], calling `on_upgrade` first if this open has to rewrite an existing
/// store, which can take a while on a large cache. A brand-new store and an
/// up-to-date one stay silent.
pub fn open_notifying(on_upgrade: impl FnOnce()) -> Result<Connection> {
    let path = db_path();
    // The key comes from the OS keychain, historically the slowest and most
    // failure-prone step of startup (a Flatpak Secret portal with no backend
    // used to hang here forever). Bracket it in the log so a stall is visible.
    crate::mlog!(crate::log::Level::Warn, "store", "resolving store key");
    let started = std::time::Instant::now();
    let key = crate::secrets::db_key();
    crate::mlog!(
        crate::log::Level::Warn,
        "store",
        "store key resolved in {:?}: {}",
        started.elapsed(),
        match &key {
            Ok(Some(_)) => "encrypted".to_string(),
            Ok(None) => "plaintext (keychain disabled)".to_string(),
            Err(error) => format!("failed: {error:#}"),
        }
    );
    let key = key?;
    let _ = STORE_KEY.set(key.clone());
    match key {
        Some(key) => open_inner(path.as_ref(), Some(&key), on_upgrade),
        // Keyring disabled (tests/headless): keep the store plaintext as before.
        None => open_inner(path.as_ref(), None, on_upgrade),
    }
}

/// [`open_notifying`] for a store at a host-chosen path and key.
pub fn open_at_notifying(
    path: impl AsRef<Path>,
    key: Option<&str>,
    on_upgrade: impl FnOnce(),
) -> Result<Connection> {
    open_inner(path.as_ref(), key, on_upgrade)
}

/// The key [`open`] resolved, so the local-folder archive opens with the same
/// one without asking the keychain a second time.
static STORE_KEY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// Open the local-folder archive (`archive.db`, beside the store), encrypted
/// with the store's key. See `store::archive`.
pub fn open_archive() -> Result<Connection> {
    open_archive_at(archive_path(), archive_key()?.as_deref())
}

/// The key `archive.db` is encrypted with: the store's.
fn archive_key() -> Result<Option<String>> {
    match STORE_KEY.get() {
        Some(key) => Ok(key.clone()),
        None => crate::secrets::db_key(),
    }
}

/// Open an archive at `path`, keyed when `key` is given. The archive shares
/// the store's schema, so every message-list and thread read works on it
/// unchanged, plus the `archive_raw` table holding the original messages.
pub fn open_archive_at(path: impl AsRef<Path>, key: Option<&str>) -> Result<Connection> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let fresh = !path.exists();
    let mut conn = open_keyed_connection(path, key, || {})?;
    // Deleting archived mail should give the disk space back. auto_vacuum only
    // takes effect when set before the first table exists.
    if fresh {
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")
            .context("set archive auto_vacuum")?;
    }
    configure_connection(&mut conn)?;
    // The archive holds the only copy of its mail: a commit must reach the
    // disk before the server copy is removed.
    conn.execute_batch("PRAGMA synchronous = FULL;")
        .context("set archive synchronous")?;
    with_anyhow_busy_retry(|| run_migrations(&conn)).context("run migrations")?;
    conn.execute_batch(super::archive::ARCHIVE_DDL)
        .context("create archive tables")?;
    Ok(conn)
}

/// Write a consistent copy of the open archive to `dest`, encrypted with
/// `passphrase` rather than the keychain's key, so it can be restored on a
/// computer that never had that key (see `store::restore_local_backup`).
/// Copying `archive.db` itself while Meron runs is not a backup: recent
/// commits live in its WAL.
pub fn backup_archive_to(conn: &Connection, dest: &Path, passphrase: &str) -> Result<()> {
    if passphrase.is_empty() {
        anyhow::bail!("A passphrase is required");
    }
    // Renaming the snapshot over a database in use (or its WAL) would corrupt it.
    let live = [
        conn.path().map(PathBuf::from),
        Some(archive_path()),
        Some(db_path()),
    ];
    for database in live.into_iter().flatten() {
        for suffix in ["", "-wal", "-shm"] {
            if same_file_path(dest, &sibling_with_suffix(&database, suffix)) {
                anyhow::bail!("Choose another file: {} is in use by Meron", dest.display());
            }
        }
    }
    let file_name = dest
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive.db");
    let partial = dest.with_file_name(format!("{file_name}.partial"));
    let _ = std::fs::remove_file(&partial);
    let user_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let exported = conn
        .execute(
            "ATTACH DATABASE ?1 AS snapshot KEY ?2",
            params![partial.to_string_lossy(), passphrase],
        )
        .map_err(anyhow::Error::from)
        .and_then(|_| {
            conn.execute_batch(&format!(
                "SELECT sqlcipher_export('snapshot');
                 PRAGMA snapshot.user_version = {user_version};"
            ))
            .map_err(anyhow::Error::from)
        });
    let _ = conn.execute_batch("DETACH DATABASE snapshot;");
    if let Err(err) = exported {
        let _ = std::fs::remove_file(&partial);
        return Err(err).context("export archive snapshot");
    }
    std::fs::rename(&partial, dest).context("move archive snapshot into place")?;
    Ok(())
}

/// Whether two paths name the same file, comparing resolved directories so a
/// path spelled through a symlink or `..` still matches.
fn same_file_path(a: &Path, b: &Path) -> bool {
    let resolve = |path: &Path| {
        let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty());
        let dir = dir
            .map(|dir| dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        path.file_name().map(|name| dir.join(name))
    };
    matches!((resolve(a), resolve(b)), (Some(a), Some(b)) if a == b)
}

fn archive_path() -> PathBuf {
    if let Ok(path) = std::env::var("MERON_ARCHIVE_DB") {
        return PathBuf::from(path);
    }
    app_dir().join("archive.db")
}

/// Open an unencrypted store. SQLCipher leaves a database untouched until a key
/// is set, so this stays byte-compatible with pre-encryption databases and is
/// what tests and the headless/`MERON_KEYRING=off` path use.
pub fn open_at(path: impl AsRef<Path>) -> Result<Connection> {
    open_inner(path.as_ref(), None, || {})
}

/// Open an encrypted store, keying the connection with `key` (64 hex chars = a
/// raw 32-byte SQLCipher key). A legacy plaintext database at `path` is migrated
/// in place to an encrypted one on first keyed open.
pub fn open_at_keyed(path: impl AsRef<Path>, key: &str) -> Result<Connection> {
    open_inner(path.as_ref(), Some(key), || {})
}

fn open_inner(path: &Path, key: Option<&str>, on_upgrade: impl FnOnce()) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut on_upgrade = Some(on_upgrade);
    let mut announce_upgrade = || {
        if let Some(on_upgrade) = on_upgrade.take() {
            on_upgrade();
        }
    };
    let mut conn = open_keyed_connection(path, key, &mut announce_upgrade)?;
    configure_connection(&mut conn)?;
    if upgrade_pending(&conn) {
        announce_upgrade();
    }
    with_anyhow_busy_retry(|| run_migrations(&conn)).context("run migrations")?;
    // The built-in "Archive" Meron folder (main store only, not archive.db).
    with_anyhow_busy_retry(|| super::ensure_meron_archive_folder(&conn))
        .context("ensure the Archive Meron folder")?;
    with_anyhow_busy_retry(|| invalidate_body_cache_if_needed(&conn))
        .context("invalidate body cache if needed")?;
    Ok(conn)
}

fn configure_connection(conn: &mut Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_millis(5000))
        .context("set busy timeout")?;
    // Every store transaction writes, and most read before their first write
    // (`upsert_messages`, `move_messages_by_uid`, ...). Under WAL a DEFERRED
    // transaction starts as a reader, and its first write then fails at once
    // with SQLITE_BUSY ("database is locked") when another connection holds
    // or has just committed a write: SQLite does not run the busy handler for
    // a reader asking to become a writer. Several connections share this file
    // (each command's own, the engine's, IDLE syncs), so take the write lock
    // at BEGIN, where the busy timeout above does wait.
    conn.set_transaction_behavior(TransactionBehavior::Immediate);
    // WAL + synchronous=NORMAL is the durable-but-fast desktop combo; busy_timeout
    // avoids spurious SQLITE_BUSY now that a reader can overlap a writer under WAL.
    with_busy_retry(|| {
        conn.execute_batch(
            // foreign_keys is per-connection and off by default. No table declares a
            // FOREIGN KEY today, so this is a no-op for now — it's set so that any FK
            // constraints added in a future migration are enforced automatically.
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )
    })
    .context("set connection pragmas")?;
    Ok(())
}

/// Opens `path` and applies the SQLCipher `key` (if any). When the key is set
/// but the file turns out to be an unencrypted legacy database, it is migrated
/// in place to an encrypted database before returning, calling `on_encrypt`
/// first because that exports the whole file.
fn open_keyed_connection(
    path: &Path,
    key: Option<&str>,
    on_encrypt: impl FnOnce(),
) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("open db {}", path.display()))?;
    let Some(key) = key else {
        return Ok(conn);
    };
    apply_key(&conn, key)?;
    if database_readable(&conn) {
        return Ok(conn);
    }
    // The key didn't unlock the file. Either it predates encryption and is
    // plaintext (the one-time upgrade below), or we were handed the wrong key —
    // which happens when the keychain lost our entry and minted a fresh one.
    // Refuse the migration in that case: it would fail anyway, and the explicit
    // error names the real problem instead of blaming the schema.
    if is_encrypted_database(path) {
        anyhow::bail!(
            "{} is encrypted with a different key — the keychain entry holding \
             the store key is missing or was replaced",
            path.display()
        );
    }
    drop(conn);
    on_encrypt();
    encrypt_plaintext_db(path, key)
        .with_context(|| format!("encrypt legacy plaintext db {}", path.display()))?;
    let conn = Connection::open(path).with_context(|| format!("reopen db {}", path.display()))?;
    apply_key(&conn, key)?;
    if !database_readable(&conn) {
        anyhow::bail!(
            "database still unreadable after encryption migration: {}",
            path.display()
        );
    }
    Ok(conn)
}

/// Apply the raw 32-byte key (as 64 hex chars). Using the raw-key form makes
/// SQLCipher skip PBKDF2 derivation, so opening is cheap and deterministic.
fn apply_key(conn: &Connection, key: &str) -> Result<()> {
    conn.execute_batch(&format!("PRAGMA key = \"x'{key}'\";"))
        .context("apply sqlcipher key")
}

/// Whether `path` holds an existing *encrypted* database, i.e. a non-empty file
/// that does not carry SQLite's plaintext header. Used to tell "wrong key" from
/// "legacy plaintext database" without guessing.
fn is_encrypted_database(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 16];
    match file.read_exact(&mut header) {
        Ok(()) => &header != b"SQLite format 3\0",
        // Shorter than a header: a freshly created or empty file, not encrypted.
        Err(_) => false,
    }
}

/// A cheap probe that succeeds only when the page cipher matches the file: a
/// plaintext file opened with a key (or a wrong key) fails to decrypt here.
fn database_readable(conn: &Connection) -> bool {
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })
    .is_ok()
}

/// One-time upgrade of a plaintext database to an encrypted one. Exports the
/// plaintext contents into a fresh encrypted sibling via `sqlcipher_export`,
/// then atomically replaces the original and drops its now-stale WAL/SHM files.
fn encrypt_plaintext_db(path: &Path, key: &str) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("meron.db");
    let tmp = path.with_file_name(format!("{file_name}.sqlcipher-migrating"));
    let _ = std::fs::remove_file(&tmp);
    {
        let plain =
            Connection::open(path).with_context(|| format!("open plaintext {}", path.display()))?;
        // Fold any WAL frames into the main file so the export sees committed data.
        let _ = plain.pragma_update(None, "journal_mode", "DELETE");
        // `sqlcipher_export` copies the schema and rows but not `user_version`,
        // so carry it across explicitly — otherwise migrations re-run on the
        // encrypted copy and trip over the already-migrated schema.
        let user_version: i64 = plain.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        plain
            .execute_batch(&format!(
                "ATTACH DATABASE '{}' AS encrypted KEY \"x'{key}'\";
                 SELECT sqlcipher_export('encrypted');
                 PRAGMA encrypted.user_version = {user_version};
                 DETACH DATABASE encrypted;",
                sql_single_quote(&tmp)
            ))
            .context("sqlcipher_export to encrypted db")?;
    }
    std::fs::rename(&tmp, path).context("replace plaintext db with encrypted db")?;
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(sibling_with_suffix(path, suffix));
    }
    Ok(())
}

fn sibling_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn sql_single_quote(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn with_busy_retry<T>(mut f: impl FnMut() -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut delay = Duration::from_millis(10);
    loop {
        match f() {
            Err(err) if is_database_locked(&err) && Instant::now() < deadline => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(100));
            }
            result => return result,
        }
    }
}

fn with_anyhow_busy_retry<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut delay = Duration::from_millis(10);
    loop {
        match f() {
            Err(err) if anyhow_error_is_database_locked(&err) && Instant::now() < deadline => {
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(100));
            }
            result => return result,
        }
    }
}

fn anyhow_error_is_database_locked(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<rusqlite::Error>()
            .is_some_and(is_database_locked)
    })
}

fn is_database_locked(err: &rusqlite::Error) -> bool {
    matches!(
        err,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            )
    )
}

fn db_path() -> PathBuf {
    if let Ok(path) = std::env::var("MERON_CORE_DB") {
        return PathBuf::from(path);
    }
    config_dir().join("meron.db")
}

/// The directory holding the store, and the natural home for anything else that
/// belongs to this app profile (the local keyring, for one). Follows
/// `MERON_CORE_DB`, so dev and production profiles stay separate.
pub fn app_dir() -> PathBuf {
    db_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(config_dir)
}

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config/meron")
}

// ---- Migrations -------------------------------------------------------------

/// The `user_version` [`run_migrations`] leaves behind.
pub(super) const SCHEMA_VERSION: i64 = 31;

/// Whether opening this store will rewrite existing data: a schema behind
/// [`SCHEMA_VERSION`], or bodies rendered by an older version. Only decides
/// whether to tell the user, so a failed read just answers "no".
fn upgrade_pending(conn: &Connection) -> bool {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap_or(0);
    if version == 0 {
        return false;
    }
    version < SCHEMA_VERSION
        || meta_get(conn, "body_cache_version")
            .is_ok_and(|current| current.as_deref() != Some(BODY_CACHE_VERSION))
}

/// Apply schema migrations in order, tracked by SQLite's `PRAGMA user_version`.
///
/// Pending steps run inside one IMMEDIATE transaction that also bumps
/// `user_version`, so concurrent first-open callers serialize before reading
/// the version. A crash mid-migration rolls back cleanly and the step re-runs
/// next launch. Append-only: to evolve the schema, add a new `if version < N`
/// block; never edit or reorder a shipped one. (Cache-only invalidation lives in
/// `invalidate_body_cache_if_needed`, kept off this counter so a render change
/// doesn't look like a schema change.)
///
/// The local-folder archive runs these too (`open_archive_at`), and there the
/// `messages` rows are not a cache: a migration may rebuild them, but never
/// drop cached mail on the assumption that a sync will bring it back.
pub(super) fn run_migrations(conn: &Connection) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    if version < 1 {
        migrate_v1(&tx)?;
    }
    if version < 2 {
        migrate_v2(&tx)?;
    }
    if version < 3 {
        migrate_v3(&tx)?;
    }
    if version < 4 {
        migrate_v4(&tx)?;
    }
    if version < 5 {
        migrate_v5(&tx)?;
    }
    if version < 6 {
        migrate_v6(&tx)?;
    }
    if version < 7 {
        migrate_v7(&tx)?;
    }
    if version < 8 {
        migrate_v8(&tx)?;
    }
    if version < 9 {
        migrate_v9(&tx)?;
    }
    if version < 10 {
        migrate_v10(&tx)?;
    }
    if version < 11 {
        migrate_v11(&tx)?;
    }
    if version < 12 {
        migrate_v12(&tx)?;
    }
    if version < 13 {
        migrate_v13(&tx)?;
    }
    if version < 14 {
        migrate_v14(&tx)?;
    }
    if version < 15 {
        migrate_v15(&tx)?;
    }
    if version < 16 {
        migrate_v16(&tx)?;
    }
    if version < 17 {
        migrate_v17(&tx)?;
    }
    if version < 18 {
        migrate_v18(&tx)?;
    }
    if version < 19 {
        migrate_v19(&tx)?;
    }

    if version < 20 {
        migrate_v20(&tx)?;
    }
    if version < 21 {
        migrate_v21(&tx)?;
    }
    if version < 22 {
        migrate_v22(&tx)?;
    }
    if version < 23 {
        migrate_v23(&tx)?;
    }
    if version < 24 {
        migrate_v24(&tx)?;
    }
    if version < 25 {
        migrate_v25(&tx)?;
    }
    if version < 26 {
        migrate_v26(&tx)?;
    } else {
        repair_v26_meron_uids(&tx)?;
    }
    if version < 27 {
        migrate_v27(&tx)?;
    }
    if version < 28 {
        migrate_v28(&tx)?;
    }
    if version < 29 {
        migrate_v29(&tx)?;
    }
    if version < 30 {
        migrate_v30(&tx)?;
    }
    if version < 31 {
        migrate_v31(&tx)?;
    }
    tx.commit()?;
    Ok(())
}

/// A database as v5 left it, for tests that need to migrate an *existing*
/// install rather than a fresh one.
#[cfg(test)]
pub(super) fn migrate_to_v5(conn: &Connection) -> Result<()> {
    migrate_v1(conn)?;
    migrate_v2(conn)?;
    migrate_v3(conn)?;
    migrate_v4(conn)?;
    migrate_v5(conn)
}

#[cfg(test)]
pub(super) fn migrate_to_v8(conn: &Connection) -> Result<()> {
    migrate_to_v5(conn)?;
    migrate_v6(conn)?;
    migrate_v7(conn)?;
    migrate_v8(conn)
}

pub(super) fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM meta WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()?)
}

pub(super) fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Track rendered-body cache format separately from schema version.
///
/// When the render format changes, re-render every HTML-derived body in place
/// (`body_is_rendered = 1`) from its stored `body_html`. Re-rendering rather than
/// nulling keeps the FTS `body` column populated, so full-text search over these
/// messages keeps working across a version bump instead of degrading until each
/// message is reopened.
pub(super) fn invalidate_body_cache_if_needed(conn: &Connection) -> Result<()> {
    let current = meta_get(conn, "body_cache_version")?;
    if current.as_deref() == Some(BODY_CACHE_VERSION) {
        return Ok(());
    }

    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare(
            "SELECT id, json_extract(json, '$.body_html') FROM messages
             WHERE json_extract(json, '$.body_is_rendered') = 1",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (id, html) in rows {
            // No HTML source to re-render from: drop the stale body and let the
            // read path fall back if a source ever reappears.
            let body = html.as_deref().map(crate::parse::render_body);
            tx.execute(
                "UPDATE messages SET body = ?2 WHERE id = ?1",
                params![id, body],
            )?;
        }
    }
    meta_set(&tx, "body_cache_version", BODY_CACHE_VERSION)?;
    tx.commit()?;
    Ok(())
}
