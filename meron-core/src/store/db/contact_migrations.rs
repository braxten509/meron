//! Contact migrations for the local mail database.
//!
//! The database opener runs migrations in schema-version order.
//! Existing profiles and fresh installations share the same upgrade path.

use super::*;

/// Recipient autocomplete's directory (migration v17): one row per account,
/// address and display name seen as a mail message's sender, To or Cc,
/// counting the messages it appears on. Keeping each name apart means a name
/// leaves with the last message carrying it. Suggesting from the messages
/// themselves meant reading and parsing every cached row's recipient lists per
/// keystroke. Triggers on `messages` keep it in step with every write path;
/// the update trigger only fires when the addresses could have changed, as an
/// envelope upsert rewrites `json` on every resync.
fn contacts_ddl() -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS contacts (
           account  TEXT NOT NULL,
           addr_key TEXT NOT NULL,   -- lower(trim(addr))
           name     TEXT NOT NULL,   -- '' when the address came without one
           addr     TEXT NOT NULL,
           count    INTEGER NOT NULL DEFAULT 0,
           PRIMARY KEY (account, addr_key, name)
         ) WITHOUT ROWID;
         {}",
        contacts_triggers(false)
    )
}

/// The `messages` triggers that keep `contacts` in step. With `track_date`
/// (migration v23) they also keep `last_date`, the newest message an entry was
/// seen on. Removing a message can't lower it without rescanning that
/// address's mail, so it only ever moves forward until the entry goes.
pub(super) fn contacts_triggers(track_date: bool) -> String {
    let add = |row: &str| {
        let (date_col, date_val, date_set) = if track_date {
            (
                ", last_date",
                format!(", {row}.date"),
                ", last_date = max(last_date, excluded.last_date)",
            )
        } else {
            ("", String::new(), "")
        };
        format!(
            "INSERT INTO contacts(account, addr_key, name, addr, count{date_col})
               SELECT {row}.account, lower(trim(addr)), trim(COALESCE(name, '')), max(trim(addr)), 1{date_val}
               FROM ({occurrences}) WHERE {row}.uid <> 0 AND trim(COALESCE(addr, '')) <> ''
               GROUP BY lower(trim(addr)), trim(COALESCE(name, ''))
             ON CONFLICT(account, addr_key, name) DO UPDATE SET count = count + 1{date_set};",
            occurrences = contact_occurrences(row),
        )
    };
    let remove = |row: &str| {
        let keys = format!(
            "SELECT lower(trim(addr)), trim(COALESCE(name, '')) FROM ({})
               WHERE trim(COALESCE(addr, '')) <> ''",
            contact_occurrences(row)
        );
        format!(
            "UPDATE contacts SET count = count - 1
               WHERE account = {row}.account AND {row}.uid <> 0 AND (addr_key, name) IN ({keys});
             DELETE FROM contacts
               WHERE account = {row}.account AND count <= 0 AND (addr_key, name) IN ({keys});"
        )
    };
    let addressees = |row: &str| format!("json_extract({row}.json, '$.to', '$.cc')");
    // The date only matters to `last_date`, so without it a date change
    // alone doesn't rewrite the directory.
    let date_changed = if track_date {
        "\n             OR old.date IS NOT new.date"
    } else {
        ""
    };
    let date_column = if track_date { ", date" } else { "" };
    format!(
        "CREATE TRIGGER IF NOT EXISTS contacts_ai AFTER INSERT ON messages BEGIN
           {add_new}
         END;
         CREATE TRIGGER IF NOT EXISTS contacts_ad AFTER DELETE ON messages BEGIN
           {remove_old}
         END;
         CREATE TRIGGER IF NOT EXISTS contacts_au
           AFTER UPDATE OF account, uid, from_name, from_addr, json{date_column} ON messages
           WHEN old.account IS NOT new.account
             OR (old.uid <> 0) IS NOT (new.uid <> 0)
             OR old.from_name IS NOT new.from_name
             OR old.from_addr IS NOT new.from_addr
             OR {old_addressees} IS NOT {new_addressees}{date_changed}
         BEGIN
           {remove_old}
           {add_new}
         END;",
        add_new = add("new"),
        remove_old = remove("old"),
        old_addressees = addressees("old"),
        new_addressees = addressees("new"),
    )
}

/// `row`'s sender and its To and Cc entries, as `(name, addr)` rows.
pub(super) fn contact_occurrences(row: &str) -> String {
    let list = |path: &str| {
        format!(
            "SELECT json_extract(value, '$.name'), json_extract(value, '$.addr')
               FROM json_each({row}.json, '{path}') WHERE type = 'object'"
        )
    };
    format!(
        "SELECT {row}.from_name AS name, {row}.from_addr AS addr
         UNION ALL {} UNION ALL {}",
        list("$.to"),
        list("$.cc"),
    )
}

