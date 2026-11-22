//! Migrations 01 to 16 for the local mail database.
//!
//! The database opener runs migrations in schema-version order.
//! Existing profiles and fresh installations share the same upgrade path.

use super::*;

pub(super) fn migrate_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(ACCOUNTS_DDL)?;
    conn.execute_batch(MESSAGES_DDL)?;
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch("PRAGMA user_version = 1;")?;
    Ok(())
}

/// Per-subscription extra metadata that doesn't warrant its own typed column
/// (feed icon/logo, …), stored as a JSON object. Mirrors the `messages.json`
/// approach; defaults to an empty object so existing rows stay valid.
pub(super) fn migrate_v2(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE subscriptions ADD COLUMN json TEXT NOT NULL DEFAULT '{}';")?;
    conn.execute_batch("PRAGMA user_version = 2;")?;
    Ok(())
}

/// Per-account secret blob (IMAP password / OAuth tokens) for platforms without
/// an OS keychain (Android, iOS sandbox). Desktop continues to use the keychain
/// via the `secrets` module; only the mobile FFI path reads/writes this table.
pub(super) fn migrate_v3(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS account_secrets (
           account_id TEXT PRIMARY KEY,
           blob       TEXT NOT NULL
         );",
    )?;
    conn.execute_batch("PRAGMA user_version = 3;")?;
    Ok(())
}

/// RFC 6154 special-use role reported by LIST for a folder ("drafts", "sent",
/// …), NULL when the server doesn't advertise one. Lets role lookups (which
/// folder holds drafts?) trust the server over name heuristics.
pub(super) fn migrate_v4(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE folders ADD COLUMN special_use TEXT;")?;
    conn.execute_batch("PRAGMA user_version = 4;")?;
    Ok(())
}

/// Stable message identities we've already seen, used to suppress "new mail"
/// notifications when Gmail restores an older message with a fresh INBOX UID.
pub(super) fn migrate_v5(conn: &Connection) -> Result<()> {
    conn.execute_batch(OBSERVED_MAIL_IDENTITIES_DDL)?;
    conn.execute_batch("PRAGMA user_version = 5;")?;
    Ok(())
}

/// Searchable `To`/`Cc` text plus the FTS index over it, so a lookup by recipient finds cached mail
/// the way a lookup by sender already does. Existing rows are backfilled from `json` through the
/// same formatter the write path uses, then indexed in one `rebuild` — cheaper than letting the
/// triggers fire per row.
pub(super) fn migrate_v6(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE messages ADD COLUMN recipients TEXT;")?;
    backfill_recipients(conn)?;
    conn.execute_batch(MESSAGES_RECIPIENTS_FTS_DDL)?;
    conn.execute_batch(
        "INSERT INTO messages_recipients_fts(messages_recipients_fts) VALUES('rebuild');",
    )?;
    conn.execute_batch("PRAGMA user_version = 6;")?;
    Ok(())
}

/// Stable server-search snapshots. IMAP SEARCH returns an unordered UID set,
/// while the UI pages by message date; retaining the resolved order prevents a
/// later page from losing messages whose Date header does not follow UID order.
pub(super) fn migrate_v7(conn: &Connection) -> Result<()> {
    conn.execute_batch(MAIL_SEARCH_HITS_DDL)?;
    conn.execute_batch("PRAGMA user_version = 7;")?;
    Ok(())
}

/// Access paths for thread-key inheritance, which until now scanned the whole
/// account cache per upserted reply. See `MESSAGES_THREAD_KEY_INDEXES_DDL`.
pub(super) fn migrate_v8(conn: &Connection) -> Result<()> {
    conn.execute_batch(MESSAGES_THREAD_KEY_INDEXES_DDL)?;
    conn.execute_batch("PRAGMA user_version = 8;")?;
    Ok(())
}

