//! Exercises protocol behavior for thread identity.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn requested_mobile_uids_uses_message_ids_without_thread_fallback() {
    let data_dir = unique_data_dir("targeted-uids");
    seed_mobile_account(&data_dir, "me@example.com");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 7,
                subject: "One".to_string(),
                thread_key: "topic".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 8,
                subject: "Two".to_string(),
                thread_key: "topic".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    let parsed = ParsedThreadId {
        account: "me@example.com".to_string(),
        folder: "INBOX".to_string(),
        thread_key: "topic".to_string(),
        uid: None,
        subject_filter: None,
    };

    let targeted = requested_mobile_uids(
        &conn,
        &parsed,
        &json!({ "message_ids": ["me@example.com#INBOX#t.dG9waWM#8"] }),
    )
    .unwrap();
    assert_eq!(targeted, vec![8]);

    let malformed =
        requested_mobile_uids(&conn, &parsed, &json!({ "message_ids": ["bad"] })).unwrap();
    assert!(malformed.is_empty());

    let fallback = requested_mobile_uids(&conn, &parsed, &json!({})).unwrap();
    assert_eq!(fallback, vec![7, 8]);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn parse_thread_id_round_trips_branch_root_containing_hash() {
    // '#' is legal atext in a Message-ID; the branch join must not truncate it.
    let compound = store::branch_compound_key("abc#123@host", "Topic");
    let id = format_thread_id("me@example.com", "INBOX", &compound);
    let parsed = parse_thread_id(&id).unwrap();
    assert_eq!(parsed.thread_key, "abc#123@host");
    assert_eq!(parsed.subject_filter.as_deref(), Some("Topic"));

    // Legacy unbranched ids keep resolving to the whole thread.
    let plain = parse_thread_id(&format_thread_id("me@example.com", "INBOX", "topic")).unwrap();
    assert_eq!(plain.thread_key, "topic");
    assert_eq!(plain.subject_filter, None);
}

#[test]
fn cached_thread_uids_uid_style_survives_thread_key_backfill() {
    let data_dir = unique_data_dir("uid-style-backfill");
    seed_mobile_account(&data_dir, "me@example.com");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    // The row was rendered as a uid-style card while its thread_key was empty,
    // then a later sync filled in the real key.
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[MessageHeader {
            uid: 9,
            subject: "One".to_string(),
            thread_key: "real-key@host".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();
    let parsed = parse_thread_id("me@example.com#INBOX#9").unwrap();
    assert_eq!(parsed.uid, Some(9));
    assert_eq!(cached_thread_uids(&conn, &parsed).unwrap(), vec![9]);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn update_mobile_read_state_scopes_branched_threads_to_their_uids() {
    let data_dir = unique_data_dir("branch-read-scope");
    seed_mobile_account(&data_dir, "me@example.com");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    store::upsert_messages(
        &conn,
        "me@example.com",
        "INBOX",
        &[
            MessageHeader {
                uid: 1,
                subject: "Topic".to_string(),
                thread_key: "root@host".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 2,
                subject: "New topic".to_string(),
                thread_key: "root@host".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    let parsed = ParsedThreadId {
        account: "me@example.com".to_string(),
        folder: "INBOX".to_string(),
        thread_key: "root@host".to_string(),
        uid: None,
        subject_filter: Some("New topic".to_string()),
    };
    let uids = cached_thread_uids(&conn, &parsed).unwrap();
    assert_eq!(uids, vec![2]);
    update_mobile_read_state(&conn, &parsed, &json!({}), &uids, true).unwrap();

    // Only the acted-on branch flips; the sibling branch keeps its unread state.
    let headers = store::get_thread_headers(&conn, "me@example.com", "INBOX", "root@host").unwrap();
    let seen_by_uid: std::collections::HashMap<u32, bool> = headers
        .iter()
        .map(|header| (header.uid, header.seen))
        .collect();
    assert!(seen_by_uid[&2]);
    assert!(!seen_by_uid[&1]);
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn parse_thread_id_with_request_folder_overrides_encoded_folder() {
    let parsed = parse_thread_id_with_request_folder(
        "me@example.com#INBOX#t.dG9waWM",
        &json!({ "folder": "Trash" }),
    )
    .unwrap();

    assert_eq!(parsed.account, "me@example.com");
    assert_eq!(parsed.folder, "Trash");
    assert_eq!(parsed.thread_key, "topic");
}