pub(super) fn migrate_v17(conn: &Connection) -> Result<()> {
    conn.execute_batch(&contacts_ddl())?;
    let list = |path: &str| {
        format!(
            "SELECT m.id, m.account, json_extract(e.value, '$.name'), json_extract(e.value, '$.addr')
               FROM messages m, json_each(m.json, '{path}') e
               WHERE m.uid <> 0 AND e.type = 'object'"
        )
    };
    conn.execute_batch(&format!(
        "INSERT INTO contacts(account, addr_key, name, addr, count)
           SELECT account, addr_key, name, max(addr), count(*) FROM (
             SELECT id, account, lower(trim(addr)) AS addr_key, trim(COALESCE(name, '')) AS name,
                    max(trim(addr)) AS addr
             FROM (
               SELECT id, account, from_name AS name, from_addr AS addr FROM messages WHERE uid <> 0
               UNION ALL {} UNION ALL {}
             )
             WHERE trim(COALESCE(addr, '')) <> ''
             GROUP BY id, lower(trim(addr)), trim(COALESCE(name, ''))
           )
           WHERE true
           GROUP BY account, addr_key, name;
         PRAGMA user_version = 17;",
        list("$.to"),
        list("$.cc"),
    ))?;
    Ok(())
}

/// Calendar invitations the user answered from the card (migration v18), keyed
/// by the event rather than the message so every copy of the invite — the
/// original, an update, the one in another folder — shows the same answer.
/// `sequence` is the revision answered: a later one asks again.
pub(super) fn migrate_v18(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS invite_responses (
           account       TEXT NOT NULL,
           uid           TEXT NOT NULL,     -- iCalendar UID
           recurrence_id INTEGER NOT NULL DEFAULT 0,   -- 0 = the whole series
           response      TEXT NOT NULL,     -- accepted | tentative | declined
           sequence      INTEGER NOT NULL DEFAULT 0,
           responded_at  INTEGER NOT NULL DEFAULT 0,
           PRIMARY KEY (account, uid, recurrence_id)
         );
         PRAGMA user_version = 18;",
    )?;
    Ok(())
}

/// The local calendar (migration v19): events the user made, and invitations they accepted. Like
/// tasks, a table rather than a blob, queried by time. `recurring` rows are expanded on read from
/// `rrule` (an event made here) or `ics` (an invitation, whose own rules and exceptions are kept
/// whole), so a range query takes every recurring row that starts before its end.
pub(super) fn migrate_v19(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
           id          TEXT PRIMARY KEY,
           title       TEXT NOT NULL DEFAULT '',
           notes       TEXT NOT NULL DEFAULT '',
           location    TEXT NOT NULL DEFAULT '',
           start_at    INTEGER NOT NULL DEFAULT 0,   -- all-day: UTC midnight
           end_at      INTEGER NOT NULL DEFAULT 0,   -- exclusive
           all_day     INTEGER NOT NULL DEFAULT 0,
           recurring   INTEGER NOT NULL DEFAULT 0,
           rrule       TEXT NOT NULL DEFAULT '',     -- daily | weekly | monthly | yearly
           tzid        TEXT NOT NULL DEFAULT '',     -- the zone a repeat keeps its time in
           account     TEXT NOT NULL DEFAULT '',     -- mail link; empty for an event made here
           thread_id   TEXT NOT NULL DEFAULT '',
           message_id  TEXT NOT NULL DEFAULT '',
           invite_uid  TEXT NOT NULL DEFAULT '',     -- iCalendar UID of an accepted invitation
           recurrence_id INTEGER NOT NULL DEFAULT 0, -- an invitation to one occurrence alone; 0 = the event
           response    TEXT NOT NULL DEFAULT '',     -- accepted | tentative
           ics         TEXT NOT NULL DEFAULT '',
           json        TEXT NOT NULL DEFAULT '{}',
           created_at  INTEGER NOT NULL DEFAULT 0,
           updated_at  INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS events_start_idx ON events(start_at);
         -- A decline (or cancellation) of an occurrence that stands for every
         -- later one too (RANGE=THISANDFUTURE).
         ALTER TABLE invite_responses ADD COLUMN this_and_future INTEGER NOT NULL DEFAULT 0;
         CREATE UNIQUE INDEX IF NOT EXISTS events_invite_idx
           ON events(account, invite_uid, recurrence_id) WHERE invite_uid <> '';
         PRAGMA user_version = 19;",
    )?;
    Ok(())
}