/// Avoid reindexing unchanged headers and observe cached identities immediately.
/// Sync classifies arrivals before writing its fetched batch.
pub(super) fn migrate_v9(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TRIGGER messages_au;
         CREATE TRIGGER messages_au AFTER UPDATE OF subject, from_name, from_addr, body ON messages
         WHEN old.subject IS NOT new.subject OR old.from_name IS NOT new.from_name
           OR old.from_addr IS NOT new.from_addr OR old.body IS NOT new.body
         BEGIN
           INSERT INTO messages_fts(messages_fts, rowid, subject, from_name, from_addr, body)
           VALUES ('delete', old.id, old.subject, old.from_name, old.from_addr, old.body);
           INSERT INTO messages_fts(rowid, subject, from_name, from_addr, body)
           VALUES (new.id, new.subject, new.from_name, new.from_addr, new.body);
         END;
         DROP TRIGGER messages_recipients_au;
         CREATE TRIGGER messages_recipients_au AFTER UPDATE OF recipients ON messages
         WHEN old.recipients IS NOT new.recipients
         BEGIN
           INSERT INTO messages_recipients_fts(messages_recipients_fts, rowid, recipients)
           VALUES ('delete', old.id, old.recipients);
           INSERT INTO messages_recipients_fts(rowid, recipients) VALUES (new.id, new.recipients);
         END;",
    )?;
    conn.execute_batch(
        "INSERT OR IGNORE INTO observed_mail_identities(account, identity, first_seen_at)
         SELECT account, CASE
           WHEN json_extract(json, '$.gmail_msg_id') IS NOT NULL THEN 'gmail:' || json_extract(json, '$.gmail_msg_id')
           WHEN COALESCE(json_extract(json, '$.message_id'), '') <> '' THEN 'message-id:' || lower(json_extract(json, '$.message_id'))
           ELSE 'uid:' || lower(folder) || ':' || uid END, unixepoch()
         FROM messages WHERE uid <> 0;
         DROP TRIGGER IF EXISTS pending_mail_identity_ai;
         DROP TRIGGER IF EXISTS pending_mail_identity_au;
         DROP TRIGGER IF EXISTS pending_mail_identity_ad;
         DROP TABLE IF EXISTS pending_mail_identities;
         DROP TRIGGER IF EXISTS observe_mail_identity_ai;
         DROP TRIGGER IF EXISTS observe_mail_identity_au;",
    )?;
    for (name, event) in [
        ("ai", "INSERT"),
        ("au", "UPDATE OF json, uid, folder, account"),
    ] {
        let changed = if name == "au" {
            " AND (old.json IS NOT new.json OR old.uid IS NOT new.uid OR old.folder IS NOT new.folder OR old.account IS NOT new.account)"
        } else {
            ""
        };
        conn.execute_batch(&format!(
            "CREATE TRIGGER observe_mail_identity_{name} AFTER {event} ON messages WHEN new.uid <> 0{changed} BEGIN
             INSERT INTO observed_mail_identities(account, identity, first_seen_at)
             VALUES(new.account, CASE
               WHEN json_extract(new.json, '$.gmail_msg_id') IS NOT NULL THEN 'gmail:' || json_extract(new.json, '$.gmail_msg_id')
               WHEN COALESCE(json_extract(new.json, '$.message_id'), '') <> '' THEN 'message-id:' || lower(json_extract(new.json, '$.message_id'))
               ELSE 'uid:' || lower(new.folder) || ':' || new.uid END, unixepoch())
             ON CONFLICT DO NOTHING;
             END;"
        ))?;
    }
    conn.execute_batch("PRAGMA user_version = 9;")?;
    Ok(())
}

/// Tasks: lists and their items, for the optional Tasks surface. Local-only —
/// nothing here is synced to a server, so the rows are also carried by the
/// config backup.
pub(super) fn migrate_v10(conn: &Connection) -> Result<()> {
    conn.execute_batch(TASKS_DDL)?;
    conn.execute_batch("PRAGMA user_version = 10;")?;
    Ok(())
}

/// `messages.files`: the cached body's real attachments as a JSON array of
/// `{filename, mime}`, NULL when it has none, so list cards can name them and
/// the attachments filter can page without parsing every row's `json`. Written
/// with header BODYSTRUCTURE metadata and refined by `save_cached_message`.
///
/// Rows cached before this are filled by [`crate::store::backfill_files_batch`] after
/// startup, not here: deriving them parses every row's `json` (HTML bodies
/// included), which on a large cache holds the store long enough for the
/// frontend's first requests to time out.
pub(super) fn migrate_v11(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE messages ADD COLUMN files TEXT;")?;
    conn.execute_batch("PRAGMA user_version = 11;")?;
    Ok(())
}

