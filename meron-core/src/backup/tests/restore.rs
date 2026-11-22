//! Restoring accounts, settings and feeds into an existing installation.
//! The cases cover duplicate accounts and rollback on failure.
//! Older platform preferences and additional fields must survive import.

use super::*;

#[test]
fn existing_accounts_are_skipped_not_overwritten() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    let data = parse(
        &export(&source, false, None, Host::default(), &no_secrets).unwrap(),
        None,
    )
    .unwrap();

    // The target already has this account, pointed at a different server.
    let target = test_conn();
    insert_mail_account(
        &target,
        "ada@example.com",
        json!({ "host": "live.example.com", "port": 993, "user": "ada" }),
        json!({}),
    );

    let sink = SecretSink::default();
    let summary = apply(&target, &data, &sink.saver()).unwrap();
    assert_eq!(summary.accounts, 0);
    assert_eq!(summary.skipped, 1);

    let host: String = target
        .query_row(
            "SELECT json_extract(config, '$.host') FROM accounts WHERE id = 'ada@example.com'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(host, "live.example.com", "the live account was overwritten");
}

#[test]
fn settings_overwrite_on_restore() {
    let source = test_conn();
    store::setting_set(&source, "theme_id", &json!("nord")).unwrap();
    let data = parse(
        &export(&source, false, None, Host::default(), &no_secrets).unwrap(),
        None,
    )
    .unwrap();

    let target = test_conn();
    store::setting_set(&target, "theme_id", &json!("solarized")).unwrap();
    store::setting_set(&target, "font_scale", &json!(150)).unwrap();

    let sink = SecretSink::default();
    apply(&target, &data, &sink.saver()).unwrap();

    let settings =
        store::settings_get(&target, &["theme_id".to_string(), "font_scale".to_string()]).unwrap();
    assert_eq!(settings["theme_id"], "nord");
    // A key the backup doesn't mention is left alone rather than reset.
    assert_eq!(settings["font_scale"], 150);
}

