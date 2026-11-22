//! Exercises store behavior for accounts.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn rejected_sign_in_persists_until_the_account_is_saved_again() {
    let conn = test_conn();
    let creds = proxy_test_creds(crate::proxy::ProxyChoice::Global);
    let meta = AccountMeta {
        engine: "mail".to_string(),
        provider: "custom".to_string(),
        email: "user@example.com".to_string(),
        display_name: String::new(),
        avatar_url: String::new(),
        sender_name: String::new(),
    };
    upsert_account(&conn, "acct", &meta, &creds).unwrap();
    assert_eq!(stored_creds(&conn, "acct").rejected_grant, None);

    let refused = crate::imap::grant_fingerprint("old-token");
    set_account_rejected_grant(&conn, "acct", &refused).unwrap();
    let mut stored = stored_creds(&conn, "acct");
    assert_eq!(stored.rejected_grant.as_deref(), Some(refused.as_str()));
    // Nothing else in the config moved.
    assert_eq!(stored.host, creds.host);
    assert_eq!(stored.token_expires_at, creds.token_expires_at);

    // The verdict is on that token alone (secrets are applied after loading).
    stored.refresh_token = Some("old-token".to_string());
    assert!(stored.auth_rejected());
    stored.refresh_token = Some("new-token".to_string());
    assert!(!stored.auth_rejected());
    stored.refresh_token = None;
    assert!(!stored.auth_rejected());

    upsert_account(&conn, "acct", &meta, &creds).unwrap();
    assert_eq!(stored_creds(&conn, "acct").rejected_grant, None);
}

#[test]
fn account_cert_pin_round_trips_and_defaults_to_none() {
    let conn = test_conn();
    let mut creds = proxy_test_creds(crate::proxy::ProxyChoice::Global);
    creds.cert_pin = Some("a".repeat(64));
    creds.smtp_cert_pin = Some("b".repeat(64));
    let meta = AccountMeta {
        engine: "mail".to_string(),
        provider: "custom".to_string(),
        email: "user@example.com".to_string(),
        display_name: String::new(),
        avatar_url: String::new(),
        sender_name: String::new(),
    };
    upsert_account(&conn, "acct", &meta, &creds).unwrap();
    let stored = stored_creds(&conn, "acct");
    assert_eq!(stored.cert_pin.as_deref(), Some("a".repeat(64).as_str()));
    assert_eq!(
        stored.smtp_cert_pin.as_deref(),
        Some("b".repeat(64).as_str())
    );

    // Accounts written before pinning existed have no keys in their config JSON.
    conn.execute(
        "UPDATE accounts SET config = json_remove(config, '$.cert_pin', '$.smtp_cert_pin') WHERE id = 'acct'",
        [],
    )
    .unwrap();
    let stored = stored_creds(&conn, "acct");
    assert_eq!(stored.cert_pin, None);
    assert_eq!(stored.smtp_cert_pin, None);
}

#[test]
fn saving_an_account_again_keeps_settings_the_form_does_not_carry() {
    let conn = test_conn();
    let custom = crate::proxy::ProxyChoice::from_json(&json!({
        "mode": "socks5",
        "host": "127.0.0.1",
        "port": 9050,
    }));
    let mut creds = proxy_test_creds(custom.clone());
    creds.cert_pin = Some("a".repeat(64));
    creds.smtp_cert_pin = Some("b".repeat(64));
    let meta = AccountMeta {
        engine: "mail".to_string(),
        provider: "custom".to_string(),
        email: "user@example.com".to_string(),
        display_name: String::new(),
        avatar_url: String::new(),
        sender_name: String::new(),
    };
    upsert_account(&conn, "acct", &meta, &creds).unwrap();

    // What a reconnect builds: servers and credentials, nothing else.
    let mut reconnected = proxy_test_creds(crate::proxy::ProxyChoice::Global);
    let stored = load_account(&conn, "acct").unwrap().expect("account");
    reconnected.carry_over(
        &stored,
        crate::imap::OmittedSettings {
            proxy: true,
            cert_pin: true,
            smtp_cert_pin: true,
            password: true,
        },
    );
    upsert_account(&conn, "acct", &meta, &reconnected).unwrap();

    let after = stored_creds(&conn, "acct");
    assert_eq!(after.proxy, custom);
    assert_eq!(after.cert_pin.as_deref(), Some("a".repeat(64).as_str()));
    assert_eq!(
        after.smtp_cert_pin.as_deref(),
        Some("b".repeat(64).as_str())
    );
}

