//! New-arrival classification and folder notification preferences.
//! The fixtures revisit mail after archive, restore and folder sync.
//! Repeated observations must keep distinct arrivals without duplicating alerts.

use super::*;

#[test]
fn cached_archive_identity_suppresses_inbox_arrival_and_survives_deletion() {
    let conn = test_conn();
    let mut message = MessageHeader {
        uid: 5,
        message_id: "known@example.com".into(),
        ..Default::default()
    };
    upsert_messages(&conn, "acct", "Archive", &[message.clone()]).unwrap();
    delete_messages_by_uid(&conn, "acct", "Archive", &[5]).unwrap();
    message.uid = 20;
    assert!(
        classify_inbox_arrivals(&conn, "acct", 20, 21, &[message])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn folder_notify_opt_in_survives_folder_list_sync() {
    let conn = test_conn();
    let folder = Folder {
        name: "Work".into(),
        display_name: "Work".into(),
        ..Default::default()
    };
    upsert_folders(&conn, "acct", std::slice::from_ref(&folder)).unwrap();
    ensure_folder(&conn, "acct", "INBOX").unwrap();
    assert!(folder_notifies(&conn, "acct", "inbox").unwrap());
    assert!(!folder_notifies(&conn, "acct", "Work").unwrap());
    assert!(!set_folder_notify(&conn, "acct", "Missing", true).unwrap());

    assert!(set_folder_notify(&conn, "acct", "Work", true).unwrap());
    // A later LIST sync rewrites the row's server-owned columns only.
    upsert_folders(&conn, "acct", &[folder]).unwrap();
    assert!(folder_notifies(&conn, "acct", "Work").unwrap());
    assert_eq!(notify_folders(&conn, "acct").unwrap(), vec!["Work"]);
    let listed = get_folders(&conn, "acct").unwrap();
    assert!(listed.iter().any(|f| f.name == "Work" && f.notify));
    assert!(listed.iter().any(|f| f.name == "INBOX" && !f.notify));

    assert!(set_folder_notify(&conn, "acct", "Work", false).unwrap());
    assert!(notify_folders(&conn, "acct").unwrap().is_empty());
}

#[test]
fn notify_plan_gives_idle_slots_to_the_earliest_opt_ins() {
    let conn = test_conn();
    // Opted in out of name order: slots follow the opt-in order, not the name.
    let names = ["Zeta", "Beta", "Mid", "Alpha", "Omega"];
    for name in names {
        ensure_folder(&conn, "acct", name).unwrap();
        assert!(set_folder_notify(&conn, "acct", name, true).unwrap());
    }
    let plan = notify_folder_plan(&conn, "acct").unwrap();
    assert_eq!(plan.live, vec!["Zeta", "Beta", "Mid"]);
    assert_eq!(plan.polled, vec!["Alpha", "Omega"]);
    assert!(folder_notifies_live(&conn, "acct", "Mid").unwrap());
    assert!(!folder_notifies_live(&conn, "acct", "Alpha").unwrap());

    // Enabling again keeps a folder's place; a later opt-in never demotes it.
    assert!(set_folder_notify(&conn, "acct", "Zeta", true).unwrap());
    assert_eq!(notify_folder_plan(&conn, "acct").unwrap(), plan);

    // Freeing a slot promotes the longest-waiting polled folder.
    assert!(set_folder_notify(&conn, "acct", "Beta", false).unwrap());
    let plan = notify_folder_plan(&conn, "acct").unwrap();
    assert_eq!(plan.live, vec!["Zeta", "Mid", "Alpha"]);
    assert_eq!(plan.polled, vec!["Omega"]);
    assert!(
        get_folders(&conn, "acct")
            .unwrap()
            .iter()
            .all(|f| f.notify == (f.name != "Beta"))
    );
}

#[test]
fn custom_folder_arrivals_skip_mail_already_seen_elsewhere() {
    let conn = test_conn();
    let filed = MessageHeader {
        uid: 7,
        message_id: "filed@example.com".into(),
        ..Default::default()
    };
    upsert_messages(&conn, "acct", "INBOX", &[filed.clone()]).unwrap();
    // The user moved `filed` out of INBOX; a server rule delivered `direct`.
    let batch = [
        MessageHeader { uid: 20, ..filed },
        MessageHeader {
            uid: 21,
            message_id: "direct@example.com".into(),
            ..Default::default()
        },
    ];
    let arrivals = classify_arrivals(&conn, "acct", "Work", 20, 22, &batch).unwrap();
    assert_eq!(
        arrivals
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>(),
        vec![21]
    );
}

#[test]
fn classified_arrivals_survive_companion_writes_and_later_inbox_sync() {
    let conn = test_conn();
    let arrival = MessageHeader {
        uid: 20,
        message_id: "arrival@example.com".into(),
        thread_key: "arrival@example.com".into(),
        ..Default::default()
    };
    let notified = classify_inbox_arrivals(&conn, "acct", 20, 21, &[arrival.clone()]).unwrap();
    upsert_messages(&conn, "acct", "INBOX", &[arrival.clone()]).unwrap();
    for folder in ["Sent", "Drafts"] {
        upsert_messages(
            &conn,
            "acct",
            folder,
            &[MessageHeader {
                uid: 1,
                message_id: format!("old-{folder}@example.com"),
                ..Default::default()
            }],
        )
        .unwrap();
    }
    assert!(
        classify_inbox_arrivals(&conn, "acct", 20, 21, &[arrival])
            .unwrap()
            .is_empty()
    );
    assert_eq!(notified.len(), 1);
    assert_eq!(notified[0].uid, 20);
}

#[test]
fn new_unread_inbox_messages_ignores_empty_or_non_growing_windows() {
    let conn = test_conn();
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[MessageHeader {
            uid: 1,
            subject: "Unread".to_string(),
            from_addr: "new@example.com".to_string(),
            thread_key: "new".to_string(),
            ..Default::default()
        }],
    )
    .unwrap();

    assert!(
        new_unread_inbox_messages(&conn, "acct", 0, 2, &[])
            .unwrap()
            .is_none()
    );
    assert!(
        new_unread_inbox_messages(&conn, "acct", 2, 2, &[])
            .unwrap()
            .is_none()
    );
    assert!(
        new_unread_inbox_messages(&conn, "acct", 3, 2, &[])
            .unwrap()
            .is_none()
    );
}

#[test]
fn new_unread_inbox_messages_ignores_observed_gmail_message_restored_with_new_uid() {
    let conn = test_conn();
    let old = vec![MessageHeader {
        uid: 10,
        subject: "Old unread".to_string(),
        from_addr: "old@example.com".to_string(),
        thread_key: "gmthrid:1".to_string(),
        gmail_msg_id: Some(999),
        ..Default::default()
    }];
    upsert_messages(&conn, "acct", "INBOX", &old).unwrap();
    delete_messages_by_uid(&conn, "acct", "INBOX", &[10]).unwrap();

    let restored = vec![MessageHeader {
        uid: 20,
        subject: "Old unread".to_string(),
        from_addr: "old@example.com".to_string(),
        thread_key: "gmthrid:1".to_string(),
        gmail_msg_id: Some(999),
        ..Default::default()
    }];
    assert!(
        classify_inbox_arrivals(&conn, "acct", 20, 21, &restored)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn new_unread_inbox_messages_ignores_observed_message_id_restored_with_new_uid() {
    let conn = test_conn();
    let old = vec![MessageHeader {
        uid: 10,
        subject: "Old unread".to_string(),
        from_addr: "old@example.com".to_string(),
        thread_key: "mid@example.com".to_string(),
        message_id: "Mid@Example.com".to_string(),
        ..Default::default()
    }];
    upsert_messages(&conn, "acct", "INBOX", &old).unwrap();
    delete_messages_by_uid(&conn, "acct", "INBOX", &[10]).unwrap();

    let restored = vec![MessageHeader {
        uid: 20,
        subject: "Old unread".to_string(),
        from_addr: "old@example.com".to_string(),
        thread_key: "mid@example.com".to_string(),
        message_id: "mid@example.com".to_string(),
        ..Default::default()
    }];
    assert!(
        classify_inbox_arrivals(&conn, "acct", 20, 21, &restored)
            .unwrap()
            .is_empty()
    );
}