/// One row per live search: the identity a snapshot page is validated against.
///
/// A first page fetches only the newest hits; the rest wait in `mail_search_pending`, one row per
/// server hit with its Date header resolved up front, so batches are taken newest-first by date
/// across every folder rather than by UID, which orders mailbox insertion and not send time.
///
/// `cache_resume` is set when a batch cut off its cached-only hits: the keyset cursor of the last
/// one it took, from which paging past the snapshot continues through the cache.
const MAIL_SEARCH_SNAPSHOTS_DDL: &str = "
CREATE TABLE IF NOT EXISTS mail_search_snapshots (
  token        TEXT PRIMARY KEY,
  account      TEXT NOT NULL,
  query        TEXT NOT NULL,
  scope        TEXT NOT NULL,
  cache_resume TEXT,
  created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS mail_search_snapshots_created_idx
  ON mail_search_snapshots(account, created_at);
CREATE TABLE IF NOT EXISTS mail_search_pending (
  token   TEXT NOT NULL,
  account TEXT NOT NULL,
  folder  TEXT NOT NULL,
  uid     INTEGER NOT NULL,
  date    INTEGER NOT NULL,
  PRIMARY KEY (token, folder, uid)
);
CREATE INDEX IF NOT EXISTS mail_search_pending_order_idx
  ON mail_search_pending(token, date DESC, uid DESC, folder DESC);
CREATE INDEX IF NOT EXISTS mail_search_pending_account_idx
  ON mail_search_pending(account);
";

/// Resumable live searches; see `MAIL_SEARCH_SNAPSHOTS_DDL`. Hit rows saved by
/// earlier versions have no snapshot row, so their cursors fall back to cached
/// keyset paging, as an expired snapshot does.
pub(super) fn migrate_v12(conn: &Connection) -> Result<()> {
    conn.execute_batch(MAIL_SEARCH_SNAPSHOTS_DDL)?;
    conn.execute_batch("PRAGMA user_version = 12;")?;
    Ok(())
}

/// Unseen server UIDs that the last sync found outside the cached window, so a folder's unread
/// total covers mail older than what the cache holds. A row is dropped once its message is cached
/// there, whether inserted or moved in from another folder: the cached row then carries the unread
/// state, and deleting or moving it later can't bring the stale row back into the count.
pub(super) fn migrate_v13(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS uncached_unseen (
           account TEXT NOT NULL,
           folder  TEXT NOT NULL,
           uid     INTEGER NOT NULL,
           PRIMARY KEY (account, folder, uid)
         ) WITHOUT ROWID;
         CREATE TRIGGER IF NOT EXISTS messages_uncached_unseen_ai AFTER INSERT ON messages
         WHEN new.uid <> 0 BEGIN
           DELETE FROM uncached_unseen
            WHERE account = new.account AND folder = new.folder AND uid = new.uid;
         END;
         CREATE TRIGGER IF NOT EXISTS messages_uncached_unseen_au
         AFTER UPDATE OF account, folder, uid ON messages
         WHEN new.uid <> 0 BEGIN
           DELETE FROM uncached_unseen
            WHERE account = new.account AND folder = new.folder AND uid = new.uid;
         END;",
    )?;
    conn.execute_batch("PRAGMA user_version = 13;")?;
    Ok(())
}

/// UIDs this process removed from a folder. A folder sync that fetched its
/// snapshot before the removal must not write those rows back, or an archived
/// thread reappears in the mailbox it just left.
pub(super) fn migrate_v14(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS removed_message_uids (
           account TEXT NOT NULL,
           folder  TEXT NOT NULL,
           uid     INTEGER NOT NULL,
           epoch   INTEGER NOT NULL,
           PRIMARY KEY (account, folder, uid)
         ) WITHOUT ROWID;",
    )?;
    conn.execute_batch("PRAGMA user_version = 14;")?;
    Ok(())
}

/// Folder syncs with a snapshot in flight, shared by every connection so one
/// engine does not prune removal markers another engine's sync still needs.
pub(super) fn migrate_v15(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS active_message_syncs (
           id    INTEGER PRIMARY KEY AUTOINCREMENT,
           epoch INTEGER NOT NULL,
           owner TEXT NOT NULL
         );",
    )?;
    conn.execute_batch("PRAGMA user_version = 15;")?;
    Ok(())
}

/// Per-folder opt-in to live sync and new-mail notifications. INBOX is always
/// watched and never consults the flag.
pub(super) fn migrate_v16(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE folders ADD COLUMN notify INTEGER NOT NULL DEFAULT 0;")?;
    conn.execute_batch("PRAGMA user_version = 16;")?;
    Ok(())
}

fn backfill_recipients(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, json_extract(json, '$.to'), json_extract(json, '$.cc') FROM messages
         WHERE json_extract(json, '$.to') IS NOT NULL OR json_extract(json, '$.cc') IS NOT NULL",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                crate::store::parse_recipients_json(row.get::<_, Option<String>>(1)?),
                crate::store::parse_recipients_json(row.get::<_, Option<String>>(2)?),
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, to, cc) in rows {
        conn.execute(
            "UPDATE messages SET recipients = ?2 WHERE id = ?1",
            params![id, crate::store::recipients_index_text(&to, &cc)],
        )?;
    }
    Ok(())
}