#[test]
fn a_setting_that_was_sent_is_not_carried_over() {
    let mut creds = proxy_test_creds(crate::proxy::ProxyChoice::Direct);
    creds.cert_pin = Some("a".repeat(64));
    let mut replacement = proxy_test_creds(crate::proxy::ProxyChoice::Global);
    replacement.cert_pin = None;
    replacement.carry_over(
        &creds,
        crate::imap::OmittedSettings {
            proxy: false,
            cert_pin: false,
            smtp_cert_pin: true,
            password: false,
        },
    );

    assert_eq!(replacement.proxy, crate::proxy::ProxyChoice::Global);
    assert_eq!(replacement.cert_pin, None);
}

#[test]
fn an_omitted_password_is_carried_over() {
    let mut stored = proxy_test_creds(crate::proxy::ProxyChoice::Direct);
    stored.password = "hunter2".to_string();

    let mut edited = proxy_test_creds(crate::proxy::ProxyChoice::Direct);
    edited.password = String::new();
    edited.port = 1143;
    edited.carry_over(
        &stored,
        crate::imap::OmittedSettings {
            proxy: false,
            cert_pin: false,
            smtp_cert_pin: false,
            password: true,
        },
    );

    assert_eq!(edited.password, "hunter2");
    // The edit itself still applies.
    assert_eq!(edited.port, 1143);
}

#[test]
fn a_sent_password_replaces_the_stored_one() {
    let mut stored = proxy_test_creds(crate::proxy::ProxyChoice::Direct);
    stored.password = "old".to_string();

    let mut replacement = proxy_test_creds(crate::proxy::ProxyChoice::Direct);
    replacement.password = "new".to_string();
    replacement.carry_over(
        &stored,
        crate::imap::OmittedSettings {
            proxy: false,
            cert_pin: false,
            smtp_cert_pin: false,
            password: false,
        },
    );

    assert_eq!(replacement.password, "new");
}

#[test]
fn upsert_account_keeps_an_existing_avatar() {
    let conn = test_conn();
    proxy_test_account(&conn, "acct", crate::proxy::ProxyChoice::Direct);
    conn.execute(
        "UPDATE accounts SET avatar_url = ?1 WHERE id = ?2",
        rusqlite::params!["/media/avatars/acct.png", "acct"],
    )
    .unwrap();

    proxy_test_account(&conn, "acct", crate::proxy::ProxyChoice::Direct);

    let stored: String = conn
        .query_row(
            "SELECT avatar_url FROM accounts WHERE id = ?1",
            ["acct"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, "/media/avatars/acct.png");
}

#[test]
fn set_account_cert_pins_replaces_only_the_pins() {
    let conn = test_conn();
    proxy_test_account(&conn, "acct", crate::proxy::ProxyChoice::Direct);
    set_account_cert_pins(&conn, "acct", Some(&"c".repeat(64)), None).unwrap();

    let stored = stored_creds(&conn, "acct");
    assert_eq!(stored.cert_pin.as_deref(), Some("c".repeat(64).as_str()));
    assert_eq!(stored.smtp_cert_pin, None);
    assert_eq!(stored.host, "imap.example.com");
    assert_eq!(stored.smtp_port, 587);
    assert_eq!(stored.proxy, crate::proxy::ProxyChoice::Direct);

    // Clearing is explicit: pass no pin and the account stops trusting it.
    set_account_cert_pins(&conn, "acct", None, None).unwrap();
    assert_eq!(stored_creds(&conn, "acct").cert_pin, None);
}

#[test]
fn account_proxy_round_trips_and_defaults_to_global() {
    let conn = test_conn();
    let custom = crate::proxy::ProxyChoice::from_json(&json!({
        "mode": "socks5",
        "host": "127.0.0.1",
        "port": 9050,
        "username": "u",
        "password": "p",
    }));
    proxy_test_account(&conn, "acct", custom.clone());
    assert_eq!(stored_proxy(&conn, "acct"), custom);

    // An account written before proxy support has no key in its config JSON.
    conn.execute(
        "UPDATE accounts SET config = json_remove(config, '$.proxy') WHERE id = 'acct'",
        [],
    )
    .unwrap();
    assert_eq!(
        stored_proxy(&conn, "acct"),
        crate::proxy::ProxyChoice::Global
    );
}

#[test]
fn set_account_proxy_replaces_only_the_proxy_entry() {
    let conn = test_conn();
    proxy_test_account(&conn, "acct", crate::proxy::ProxyChoice::Global);
    set_account_proxy(&conn, "acct", &crate::proxy::ProxyChoice::Direct).unwrap();

    let (_, creds) = load_accounts(&conn)
        .unwrap()
        .into_iter()
        .find(|(account, _)| account == "acct")
        .unwrap();
    assert_eq!(creds.proxy, crate::proxy::ProxyChoice::Direct);
    // The rest of the connection config survives the targeted update.
    assert_eq!(creds.host, "imap.example.com");
    assert_eq!(creds.smtp_port, 587);
    assert!(creds.smtp_starttls);
}
