//! Regression tests for archive.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;

fn archive() -> (Connection, std::path::PathBuf) {
    let directory =
        std::env::temp_dir().join(format!("meron-archive-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let conn = super::super::open_archive_at(directory.join("archive.db"), None).unwrap();
    (conn, directory)
}

const RAW: &[u8] = b"Message-ID: <a@example.com>\r\nFrom: Ann <ann@example.com>\r\nTo: bob@example.com\r\nSubject: Hello\r\nDate: Tue, 1 Oct 2024 10:00:00 +0000\r\n\r\nBody text\r\n";

fn input(raw: &[u8]) -> ArchiveInput<'_> {
    ArchiveInput {
        raw,
        seen: true,
        starred: false,
        thread_key: String::new(),
        date_fallback: 0,
        source: "test".into(),
    }
}

#[test]
fn archived_message_reads_back_and_lists() {
    let (conn, directory) = archive();
    create_local_folder(&conn, "Old/2024").unwrap();
    let uids = archive_messages(&conn, "Old/2024", &[input(RAW)], &directory).unwrap();
    assert_eq!(uids, vec![1]);
    assert_eq!(raw_message(&conn, "Old/2024", 1).unwrap().unwrap(), RAW);
    // The parent was created alongside.
    let folders = crate::store::get_folders(&conn, LOCAL_ACCOUNT_ID).unwrap();
    assert_eq!(
        folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
        vec!["Old", "Old/2024"]
    );
    assert!(folders.iter().all(|f| f.role == "folder"));
    let cached = crate::store::get_cached_message(&conn, LOCAL_ACCOUNT_ID, "Old/2024", 1)
        .unwrap()
        .unwrap();
    assert_eq!(cached.subject, "Hello");
    assert_eq!(cached.message_id, "a@example.com");
    // The same bytes again are not stored twice.
    let again = archive_messages(&conn, "Old/2024", &[input(RAW)], &directory).unwrap();
    assert_eq!(again, vec![1]);
    let stats = local_storage_stats(&conn).unwrap();
    assert_eq!(stats["messages"], 1);
}

#[test]
fn move_rekeys_and_keeps_flags() {
    let (conn, directory) = archive();
    create_local_folder(&conn, "A").unwrap();
    create_local_folder(&conn, "B").unwrap();
    archive_messages(&conn, "A", &[input(RAW)], &directory).unwrap();
    set_local_flags(&conn, "A", &[1], Some(false), Some(true)).unwrap();
    let moved = move_local_messages(&conn, "A", "B", &[1], &directory).unwrap();
    assert_eq!(moved, vec![1]);
    assert!(raw_message(&conn, "A", 1).unwrap().is_none());
    let copies = raw_messages_for_copy(&conn, "B", &moved).unwrap();
    assert_eq!(copies.len(), 1);
    assert!(!copies[0].seen && copies[0].starred);
    let (seen, starred): (i64, i64) = conn
        .query_row(
            "SELECT seen, starred FROM messages WHERE account = 'local' AND folder = 'B' AND uid = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((seen, starred), (0, 1));
    let gone: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE account = 'local' AND folder = 'A'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(gone, 0);
}

#[test]
fn restoring_media_repairs_an_attachment_left_without_a_file() {
    let (conn, directory) = archive();
    let created = create_local_folder(&conn, "Archive").unwrap();
    assert_eq!(created.role, "folder");
    let raw = b"Message-ID: <b@example.com>\r\nFrom: ann@example.com\r\nSubject: File\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nHi\r\n--b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"a.txt\"\r\n\r\nfile\r\n--b--\r\n";
    archive_messages(&conn, "Archive", &[input(raw)], &directory).unwrap();
    // As a failed cache write leaves it: an attachment with no file key.
    conn.execute(
        "UPDATE messages SET json = json_set(json, '$.attachments[0].key', NULL) WHERE account = 'local'",
        [],
    )
    .unwrap();
    let missing = |conn: &Connection| {
        let cached = crate::store::get_cached_message(conn, LOCAL_ACCOUNT_ID, "Archive", 1)
            .unwrap()
            .unwrap();
        crate::parse::missing_media_count(&directory, &cached)
    };
    assert_eq!(missing(&conn), 1);
    restore_local_media(&conn, &[("Archive".to_string(), 1)], &directory).unwrap();
    assert_eq!(missing(&conn), 0);
}

#[test]
fn deleted_folder_takes_its_subtree_and_mail() {
    let (conn, directory) = archive();
    create_local_folder(&conn, "A/B").unwrap();
    archive_messages(&conn, "A/B", &[input(RAW)], &directory).unwrap();
    let (removed, deleted) = delete_local_folder(&conn, "A").unwrap();
    assert_eq!(removed, vec!["A/B".to_string(), "A".to_string()]);
    assert_eq!(deleted, 1);
    assert_eq!(local_storage_stats(&conn).unwrap()["messages"], 0);
}

#[test]
fn renamed_folder_takes_its_subtree_and_mail() {
    let (conn, directory) = archive();
    create_local_folder(&conn, "A/B").unwrap();
    create_local_folder(&conn, "C").unwrap();
    archive_messages(&conn, "A/B", &[input(RAW)], &directory).unwrap();
    set_local_flags(&conn, "A/B", &[1], None, Some(true)).unwrap();
    let renamed = rename_local_folder(&conn, "A", "X/Y", &directory).unwrap();
    assert_eq!(
        renamed,
        vec![
            ("A/B".to_string(), "X/Y/B".to_string()),
            ("A".to_string(), "X/Y".to_string()),
        ]
    );
    let names = crate::store::get_folders(&conn, LOCAL_ACCOUNT_ID)
        .unwrap()
        .into_iter()
        .map(|folder| folder.name)
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["C", "X", "X/Y", "X/Y/B"]);
    assert_eq!(raw_message(&conn, "X/Y/B", 1).unwrap().unwrap(), RAW);
    let cached = crate::store::get_cached_message(&conn, LOCAL_ACCOUNT_ID, "X/Y/B", 1)
        .unwrap()
        .unwrap();
    assert_eq!(cached.subject, "Hello");
    let starred: i64 = conn
        .query_row(
            "SELECT starred FROM messages WHERE account = 'local' AND folder = 'X/Y/B' AND uid = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(starred, 1);
    // The UID counter came along: the next message does not reuse UID 1.
    let next = archive_messages(
        &conn,
        "X/Y/B",
        &[input(b"Message-ID: <c@example.com>\r\n\r\nHi\r\n")],
        &directory,
    )
    .unwrap();
    assert_eq!(next, vec![2]);

    assert!(
        rename_local_folder(&conn, "C", "X/Y", &directory)
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert!(
        rename_local_folder(&conn, "X", "X/Y/Z", &directory)
            .unwrap_err()
            .to_string()
            .contains("inside itself")
    );
    assert!(
        rename_local_folder(&conn, "C", "C", &directory)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn index_is_rebuilt_from_the_kept_original() {
    let (conn, directory) = archive();
    create_local_folder(&conn, "A").unwrap();
    archive_messages(&conn, "A", &[input(RAW)], &directory).unwrap();
    conn.execute("DELETE FROM messages", []).unwrap();
    assert_eq!(repair_local_index(&conn, &directory).unwrap(), 1);
    assert_eq!(repair_local_index(&conn, &directory).unwrap(), 0);
}

#[test]
fn folder_names_are_normalized() {
    assert_eq!(normalize_local_folder_name(" A / B ").unwrap(), "A/B");
    assert!(normalize_local_folder_name("A//B").is_err());
    assert!(normalize_local_folder_name("  ").is_err());
    assert_eq!(normalize_local_folder_name("inbox").unwrap(), "INBOX");
    assert_eq!(
        normalize_local_folder_name("inbox/2024").unwrap(),
        "INBOX/2024"
    );
}

#[test]
fn a_parent_named_inbox_is_addressable_with_its_children() {
    let (conn, _directory) = archive();
    create_local_folder(&conn, "inbox/2024").unwrap();
    let (removed, _) = delete_local_folder(&conn, "INBOX").unwrap();
    assert_eq!(removed, vec!["INBOX/2024".to_string(), "INBOX".to_string()]);
}

#[test]
fn a_snapshot_never_replaces_the_open_archive() {
    let (conn, directory) = archive();
    create_local_folder(&conn, "A").unwrap();
    for name in [
        "archive.db",
        "archive.db-wal",
        "archive.db-shm",
        "./archive.db",
    ] {
        assert!(crate::store::backup_archive_to(&conn, &directory.join(name), "pass").is_err());
    }
    assert!(super::super::folder_exists(&conn, LOCAL_ACCOUNT_ID, "A").unwrap());
}

#[test]
fn a_backup_restores_into_another_archive_with_its_passphrase() {
    const KEY: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    let directory =
        std::env::temp_dir().join(format!("meron-archive-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let conn = crate::store::open_archive_at(directory.join("archive.db"), Some(KEY)).unwrap();
    create_local_folder(&conn, "Old/2024").unwrap();
    archive_messages(&conn, "Old/2024", &[input(RAW)], &directory).unwrap();
    let stats = local_storage_stats(&conn).unwrap();
    assert_eq!(stats["messages"], 1);
    assert!(stats["file_bytes"].as_i64().unwrap() > 0);
    let tag_key = ["kept@example.com".to_string()];
    crate::store::change_tags(
        &conn,
        LOCAL_ACCOUNT_ID,
        &tag_key,
        &["todo".into()],
        &[],
        false,
    )
    .unwrap();
    let dest = directory.join("copy.db");
    crate::store::backup_archive_to(&conn, &dest, "correct horse").unwrap();

    // Another computer: another key, an archive of its own.
    let other = crate::store::open_archive_at(directory.join("other.db"), None).unwrap();
    let wrong = restore_local_backup(&other, &dest, "wrong", &directory).unwrap_err();
    assert!(format!("{wrong:#}").contains(WRONG_BACKUP_PASSPHRASE));
    assert_eq!(
        restore_local_backup(&other, &dest, "correct horse", &directory).unwrap(),
        1
    );
    assert_eq!(raw_message(&other, "Old/2024", 1).unwrap().unwrap(), RAW);
    assert_eq!(
        crate::store::tags_by_key(&other, LOCAL_ACCOUNT_ID, &tag_key).unwrap()[&tag_key[0]],
        ["todo"]
    );
    assert!(super::super::folder_exists(&other, LOCAL_ACCOUNT_ID, "Old").unwrap());
    // Restoring again adds nothing twice.
    assert_eq!(
        restore_local_backup(&other, &dest, "correct horse", &directory).unwrap(),
        0
    );
}

#[test]
fn a_source_uid_thread_key_is_rekeyed_to_the_local_uid() {
    let (conn, directory) = archive();
    create_local_folder(&conn, "inbox").unwrap();
    let headerless = b"From: ann@example.com\r\nSubject: Note\r\n\r\nBody\r\n";
    let mut message = input(headerless);
    message.thread_key = "uid:77".into();
    let uids = archive_messages(&conn, "INBOX", &[message], &directory).unwrap();
    let key: String = conn
        .query_row(
            "SELECT thread_key FROM messages WHERE account = 'local' AND folder = 'INBOX' AND uid = ?1",
            [uids[0]],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(key, "uid:77");
}

#[test]
fn folders_are_addressed_by_their_wire_name() {
    let (conn, _directory) = archive();
    let folder = create_local_folder(&conn, "Steuer & Bü/2024").unwrap();
    assert_eq!(folder.display_name, "Steuer & Bü/2024");
    assert_eq!(folder.name, crate::utf7::encode("Steuer & Bü/2024"));
    let folders = crate::store::get_folders(&conn, LOCAL_ACCOUNT_ID).unwrap();
    assert_eq!(folders[1].display_name, "Steuer & Bü/2024");
    assert_eq!(folders[0].display_name, "Steuer & Bü");
}
