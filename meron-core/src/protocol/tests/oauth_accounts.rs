//! OAuth account metadata and token updates.
//! Platform-managed accounts can exist without a refresh token.
//! The update cases cover stale tokens, missing accounts and unknown providers.

use super::*;

#[test]
fn mobile_protocol_persists_oauth_account_metadata() {
    let data_dir = unique_data_dir("oauth-account");
    let unique = unique_test_suffix();
    let email = format!("me+{unique}@gmail.com");
    let request = format!(
        r#"{{"id":63,"method":"account.addOAuth","params":{{"email":"{email}","provider":"gmail","display_name":"Gmail","sender_name":"Sender","access_token":"access","refresh_token":"refresh","token_expires_at":1700000000}}}}"#
    );
    let added = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    assert_eq!(added["id"], 63);
    assert_eq!(added["result"]["account"]["id"], email.as_str());
    assert_eq!(added["result"]["account"]["email"], email.as_str());
    assert_eq!(added["result"]["account"]["display_name"], "Gmail");
    assert_eq!(added["result"]["account"]["sender_name"], "Sender");
    assert_eq!(added["result"]["account"]["provider"], "gmail");
    assert_eq!(added["result"]["account"]["auth_type"], "gmail_oauth");
    assert_eq!(added["result"]["account"]["imap_host"], "imap.gmail.com");
    assert_eq!(added["result"]["account"]["imap_port"], 993);
    assert_eq!(added["result"]["account"]["smtp_host"], "smtp.gmail.com");
    assert_eq!(added["result"]["account"]["smtp_port"], 587);
    assert_eq!(added["result"]["account"]["needs_reconnect"], false);

    let listed = invoke_mobile_protocol_json(
        r#"{"id":64,"method":"account.list","params":{}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(listed["id"], 64);
    assert_eq!(listed["result"]["accounts"][0]["id"], email.as_str());
    assert_eq!(listed["result"]["accounts"][0]["needs_reconnect"], false);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_persists_platform_managed_oauth_account_without_refresh_token() {
    let data_dir = unique_data_dir("oauth-managed");
    let unique = unique_test_suffix();
    let email = format!("me+{unique}@gmail.com");
    let expires_at = now_epoch_seconds() + 3600;
    // Android AccountManager Gmail: access token only, no refresh token.
    let request = format!(
        r#"{{"id":70,"method":"account.addOAuth","params":{{"email":"{email}","provider":"gmail","display_name":"Gmail","access_token":"access-1","token_expires_at":{expires_at}}}}}"#
    );

    let added = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    assert_eq!(added["id"], 70, "{added}");
    assert_eq!(added["result"]["account"]["id"], email.as_str(), "{added}");
    assert_eq!(
        added["result"]["account"]["auth_type"], "gmail_oauth",
        "{added}"
    );
    // No refresh token, but a live access token => not flagged for reconnect.
    assert_eq!(
        added["result"]["account"]["needs_reconnect"], false,
        "{added}"
    );

    // Platform pushes a freshly minted token before sync.
    let new_expires = now_epoch_seconds() + 3600;
    let update = format!(
        r#"{{"id":71,"method":"account.updateOAuthToken","params":{{"account_id":"{email}","access_token":"access-2","token_expires_at":{new_expires}}}}}"#
    );
    let updated = invoke_mobile_protocol_json(&update, Some(data_dir.to_str().unwrap()));
    assert_eq!(updated["id"], 71, "{updated}");
    assert_eq!(updated["result"]["ok"], true, "{updated}");
    assert_eq!(
        updated["result"]["account"]["needs_reconnect"], false,
        "{updated}"
    );

    let listed = invoke_mobile_protocol_json(
        r#"{"id":72,"method":"account.list","params":{}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(
        listed["result"]["accounts"][0]["needs_reconnect"], false,
        "{listed}"
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_platform_oauth_update_clears_old_refresh_token() {
    let data_dir = unique_data_dir("oauth-managed-clears-refresh");
    let data_dir_str = data_dir.to_str().unwrap();
    let unique = unique_test_suffix();
    let email = format!("managed-reconnect+{unique}@gmail.com");
    let expired = now_epoch_seconds() - 3600;
    let add_browser = format!(
        r#"{{"id":74,"method":"account.addOAuth","params":{{"email":"{email}","provider":"gmail","display_name":"Gmail","access_token":"browser-access","refresh_token":"browser-refresh","token_expires_at":{expired}}}}}"#
    );
    let added = invoke_mobile_protocol_json(&add_browser, Some(data_dir_str));
    assert_eq!(added["id"], 74, "{added}");

    let managed_expires = now_epoch_seconds() + 3600;
    let update_managed = format!(
        r#"{{"id":75,"method":"account.updateOAuthToken","params":{{"account_id":"{email}","access_token":"managed-access","token_expires_at":{managed_expires}}}}}"#
    );
    let updated = invoke_mobile_protocol_json(&update_managed, Some(data_dir_str));
    assert_eq!(updated["id"], 75, "{updated}");
    assert_eq!(updated["result"]["ok"], true, "{updated}");

    let engine = crate::engine::Engine::new(Box::new(MobileHost {
        data_dir: data_dir_str.to_string(),
    }))
    .expect("engine builds");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let creds = rt
        .block_on(engine.ensure_valid_creds(&email))
        .expect("platform-managed token should not use browser refresh");
    assert_eq!(creds.access_token.as_deref(), Some("managed-access"));
    assert_eq!(creds.refresh_token.as_deref(), None);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_update_oauth_token_rejects_unknown_account() {
    let data_dir = unique_data_dir("oauth-managed-missing");
    let update = r#"{"id":73,"method":"account.updateOAuthToken","params":{"account_id":"nobody@gmail.com","access_token":"x"}}"#;
    let updated = invoke_mobile_protocol_json(update, Some(data_dir.to_str().unwrap()));
    assert!(updated["error"]["message"].is_string(), "{updated}");
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_rejects_unsupported_oauth_provider() {
    let data_dir = unique_data_dir("oauth-provider");
    let value = invoke_mobile_protocol_json(
        r#"{"id":65,"method":"account.addOAuth","params":{"email":"me@example.com","provider":"other","refresh_token":"refresh"}}"#,
        Some(data_dir.to_str().unwrap()),
    );

    assert_eq!(value["id"], 65);
    assert_eq!(value["error"]["message"], "unsupported oauth provider");
    let _ = std::fs::remove_dir_all(data_dir);
}
