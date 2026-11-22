//! Credentials used by send, sync and account discovery.
//! Cases build an engine before and after saving an account.
//! Platform-managed tokens have a separate refresh lifecycle.

use super::*;

#[test]
fn mobile_protocol_send_requires_usable_account_secret() {
    let data_directory = unique_data_directory("send-needs-secret");
    seed_mobile_account(&data_directory, "me@example.com");

    let value = invoke_mobile_protocol_json(
        r#"{"id":83,"method":"mail.send","params":{"account_id":"me@example.com","to":"you@example.com","subject":"Hi","body":"Hello"}}"#,
        Some(data_directory.to_str().unwrap()),
    );

    assert_eq!(value["id"], 83);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );
    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_sync_requires_usable_account_secret() {
    let data_directory = unique_data_directory("sync-needs-secret");
    seed_mobile_account(&data_directory, "me@example.com");

    let value = invoke_mobile_protocol_json(
        r#"{"id":84,"method":"mail.sync","params":{"account_id":"me@example.com","folder_id":"inbox","limit":10}}"#,
        Some(data_directory.to_str().unwrap()),
    );

    assert_eq!(value["id"], 84);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let desktop_name = invoke_mobile_protocol_json(
        r#"{"id":85,"method":"messages.sync","params":{"account":"me@example.com","folder":"INBOX","limit":10}}"#,
        Some(data_directory.to_str().unwrap()),
    );
    assert_eq!(desktop_name["id"], 85);
    assert!(
        desktop_name["error"]["message"]
            .as_str()
            .unwrap()
            .contains("account needs reconnect")
    );

    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_autodiscover_returns_known_provider() {
    let gmail = invoke_mobile_protocol_json(
        r#"{"id":96,"method":"account.autodiscover","params":{"email":"me@gmail.com"}}"#,
        None,
    );
    assert_eq!(gmail["id"], 96);
    assert_eq!(gmail["result"]["imap_host"], "imap.gmail.com");
    assert_eq!(gmail["result"]["smtp_host"], "smtp.gmail.com");
    assert_eq!(gmail["result"]["username"], "me@gmail.com");
    assert_eq!(gmail["result"]["source"], "known");
}

#[test]
fn engine_loads_password_account_with_secret_from_store() {
    let data_directory = unique_data_directory("engine-load");
    let directory_text = data_directory.to_str().unwrap();
    let email = format!("eng+{}@example.com", unique_test_suffix());
    add_password_account(directory_text, &email);

    // MobileHost opens the same store and applies the secret onto the creds.
    let engine = crate::engine::Engine::new(Box::new(MobileHost {
        data_dir: directory_text.to_string(),
    }))
    .expect("engine builds");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let creds = rt
        .block_on(engine.ensure_valid_creds(&email))
        .expect("account loaded with secret");
    assert_eq!(creds.user, email);
    assert_eq!(creds.password, "secret");
}

#[test]
fn engine_hydrates_account_added_after_construction() {
    let data_directory = unique_data_directory("engine-hydrate");
    let directory_text = data_directory.to_str().unwrap();
    // Build the Engine before any account exists, mirroring a long-lived
    // foreground Engine that outlives an out-of-band account add.
    let engine = crate::engine::Engine::new(Box::new(MobileHost {
        data_dir: directory_text.to_string(),
    }))
    .expect("engine builds");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let email = format!("late+{}@example.com", unique_test_suffix());
    assert!(
        rt.block_on(engine.ensure_valid_creds(&email)).is_err(),
        "unknown account should error before it exists"
    );

    add_password_account(directory_text, &email);

    let creds = rt
        .block_on(engine.ensure_valid_creds(&email))
        .expect("account hydrated from store on next use");
    assert_eq!(creds.password, "secret");
}

#[test]
fn engine_does_not_refresh_platform_managed_oauth_without_refresh_token() {
    let data_directory = unique_data_directory("engine-managed-oauth");
    let directory_text = data_directory.to_str().unwrap();
    let email = format!("managed+{}@gmail.com", unique_test_suffix());
    let add = format!(
        r#"{{"id":1,"method":"account.addOAuth","params":{{"email":"{email}","provider":"gmail","display_name":"Gmail","sender_name":"Sender","access_token":"access-only","token_expires_at":1}}}}"#
    );
    let added = invoke_mobile_protocol_json(&add, Some(directory_text));
    assert_eq!(
        added["result"]["account"]["needs_reconnect"], false,
        "{added}"
    );

    let engine = crate::engine::Engine::new(Box::new(MobileHost {
        data_dir: directory_text.to_string(),
    }))
    .expect("engine builds");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let creds = rt
        .block_on(engine.ensure_valid_creds(&email))
        .expect("access-token-only OAuth account remains usable for platform refresh");
    assert_eq!(creds.access_token.as_deref(), Some("access-only"));
    assert_eq!(creds.refresh_token.as_deref(), None);

    let _ = std::fs::remove_dir_all(data_directory);
}
