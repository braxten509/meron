//! Migrations 20 to 25 for the local mail database.
//!
//! The database opener runs migrations in schema-version order.
//! Existing profiles and fresh installations share the same upgrade path.

use super::*;

/// Local notification delivery receipts; deliberately excluded from backups.
pub(super) fn migrate_v20(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS reminder_deliveries (
            key TEXT PRIMARY KEY,
            delivered_at INTEGER NOT NULL
         );
         PRAGMA user_version = 20;",
    )?;
    Ok(())
}

/// An event's own alerts (JSON minutes before start); NULL follows the
/// app-wide default.
pub(super) fn migrate_v21(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE events ADD COLUMN alerts TEXT;
         PRAGMA user_version = 21;",
    )?;
    Ok(())
}

/// Tags (see `store::tags`). `server_tags` is what a mailbox copy last showed, NULL until it has
/// been looked at. The key lives in an expression index rather than a column so every path that
/// writes a message row is covered. Rules (see `store::rules`): `rule_actions` is what they still
/// have to do on the server, so a failed sync keeps it, and `rule_log` what they did.
pub(super) fn migrate_v22(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "ALTER TABLE messages ADD COLUMN server_tags TEXT;
         CREATE INDEX messages_tag_key_idx ON messages(account, {key}) WHERE uid <> 0;
         CREATE TABLE message_tags (
            account    TEXT NOT NULL,
            tag_key    TEXT NOT NULL,
            tag        TEXT NOT NULL,
            state      INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (account, tag_key, tag)
         ) WITHOUT ROWID;
         CREATE INDEX message_tags_tag_idx ON message_tags(account, tag);
         CREATE TABLE rule_actions (
            account     TEXT NOT NULL,
            folder      TEXT NOT NULL,
            uid         INTEGER NOT NULL,
            uidvalidity INTEGER NOT NULL,
            mark_read   INTEGER NOT NULL DEFAULT 0,
            star        INTEGER NOT NULL DEFAULT 0,
            trash       INTEGER NOT NULL DEFAULT 0,
            target      TEXT NOT NULL DEFAULT '',
            copies      TEXT NOT NULL DEFAULT '[]',
            rule_ids    TEXT NOT NULL DEFAULT '[]',
            run         INTEGER NOT NULL DEFAULT 0,
            attempts    INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (account, folder, uid, run)
         ) WITHOUT ROWID;
         CREATE TABLE rule_log (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            at      INTEGER NOT NULL,
            rule_id TEXT NOT NULL,
            account TEXT NOT NULL,
            subject TEXT NOT NULL DEFAULT '',
            count   INTEGER NOT NULL DEFAULT 1,
            failed  INTEGER NOT NULL DEFAULT 0,
            detail  TEXT NOT NULL DEFAULT ''
         );
         PRAGMA user_version = 22;",
        key = crate::store::tags::TAG_KEY_SQL,
    ))?;
    Ok(())
}

/// When each contact was last seen (migration v23), so the Contacts panel can
/// list the most recent correspondents first. The triggers are replaced with
/// ones that keep `last_date`, and the directory is rebuilt the way v17 built
/// it, now with each entry's newest message date.
pub(super) fn migrate_v23(conn: &Connection) -> Result<()> {
    let list = |path: &str| {
        format!(
            "SELECT m.id, m.account, m.date, json_extract(e.value, '$.name'), json_extract(e.value, '$.addr')
               FROM messages m, json_each(m.json, '{path}') e
               WHERE m.uid <> 0 AND e.type = 'object'"
        )
    };
    conn.execute_batch(&format!(
        "DROP TRIGGER IF EXISTS contacts_ai;
         DROP TRIGGER IF EXISTS contacts_ad;
         DROP TRIGGER IF EXISTS contacts_au;
         ALTER TABLE contacts ADD COLUMN last_date INTEGER NOT NULL DEFAULT 0;
         {triggers}
         DELETE FROM contacts;
         INSERT INTO contacts(account, addr_key, name, addr, count, last_date)
           SELECT account, addr_key, name, max(addr), count(*), max(date) FROM (
             SELECT id, account, max(date) AS date, lower(trim(addr)) AS addr_key,
                    trim(COALESCE(name, '')) AS name, max(trim(addr)) AS addr
             FROM (
               SELECT id, account, date, from_name AS name, from_addr AS addr FROM messages WHERE uid <> 0
               UNION ALL {to} UNION ALL {cc}
             )
             WHERE trim(COALESCE(addr, '')) <> ''
             GROUP BY id, lower(trim(addr)), trim(COALESCE(name, ''))
           )
           WHERE true
           GROUP BY account, addr_key, name;
         PRAGMA user_version = 23;",
        triggers = contacts_triggers(true),
        to = list("$.to"),
        cc = list("$.cc"),
    ))?;
    Ok(())
}

