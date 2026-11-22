//! Exercises store behavior for sync snapshots.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn a_sync_snapshot_from_before_a_move_cannot_put_the_message_back() {
    let conn = test_conn();
    let message = MessageHeader {
        uid: 7,
        subject: "Archive me".into(),
        from_addr: "a@example.com".into(),
        thread_key: "topic".into(),
        ..Default::default()
    };
    upsert_messages(&conn, "acct", "INBOX", &[message.clone()]).unwrap();
    // What a folder sync reads before it goes to the network.
    let fetched_at = removed_message_epoch(&conn).unwrap();
    delete_messages_by_uid(&conn, "acct", "INBOX", &[7]).unwrap();

    // The snapshot still contains the message, and it started before the move.
    release_removed_messages(&conn, "acct", "INBOX", &[7], fetched_at).unwrap();
    upsert_messages(&conn, "acct", "INBOX", &[message.clone()]).unwrap();
    assert!(!has_message(&conn, "acct", "INBOX", 7).unwrap());

    // A later sync really does see the UID in the folder again (moved back).
    let later = removed_message_epoch(&conn).unwrap();
    release_removed_messages(&conn, "acct", "INBOX", &[7], later).unwrap();
    upsert_messages(&conn, "acct", "INBOX", &[message]).unwrap();
    assert!(has_message(&conn, "acct", "INBOX", 7).unwrap());
}

#[test]
fn a_late_body_fetch_cannot_reinsert_a_removed_message() {
    let conn = test_conn();
    let header = MessageHeader {
        uid: 7,
        subject: "Archive me".into(),
        ..Default::default()
    };
    let body = Message {
        subject: "Archive me".into(),
        body: "Late body".into(),
        ..Default::default()
    };
    upsert_messages(&conn, "acct", "INBOX", &[header.clone()]).unwrap();
    let (body_fetch, _) = begin_message_sync(&conn).unwrap();
    delete_messages_by_uid(&conn, "acct", "INBOX", &[7]).unwrap();
    // An unrelated fresh sync must not clean up the marker while the body fetch
    // still owns an older snapshot.
    let (other_sync, _) = begin_message_sync(&conn).unwrap();
    end_message_sync(&conn, other_sync).unwrap();
    // fill_thread_gaps writes the header and the body in succession.
    upsert_messages(&conn, "acct", "INBOX", &[header.clone()]).unwrap();
    save_cached_message(&conn, "acct", "INBOX", 7, &body).unwrap();
    assert!(!has_message(&conn, "acct", "INBOX", 7).unwrap());
    assert!(
        get_cached_message(&conn, "acct", "INBOX", 7)
            .unwrap()
            .is_none()
    );
    end_message_sync(&conn, body_fetch).unwrap();
    // A later valid fetch can still cache a body normally.
    upsert_messages(&conn, "acct", "INBOX", &[header]).unwrap();
    save_cached_message(&conn, "acct", "INBOX", 7, &body).unwrap();
    assert_eq!(
        get_cached_message(&conn, "acct", "INBOX", 7)
            .unwrap()
            .unwrap()
            .body,
        "Late body"
    );
}

#[test]
fn removed_uid_markers_expire_after_the_oldest_sync_finishes() {
    let conn = test_conn();
    let count = || -> i64 {
        conn.query_row("SELECT count(*) FROM removed_message_uids", [], |r| {
            r.get(0)
        })
        .unwrap()
    };
    let (first, _) = begin_message_sync(&conn).unwrap();
    let (same_epoch, _) = begin_message_sync(&conn).unwrap();
    delete_messages_by_uid(&conn, "acct", "INBOX", &[7]).unwrap();
    let (later, _) = begin_message_sync(&conn).unwrap();
    assert_eq!(count(), 1);
    end_message_sync(&conn, first).unwrap();
    assert_eq!(count(), 1, "another old snapshot still needs the marker");
    end_message_sync(&conn, same_epoch).unwrap();
    assert_eq!(count(), 0, "the newer sync cannot contain the old UID");
    delete_messages_by_uid(&conn, "acct", "INBOX", &[8]).unwrap();
    assert_eq!(count(), 1);
    end_message_sync(&conn, later).unwrap();
    assert_eq!(count(), 0, "no active snapshots remain");
}

#[test]
fn another_connection_cannot_prune_markers_an_active_sync_needs() {
    let dir = std::env::temp_dir().join(format!("meron-store-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("meron.db");
    let syncing = db::open_at(&path).unwrap();
    let archiving = db::open_at(&path).unwrap();
    let count = || -> i64 {
        syncing
            .query_row("SELECT count(*) FROM removed_message_uids", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    let (stale, _) = begin_message_sync(&syncing).unwrap();
    delete_messages_by_uid(&archiving, "acct", "INBOX", &[7]).unwrap();
    let (other, _) = begin_message_sync(&archiving).unwrap();
    end_message_sync(&archiving, other).unwrap();
    assert_eq!(count(), 1, "the other connection's snapshot is still open");
    end_message_sync(&syncing, stale).unwrap();
    assert_eq!(count(), 0);

    // A snapshot left behind by a dead process does not pin markers.
    archiving
        .execute(
            "INSERT INTO active_message_syncs(epoch, owner) VALUES(0, 'dead')",
            [],
        )
        .unwrap();
    delete_messages_by_uid(&archiving, "acct", "INBOX", &[8]).unwrap();
    let (id, _) = begin_message_sync(&syncing).unwrap();
    assert_eq!(count(), 0);
    end_message_sync(&syncing, id).unwrap();

    drop(syncing);
    drop(archiving);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn concurrent_sync_registration_waits_instead_of_failing() {
    let dir = std::env::temp_dir().join(format!("meron-store-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("meron.db");
    db::open_at(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
    let handles = (0..4u32)
        .map(|worker| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let conn = db::open_at(&path).unwrap();
                barrier.wait();
                for round in 0..50 {
                    let (id, _) = begin_message_sync(&conn).unwrap();
                    delete_messages_by_uid(&conn, "acct", "INBOX", &[worker * 100 + round + 1])
                        .unwrap();
                    end_message_sync(&conn, id).unwrap();
                }
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().unwrap();
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_new_sync_purges_removal_markers_left_without_active_snapshots() {
    let conn = test_conn();
    delete_messages_by_uid(&conn, "acct", "INBOX", &[7, 8, 9]).unwrap();
    let (id, epoch) = begin_message_sync(&conn).unwrap();
    assert!(epoch > 0);
    let count: i64 = conn
        .query_row("SELECT count(*) FROM removed_message_uids", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    end_message_sync(&conn, id).unwrap();
}
