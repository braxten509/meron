//! Exercises protocol behavior for contacts.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn mobile_protocol_suggests_contacts_from_cached_messages() {
    let data_dir = unique_data_dir("contact-suggest");
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
                subject: "Hello".to_string(),
                from_name: "Aki".to_string(),
                from_addr: "aki@example.com".to_string(),
                date: 100,
                thread_key: "one".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 2,
                subject: "Again".to_string(),
                from_name: "Aki".to_string(),
                from_addr: "aki@example.com".to_string(),
                date: 200,
                thread_key: "two".to_string(),
                ..Default::default()
            },
            MessageHeader {
                uid: 3,
                subject: "Plan".to_string(),
                from_name: "Bea".to_string(),
                from_addr: "bea@example.com".to_string(),
                date: 300,
                thread_key: "three".to_string(),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    drop(conn);

    let suggestions = invoke_mobile_protocol_json(
        r#"{"id":93,"method":"mail.suggestContacts","params":{"account":"me@example.com","query":"","limit":2}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(
        suggestions["result"]["contacts"][0]["addr"], "aki@example.com",
        "{suggestions}"
    );
    assert_eq!(
        suggestions["result"]["contacts"][0]["name"], "Aki",
        "{suggestions}"
    );
    assert_eq!(
        suggestions["result"]["contacts"][1]["addr"], "bea@example.com",
        "{suggestions}"
    );

    let filtered = invoke_mobile_protocol_json(
        r#"{"id":94,"method":"contacts.suggest","params":{"account_id":"me@example.com","query":"bea","limit":8}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(filtered["result"]["contacts"].as_array().unwrap().len(), 1);
    assert_eq!(
        filtered["result"]["contacts"][0]["addr"], "bea@example.com",
        "{filtered}"
    );

    for (sort, first) in [("name", "aki@example.com"), ("recent", "bea@example.com")] {
        let directory = invoke_mobile_protocol_json(
            &json!({
                "id": 95, "method": "mail.suggestContacts",
                "params": {"query": "", "limit": 100, "sort": sort, "directory": true}
            })
            .to_string(),
            Some(data_dir.to_str().unwrap()),
        );
        assert_eq!(
            directory["result"]["contacts"][0]["addr"], first,
            "{directory}"
        );
    }

    let _ = std::fs::remove_dir_all(data_dir);
}