/// Folder-scoped contacts let readers apply the *current* folder roles, even
/// when LIST metadata arrives after the mail. Participant rows are a compact
/// reverse index: removing the newest message can find the next date with an
/// indexed lookup rather than scanning/parsing that correspondent's mail.
pub(super) fn migrate_v24(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS contacts_ai;
         DROP TRIGGER IF EXISTS contacts_ad;
         DROP TRIGGER IF EXISTS contacts_au;
         DROP TABLE contacts;
         CREATE TABLE contacts (
           account   TEXT NOT NULL,
           folder    TEXT NOT NULL,
           addr_key  TEXT NOT NULL,
           name      TEXT NOT NULL,
           addr      TEXT NOT NULL,
           count     INTEGER NOT NULL,
           last_date INTEGER NOT NULL,
           PRIMARY KEY (account, folder, addr_key, name)
         ) WITHOUT ROWID;
         CREATE TABLE contact_participants (
           message_id INTEGER NOT NULL,
           account    TEXT NOT NULL,
           folder     TEXT NOT NULL,
           addr_key   TEXT NOT NULL,
           name       TEXT NOT NULL,
           addr       TEXT NOT NULL,
           date       INTEGER NOT NULL,
           PRIMARY KEY (message_id, addr_key, name)
         ) WITHOUT ROWID;",
    )?;
    let list = |path: &str| {
        format!(
            "SELECT m.id, m.account, m.folder, m.date,
                    json_extract(e.value, '$.name'), json_extract(e.value, '$.addr')
             FROM messages m, json_each(m.json, '{path}') e
             WHERE m.uid <> 0 AND e.type = 'object'"
        )
    };
    // Build once from cached headers, before installing maintenance triggers.
    conn.execute_batch(&format!(
        "INSERT INTO contact_participants(message_id, account, folder, addr_key, name, addr, date)
           SELECT id, account, folder, lower(trim(addr)), trim(COALESCE(name, '')),
                  max(trim(addr)), date
           FROM (
             SELECT id, account, folder, date, from_name AS name, from_addr AS addr
             FROM messages WHERE uid <> 0
             UNION ALL {to} UNION ALL {cc}
           )
           WHERE trim(COALESCE(addr, '')) <> ''
           GROUP BY id, lower(trim(addr)), trim(COALESCE(name, ''));
         CREATE INDEX contact_participants_date_idx
           ON contact_participants(account, folder, addr_key, name, date DESC);
         INSERT INTO contacts(account, folder, addr_key, name, addr, count, last_date)
           SELECT account, folder, addr_key, name, max(addr), count(*), max(date)
           FROM contact_participants GROUP BY account, folder, addr_key, name;",
        to = list("$.to"),
        cc = list("$.cc"),
    ))?;
    let add = format!(
        "INSERT INTO contact_participants(message_id, account, folder, addr_key, name, addr, date)
           SELECT new.id, new.account, new.folder, lower(trim(addr)), trim(COALESCE(name, '')),
                  max(trim(addr)), new.date
           FROM ({}) WHERE new.uid <> 0 AND trim(COALESCE(addr, '')) <> ''
           GROUP BY lower(trim(addr)), trim(COALESCE(name, ''));",
        contact_occurrences("new"),
    );
    conn.execute_batch(&format!(
        "CREATE TRIGGER contact_participants_ai AFTER INSERT ON contact_participants BEGIN
           INSERT INTO contacts(account, folder, addr_key, name, addr, count, last_date)
           VALUES(new.account, new.folder, new.addr_key, new.name, new.addr, 1, new.date)
           ON CONFLICT(account, folder, addr_key, name) DO UPDATE SET
             count = count + 1, addr = max(addr, excluded.addr),
             last_date = max(last_date, excluded.last_date);
         END;
         CREATE TRIGGER contact_participants_ad AFTER DELETE ON contact_participants BEGIN
           DELETE FROM contacts
             WHERE account = old.account AND folder = old.folder
               AND addr_key = old.addr_key AND name = old.name AND count = 1;
           UPDATE contacts SET count = count - 1,
             last_date = CASE WHEN last_date = old.date THEN (
               SELECT date FROM contact_participants
               WHERE account = old.account AND folder = old.folder
                 AND addr_key = old.addr_key AND name = old.name
               ORDER BY date DESC LIMIT 1
             ) ELSE last_date END
             WHERE account = old.account AND folder = old.folder
               AND addr_key = old.addr_key AND name = old.name;
         END;
         CREATE TRIGGER contacts_ai AFTER INSERT ON messages BEGIN
           {add}
         END;
         CREATE TRIGGER contacts_ad AFTER DELETE ON messages BEGIN
           DELETE FROM contact_participants WHERE message_id = old.id;
         END;
         CREATE TRIGGER contacts_au
           AFTER UPDATE OF id, account, folder, uid, from_name, from_addr, json, date ON messages
           WHEN old.id IS NOT new.id OR old.account IS NOT new.account
             OR old.folder IS NOT new.folder OR (old.uid <> 0) IS NOT (new.uid <> 0)
             OR old.from_name IS NOT new.from_name OR old.from_addr IS NOT new.from_addr
             OR json_extract(old.json, '$.to', '$.cc') IS NOT json_extract(new.json, '$.to', '$.cc')
             OR old.date IS NOT new.date
         BEGIN
           DELETE FROM contact_participants WHERE message_id = old.id;
           {add}
         END;
         PRAGMA user_version = 24;"
    ))?;
    Ok(())
}

