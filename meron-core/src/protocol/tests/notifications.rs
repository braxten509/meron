//! Exercises protocol behavior for notifications.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn mobile_new_messages_detail_summarizes_new_unread_inbox_mail() {
    let data_dir = unique_data_dir("new-messages-detail");
    seed_mobile_account(&data_dir, "me@example.com");
    let conn = store::open_at(data_dir.join("meron.db")).unwrap();
    store::ensure_folder(&conn, "me@example.com", "INBOX").unwrap();
    let messages = vec![
        MessageHeader {
            uid: 1,
            subject: "Old".to_string(),
            from_addr: "old@example.com".to_string(),
            date: 100,
            thread_key: "old".to_string(),
            ..Default::default()
        },
        MessageHeader {
            uid: 2,
            subject: "Already read".to_string(),
            from_addr: "read@example.com".to_string(),
            date: 200,
            seen: true,
            thread_key: "read".to_string(),
            ..Default::default()
        },
        MessageHeader {
            uid: 3,
            subject: "Fresh".to_string(),
            from_name: "Aki".to_string(),
            from_addr: "aki@example.com".to_string(),
            date: 300,
            thread_key: "fresh".to_string(),
            ..Default::default()
        },
    ];
    let arrivals =
        store::classify_inbox_arrivals(&conn, "me@example.com", 2, 4, &messages).unwrap();
    store::upsert_messages(&conn, "me@example.com", "INBOX", &messages).unwrap();
    drop(conn);
    let dir = data_dir.to_str().unwrap();

    let detail =
        crate::ffi::mobile_new_messages_detail(dir, "me@example.com", "INBOX", &arrivals).unwrap();
    assert_eq!(detail["account"], "me@example.com");
    assert_eq!(detail["accountName"], "me@example.com");
    assert_eq!(detail["folder"], "inbox");
    assert_eq!(detail["count"], 1);
    assert_eq!(detail["muted"], false);
    assert_eq!(detail["from"], "Aki");
    assert_eq!(detail["subject"], "Fresh");
    // The branch-aware card key (root#grouping-subject), so a notification tap
    // opens the exact card the thread-list grouping mints.
    assert_eq!(detail["threadKey"], "fresh#Fresh");

    // One entry per arrival, so the client can post a notification per mail.
    let listed = detail["messages"].as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["uid"], 3);
    assert_eq!(listed[0]["from"], "Aki");
    assert_eq!(listed[0]["subject"], "Fresh");
    assert_eq!(listed[0]["threadKey"], "fresh#Fresh");
    // No body cached yet, so the snippet is empty rather than the event waiting.
    assert_eq!(listed[0]["preview"], "");
    assert_eq!(detail["preview"], "");

    // First sync of a mailbox (no prior uid_next) must not summarize the backlog.
    assert!(
        crate::ffi::mobile_new_unread_inbox_messages(dir, "me@example.com", 0, 4, &[]).is_none()
    );
    // No uid growth means nothing new arrived.
    assert!(
        crate::ffi::mobile_new_unread_inbox_messages(dir, "me@example.com", 4, 4, &[]).is_none()
    );

    let _ = std::fs::remove_dir_all(data_dir);
}
