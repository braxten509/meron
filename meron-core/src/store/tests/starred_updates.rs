//! Star and unstar mutations across message copies.
//! Fixtures distinguish folder-local identities from provider-wide identities.
//! An update should touch only the selected message and its copies.

use super::*;

#[test]
fn starred_mutation_clears_all_folder_copies_and_can_restore_them() {
    let conn = test_conn();
    for (account, folder, uid, subject) in [
        ("acct", "INBOX", 1, "Release"),
        ("acct", "All Mail", 42, "Re: Release"),
        ("acct", "All Mail", 43, "Other branch"),
        ("other", "INBOX", 1, "Release"),
    ] {
        upsert_messages(
            &conn,
            account,
            folder,
            &[MessageHeader {
                uid,
                subject: subject.into(),
                thread_key: "root@example.com".into(),
                starred: true,
                ..Default::default()
            }],
        )
        .unwrap();
    }
    let key = branch_compound_key("root@example.com", &thread_grouping_subject("Release"));
    let targets = starred_mutation_targets(&conn, "acct", "INBOX", Some(&key), &[], false).unwrap();
    assert_eq!(
        targets,
        BTreeMap::from([("INBOX".into(), vec![1]), ("All Mail".into(), vec![42])])
    );
    for starred in [false, true] {
        for (folder, uids) in &targets {
            for uid in uids {
                update_message_starred(&conn, "acct", folder, *uid, starred).unwrap();
            }
        }
        let cards = crate::mail_model::starred_thread_cards(&conn, 100).unwrap();
        assert_eq!(
            cards
                .iter()
                .filter(|card| card["account_id"] == "acct")
                .count(),
            if starred { 2 } else { 1 }
        );
        assert_eq!(
            cards
                .iter()
                .filter(|card| card["account_id"] == "other")
                .count(),
            1
        );
    }
}

#[test]
fn starred_mutation_keeps_folder_local_thread_keys_scoped() {
    let conn = test_conn();
    for folder in ["INBOX", "Archive"] {
        upsert_messages(
            &conn,
            "acct",
            folder,
            &[MessageHeader {
                uid: 1,
                starred: true,
                ..Default::default()
            }],
        )
        .unwrap();
    }
    assert_eq!(
        starred_mutation_targets(&conn, "acct", "INBOX", Some("uid:1"), &[], false).unwrap(),
        BTreeMap::from([("INBOX".into(), vec![1])])
    );
    assert_eq!(
        starred_mutation_targets(&conn, "acct", "INBOX", None, &[1], false).unwrap(),
        BTreeMap::from([("INBOX".into(), vec![1])])
    );
}

#[test]
fn starred_mutation_of_a_message_follows_only_its_copies() {
    let conn = test_conn();
    for (account, folder, uid, message_id) in [
        ("acct", "INBOX", 1, "<ONE@example.com>"),
        ("acct", "Archive", 99, "one@example.com"),
        ("acct", "Archive", 1, "two@example.com"),
        ("other", "INBOX", 1, "one@example.com"),
    ] {
        upsert_messages(
            &conn,
            account,
            folder,
            &[MessageHeader {
                uid,
                message_id: message_id.into(),
                thread_key: "root@example.com".into(),
                starred: true,
                ..Default::default()
            }],
        )
        .unwrap();
    }
    assert_eq!(
        starred_mutation_targets(&conn, "acct", "INBOX", None, &[1], false).unwrap(),
        BTreeMap::from([("INBOX".into(), vec![1]), ("Archive".into(), vec![99])])
    );
    assert!(
        starred_mutation_targets(&conn, "acct", "INBOX", None, &[], false)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn starred_mutation_follows_gmail_identity_without_message_id() {
    let conn = test_conn();
    for (folder, uid, gmail_msg_id) in [
        ("INBOX", 1, 123),
        ("All Mail", 9, 123),
        ("All Mail", 1, 456),
    ] {
        upsert_messages(
            &conn,
            "acct",
            folder,
            &[MessageHeader {
                uid,
                gmail_msg_id: Some(gmail_msg_id),
                starred: true,
                ..Default::default()
            }],
        )
        .unwrap();
    }
    assert_eq!(
        starred_mutation_targets(&conn, "acct", "INBOX", None, &[1], false).unwrap(),
        BTreeMap::from([("INBOX".into(), vec![1]), ("All Mail".into(), vec![9])])
    );
}

#[test]
fn starred_mutation_skips_matching_flags_and_stars_only_source_messages_and_copies() {
    let conn = test_conn();
    for (folder, uid, mid, starred) in [
        ("INBOX", 1, "one@example.com", false),
        ("INBOX", 2, "two@example.com", true),
        ("All Mail", 10, "one@example.com", true),
        ("All Mail", 20, "two@example.com", false),
        ("Sent", 30, "reply@example.com", false),
        ("Trash", 40, "old@example.com", true),
    ] {
        upsert_messages(
            &conn,
            "acct",
            folder,
            &[MessageHeader {
                uid,
                subject: "Release".into(),
                message_id: mid.into(),
                thread_key: "root@example.com".into(),
                starred,
                ..Default::default()
            }],
        )
        .unwrap();
    }
    assert_eq!(
        starred_mutation_targets(&conn, "acct", "INBOX", Some("root@example.com"), &[], true)
            .unwrap(),
        BTreeMap::from([("INBOX".into(), vec![1]), ("All Mail".into(), vec![20])])
    );
    // A multi-message action finds all copies in one pass, including a copy
    // whose source already has the desired flag.
    assert_eq!(
        starred_mutation_targets(&conn, "acct", "INBOX", None, &[1, 2], true).unwrap(),
        BTreeMap::from([("INBOX".into(), vec![1]), ("All Mail".into(), vec![20])])
    );
    assert_eq!(
        starred_mutation_targets(&conn, "acct", "INBOX", Some("root@example.com"), &[], false)
            .unwrap(),
        BTreeMap::from([
            ("INBOX".into(), vec![2]),
            ("All Mail".into(), vec![10]),
            ("Trash".into(), vec![40])
        ])
    );
    assert!(
        starred_mutation_targets(&conn, "acct", "Sent", None, &[30], false)
            .unwrap()
            .is_empty()
    );
}
