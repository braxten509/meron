//! Exercises replacement of a cached folder after its UID validity changes.
//! The fixture seeds SQLite with headers and state for the old mailbox.
//! It verifies the new headers, folder cursor and modification sequence together.

use crate::engine::store_folder_tail;
use crate::imap;
use rusqlite::Connection;

#[test]
fn a_folder_tail_read_resets_the_cache_when_uidvalidity_changes() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, date)
         VALUES('acct', 'Sent', 'old-1', 4, 'old generation', 100)",
        [],
    )
    .unwrap();
    crate::store::set_folder_state(&conn, "acct", "Sent", 111, 5).unwrap();
    crate::store::set_folder_modseq(&conn, "acct", "Sent", 42).unwrap();

    // Same generation: the tail read adds to what is cached.
    let batch = imap::RecentBatch {
        uidvalidity: 111,
        uid_next: 6,
        messages: vec![imap::MessageHeader {
            uid: 5,
            message_id: "new-1".to_string(),
            ..Default::default()
        }],
    };
    store_folder_tail(&conn, "acct", "Sent", &batch).unwrap();
    assert_eq!(cached_uids(&conn), vec![4, 5]);
    assert_eq!(
        crate::store::get_folder_modseq(&conn, "acct", "Sent").unwrap(),
        42
    );

    // A new generation: the old UIDs and the modseq that indexed them go,
    // or the next ordinary sync would see a validity matching its own and
    // skip the reset for good.
    let batch = imap::RecentBatch {
        uidvalidity: 222,
        uid_next: 3,
        messages: vec![imap::MessageHeader {
            uid: 2,
            message_id: "fresh-1".to_string(),
            ..Default::default()
        }],
    };
    store_folder_tail(&conn, "acct", "Sent", &batch).unwrap();
    assert_eq!(cached_uids(&conn), vec![2]);
    assert_eq!(
        crate::store::get_folder_state(&conn, "acct", "Sent").unwrap(),
        Some((222, 3))
    );
    assert_eq!(
        crate::store::get_folder_modseq(&conn, "acct", "Sent").unwrap(),
        0
    );
}

fn cached_uids(conn: &Connection) -> Vec<u32> {
    let mut stmt = conn
        .prepare("SELECT uid FROM messages WHERE account = 'acct' AND folder = 'Sent' ORDER BY uid")
        .unwrap();
    stmt.query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .map(|uid| uid.unwrap() as u32)
        .collect()
}