#[test]
fn a_feed_already_subscribed_is_not_duplicated() {
    let source = test_conn();
    insert_rss_account(&source, "rss-1");
    insert_subscription(
        &source,
        "rss-1",
        "feed-abc",
        "https://blog.example/feed.xml",
        "Blog",
    );
    let data = parse(
        &export(&source, false, None, Host::default(), &no_secrets).unwrap(),
        None,
    )
    .unwrap();

    let target = test_conn();
    insert_rss_account(&target, "rss-other");
    insert_subscription(
        &target,
        "rss-other",
        "feed-existing",
        "https://blog.example/feed.xml",
        "Blog",
    );

    let sink = SecretSink::default();
    let summary = apply(&target, &data, &sink.saver()).unwrap();
    assert_eq!(summary.accounts, 1);
    assert_eq!(summary.feeds, 0, "the duplicate URL was inserted anyway");

    let count: i64 = target
        .query_row("SELECT COUNT(*) FROM subscriptions", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn a_failed_import_leaves_the_store_untouched() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    insert_mail_account(&source, "bob@example.com", sample_config(), json!({}));
    let data = parse(
        &export(&source, true, Some("pw"), Host::default(), &|_| {
            secrets_with("s")
        })
        .unwrap(),
        Some("pw"),
    )
    .unwrap();

    // Secret storage failing mid-run (a locked keychain) must not leave half
    // the accounts behind.
    let target = test_conn();
    let summary = apply(&target, &data, &|_, account, _| {
        if account == "bob@example.com" {
            Err(anyhow!("keychain is locked"))
        } else {
            Ok(())
        }
    });
    assert!(summary.is_err());

    let count: i64 = target
        .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn accounts_with_a_blank_id_are_dropped() {
    let data = BackupData {
        accounts: vec![
            BackupAccount {
                id: "   ".to_string(),
                ..Default::default()
            },
            BackupAccount {
                id: "ada@example.com".to_string(),
                engine: "mail".to_string(),
                ..Default::default()
            },
        ],
        settings: Map::new(),
        ..Default::default()
    };

    let target = test_conn();
    let sink = SecretSink::default();
    let summary = apply(&target, &data, &sink.saver()).unwrap();
    assert_eq!(summary.accounts, 1);
    assert_eq!(summary.skipped, 0);
}

#[test]
fn empty_engine_and_provider_fall_back_to_mail_defaults() {
    let data = BackupData {
        accounts: vec![BackupAccount {
            id: "ada@example.com".to_string(),
            ..Default::default()
        }],
        settings: Map::new(),
        ..Default::default()
    };

    let target = test_conn();
    let sink = SecretSink::default();
    apply(&target, &data, &sink.saver()).unwrap();

    let (engine, provider): (String, String) = target
        .query_row(
            "SELECT engine, provider FROM accounts WHERE id = 'ada@example.com'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(engine, "mail");
    assert_eq!(provider, "custom");
}

#[test]
fn unknown_fields_from_a_newer_build_survive_a_round_trip() {
    // `config` and `prefs` are free-form, so a field this build has never heard
    // of must still come back rather than being silently dropped.
    let source = test_conn();
    insert_mail_account(
        &source,
        "ada@example.com",
        json!({ "host": "imap.example.com", "future_field": "keep me" }),
        json!({ "future_pref": 42 }),
    );

    let data = parse(
        &export(&source, false, None, Host::default(), &no_secrets).unwrap(),
        None,
    )
    .unwrap();
    let target = test_conn();
    let sink = SecretSink::default();
    apply(&target, &data, &sink.saver()).unwrap();

    let (config, prefs): (String, String) = target
        .query_row(
            "SELECT config, prefs FROM accounts WHERE id = 'ada@example.com'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&config).unwrap()["future_field"],
        "keep me"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&prefs).unwrap()["future_pref"],
        42
    );
}

/// A development build carried mobile preferences in their own `platform` map
/// before they became `settings` rows. Serde would ignore the field, restoring
/// such a file with every mobile preference silently dropped.
#[test]
fn a_legacy_platform_map_is_folded_into_settings() {
    let text = json!({
        "meron_backup": FORMAT_VERSION,
        "encrypted": false,
        "data": {
            "accounts": [],
            "settings": { "theme_id": "nord" },
            "platform": {
                "app:appearance_mode_v1": "Indigo",
                "app:message_font_scale_v1": 115,
                "app:hidden_navigation_accounts_v1": ["a@example.com"],
                "kanban:kanban_boards_v1": "[{\"id\":\"b1\"}]",
            },
        },
    });

    let data = parse(&text.to_string(), None).unwrap();

    assert_eq!(data.settings["mobile.app.appearance_mode_v1"], "Indigo");
    assert_eq!(data.settings["mobile.app.message_font_scale_v1"], 115);
    assert_eq!(
        data.settings["mobile.app.hidden_navigation_accounts_v1"][0],
        "a@example.com"
    );
    assert_eq!(
        data.settings["mobile.kanban.kanban_boards_v1"],
        "[{\"id\":\"b1\"}]"
    );
    // Existing rows are untouched.
    assert_eq!(data.settings["theme_id"], "nord");
}

#[test]
fn a_legacy_platform_map_never_overwrites_a_real_settings_row() {
    let text = json!({
        "meron_backup": FORMAT_VERSION,
        "encrypted": false,
        "data": {
            "settings": { "mobile.app.appearance_mode_v1": "Rose" },
            "platform": { "app:appearance_mode_v1": "Indigo" },
        },
    });

    let data = parse(&text.to_string(), None).unwrap();

    // The row is the newer home for the same setting, so it wins.
    assert_eq!(data.settings["mobile.app.appearance_mode_v1"], "Rose");
}

#[test]
fn a_legacy_platform_map_survives_encryption() {
    let source = test_conn();
    let text = export(&source, true, Some("pw"), Host::default(), &no_secrets).unwrap();

    // Rebuild the ciphertext around a payload carrying the old shape.
    let mut envelope: Value = serde_json::from_str(&text).unwrap();
    let payload = json!({
        "accounts": [],
        "settings": {},
        "platform": { "app:app_language_v1": "ja" },
    });
    let sealed = encrypt(payload.to_string().as_bytes(), "pw").unwrap();
    envelope["kdf"] = sealed.kdf_json();
    envelope["nonce"] = json!(STANDARD.encode(sealed.nonce));
    envelope["ciphertext"] = json!(STANDARD.encode(&sealed.ciphertext));

    let data = parse(&envelope.to_string(), Some("pw")).unwrap();
    assert_eq!(data.settings["mobile.app.app_language_v1"], "ja");
}

#[test]
fn a_backup_without_a_platform_map_is_unaffected() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    store::setting_set(&source, "theme_id", &json!("nord")).unwrap();

    let data = parse(
        &export(&source, false, None, Host::default(), &no_secrets).unwrap(),
        None,
    )
    .unwrap();

    assert_eq!(data.settings["theme_id"], "nord");
    assert_eq!(data.settings.len(), 1);
}
