//! Tests for the local mail database.
//!
//! The database opener runs migrations in schema-version order.
//! Existing profiles and fresh installations share the same upgrade path.

use super::*;

const TEST_KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn tmp_db_path() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meron-db-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("meron.db")
}

#[test]
fn keyed_open_round_trips() {
    let path = tmp_db_path();
    {
        let conn = open_at_keyed(&path, TEST_KEY).unwrap();
        conn.execute("INSERT INTO settings(key, value) VALUES('k', 'v')", [])
            .unwrap();
    }
    // Reopening with the key sees the data.
    let conn = open_at_keyed(&path, TEST_KEY).unwrap();
    let value: String = conn
        .query_row("SELECT value FROM settings WHERE key='k'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(value, "v");
    // The on-disk file is encrypted: a plaintext open can't read the schema.
    assert!(!database_readable(&Connection::open(&path).unwrap()));
}

#[test]
fn legacy_plaintext_is_migrated_to_encrypted() {
    let path = tmp_db_path();
    // Simulate a pre-encryption store: create it plaintext with some data.
    {
        let conn = open_at(&path).unwrap();
        conn.execute("INSERT INTO settings(key, value) VALUES('k', 'legacy')", [])
            .unwrap();
    }
    assert!(database_readable(&Connection::open(&path).unwrap()));

    // First keyed open migrates the plaintext file in place, preserving data.
    let conn = open_at_keyed(&path, TEST_KEY).unwrap();
    let value: String = conn
        .query_row("SELECT value FROM settings WHERE key='k'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(value, "legacy");
    drop(conn);

    // The file is now encrypted and still readable with the key on reopen.
    assert!(!database_readable(&Connection::open(&path).unwrap()));
    let conn = open_at_keyed(&path, TEST_KEY).unwrap();
    let value: String = conn
        .query_row("SELECT value FROM settings WHERE key='k'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(value, "legacy");
}

#[test]
fn wrong_key_on_an_encrypted_store_is_named_not_migrated() {
    let path = tmp_db_path();
    {
        let conn = open_at_keyed(&path, TEST_KEY).unwrap();
        conn.execute("INSERT INTO settings(key, value) VALUES('k', 'v')", [])
            .unwrap();
    }
    assert!(is_encrypted_database(&path));

    // A keychain that lost our entry hands back a freshly minted key. Opening
    // with it must report that, not run the plaintext migration over an
    // encrypted file.
    let other = "ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";
    let error = open_at_keyed(&path, other).unwrap_err().to_string();
    assert!(error.contains("encrypted with a different key"), "{error}");

    // The original file is untouched: the real key still opens it.
    let conn = open_at_keyed(&path, TEST_KEY).unwrap();
    let value: String = conn
        .query_row("SELECT value FROM settings WHERE key='k'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(value, "v");
}

#[test]
fn plaintext_and_missing_files_are_not_taken_for_encrypted() {
    let path = tmp_db_path();
    assert!(!is_encrypted_database(&path), "missing file");
    drop(open_at(&path).unwrap());
    assert!(!is_encrypted_database(&path), "plaintext file");
}

fn contact_message(conn: &Connection, id: i64, folder: &str, date: i64) {
    conn.execute(
        "INSERT INTO messages(id, account, folder, msg_id, uid, from_name, from_addr, date, json)
         VALUES(?1, 'a', ?2, ?1, ?1, 'Person', 'person@example.com', ?3,
           '{\"to\":[{\"name\":\"Person\",\"addr\":\"person@example.com\"}],\"cc\":[]}')",
        params![id, folder, date],
    )
    .unwrap();
}

#[test]
fn contact_participants_track_moves_deletes_and_lower_dates() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn).unwrap();
    contact_message(&conn, 1, "INBOX", 10);
    contact_message(&conn, 2, "INBOX", 20);
    let tally = |folder: &str| {
        conn.query_row(
            "SELECT count, last_date FROM contacts WHERE account = 'a' AND folder = ?1",
            [folder],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .unwrap()
    };
    // A sender repeated in To counts only once on each message.
    assert_eq!(tally("INBOX"), Some((2, 20)));
    conn.execute("UPDATE messages SET folder = 'Trash' WHERE id = 2", [])
        .unwrap();
    assert_eq!(tally("INBOX"), Some((1, 10)));
    assert_eq!(tally("Trash"), Some((1, 20)));
    conn.execute(
        "UPDATE messages SET folder = 'INBOX', date = 5 WHERE id = 2",
        [],
    )
    .unwrap();
    assert_eq!(tally("Trash"), None);
    assert_eq!(tally("INBOX"), Some((2, 10)));
    conn.execute("DELETE FROM messages WHERE id = 1", [])
        .unwrap();
    assert_eq!(tally("INBOX"), Some((1, 5)));
    conn.execute("UPDATE messages SET uid = 0 WHERE id = 2", [])
        .unwrap();
    assert_eq!(tally("INBOX"), None);
}

#[test]
fn contact_exclusion_uses_current_metadata_and_name_fallback() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn).unwrap();
    contact_message(&conn, 1, "Odd mailbox", 10);
    let visible = || {
        super::super::list_contacts(
            &conn,
            "a",
            "",
            20,
            super::super::ContactSort::Frequent,
            false,
        )
        .unwrap()
        .len()
    };
    assert_eq!(visible(), 1);
    conn.execute(
        "INSERT INTO folders(account, name, special_use) VALUES('a', 'Odd mailbox', 'junk')",
        [],
    )
    .unwrap();
    assert_eq!(visible(), 0);
    conn.execute("UPDATE folders SET special_use = 'trash'", [])
        .unwrap();
    assert_eq!(visible(), 0);
    conn.execute("UPDATE folders SET special_use = NULL", [])
        .unwrap();
    assert_eq!(visible(), 1);
    conn.execute("UPDATE messages SET folder = 'Trash'", [])
        .unwrap();
    assert_eq!(visible(), 0);
    conn.execute(
        "INSERT INTO folders(account, name, special_use) VALUES('a', 'Trash', ?1)",
        [super::super::LOCAL_SPECIAL_USE],
    )
    .unwrap();
    assert_eq!(visible(), 1);
}

#[test]
fn contact_v24_backfills_existing_cache_and_is_idempotent() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn).unwrap();
    let latest: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    // Restore the v23 directory shape without changing the message cache.
    conn.execute_batch(
        "DROP TRIGGER contacts_ai; DROP TRIGGER contacts_ad; DROP TRIGGER contacts_au;
         DROP TABLE contact_participants; DROP TABLE contacts;
         CREATE TABLE contacts(account TEXT, addr_key TEXT, name TEXT, addr TEXT,
           count INTEGER, last_date INTEGER);
         PRAGMA user_version = 23;",
    )
    .unwrap();
    contact_message(&conn, 1, "INBOX", 10);
    contact_message(&conn, 2, "Trash", 20);
    // Replay step 24 alone: later steps add columns the store already
    // has, which cannot run twice.
    migrate_v24(&conn).unwrap();
    conn.execute_batch(&format!("PRAGMA user_version = {latest};"))
        .unwrap();
    run_migrations(&conn).unwrap();
    let entries: i64 = conn
        .query_row("SELECT count(*) FROM contact_participants", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(entries, 2);
    let contacts = super::super::suggest_contacts(&conn, "a", "person", 10).unwrap();
    assert_eq!(contacts.len(), 1);
    conn.execute("DELETE FROM messages WHERE folder = 'INBOX'", [])
        .unwrap();
    assert!(
        super::super::suggest_contacts(&conn, "a", "person", 10)
            .unwrap()
            .is_empty()
    );
}