/// AI sorting: new Inbox mail held back while the AI decides (`ai_sort_hidden`, ignored once
/// `hide_until` has passed), mail this device has had decided (`ai_sort_done`), what the AI did
/// (`ai_sort_log`), and how many emails automatic sorting sent each day (`ai_sort_usage`, for the
/// daily cap). None of it holds the API key.
pub(super) fn migrate_v25(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ai_sort_hidden (
           account     TEXT NOT NULL,
           folder      TEXT NOT NULL,
           uid         INTEGER NOT NULL,
           uidvalidity INTEGER NOT NULL DEFAULT 0,
           hide_until  INTEGER NOT NULL,
           PRIMARY KEY (account, folder, uid)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS ai_sort_done (
           account  TEXT NOT NULL,
           identity TEXT NOT NULL,
           at       INTEGER NOT NULL,
           PRIMARY KEY (account, identity)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS ai_sort_log (
           id          INTEGER PRIMARY KEY AUTOINCREMENT,
           at          INTEGER NOT NULL,
           account     TEXT NOT NULL,
           uid         INTEGER NOT NULL DEFAULT 0,
           uidvalidity INTEGER NOT NULL DEFAULT 0,
           message_id  TEXT NOT NULL DEFAULT '',
           subject     TEXT NOT NULL DEFAULT '',
           sender      TEXT NOT NULL DEFAULT '',
           to_folder   TEXT NOT NULL DEFAULT '',
           reason      TEXT NOT NULL DEFAULT '',
           state       TEXT NOT NULL,
           detail      TEXT NOT NULL DEFAULT '',
           alert       INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS ai_sort_usage (
           day   TEXT PRIMARY KEY,
           count INTEGER NOT NULL
         ) WITHOUT ROWID;
         PRAGMA user_version = 25;",
    )?;
    Ok(())
}
