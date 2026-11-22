//! Backup contents and the credentials eligible for export.
//! Fixtures include mail accounts, provider settings and feed subscriptions.
//! Encrypted and plaintext exports exercise different rules for stored secrets.

use super::*;

#[test]
fn plaintext_backup_round_trips_accounts_settings_and_feeds() {
    let source = test_conn();
    insert_mail_account(
        &source,
        "ada@example.com",
        sample_config(),
        json!({ "muted": true, "aliases": [{ "email": "ada@alt.example", "name": "Ada" }] }),
    );
    insert_rss_account(&source, "rss-1");
    insert_subscription(
        &source,
        "rss-1",
        "feed-abc",
        "https://blog.example/feed.xml",
        "Blog",
    );
    store::setting_set(&source, "theme_id", &json!("nord")).unwrap();
    store::setting_set(&source, "font_scale", &json!(115)).unwrap();

    let text = export(&source, false, None, Host::default(), &no_secrets).unwrap();
    let data = parse(&text, None).unwrap();

    let target = test_conn();
    let sink = SecretSink::default();
    let summary = apply(&target, &data, &sink.saver()).unwrap();

    assert_eq!(summary.accounts, 2);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.feeds, 1);
    assert_eq!(summary.settings, 2);
    assert_eq!(summary.secrets, 0);

    // Accounts came back with their metadata and prefs intact.
    let restored = store::list_accounts(&target).unwrap();
    assert_eq!(restored.len(), 2);
    let mail = restored
        .iter()
        .find(|a| a["id"] == "ada@example.com")
        .unwrap();
    assert_eq!(mail["imap_host"], "imap.example.com");
    assert_eq!(mail["smtp_port"], 465);
    assert_eq!(mail["sender_name"], "Ada");
    assert_eq!(mail["muted"], true);
    assert_eq!(mail["aliases"][0]["email"], "ada@alt.example");

    // Settings came back parsed, not as JSON-in-a-string.
    let settings =
        store::settings_get(&target, &["theme_id".to_string(), "font_scale".to_string()]).unwrap();
    assert_eq!(settings["theme_id"], "nord");
    assert_eq!(settings["font_scale"], 115);

    // The feed is attached to the restored RSS account.
    let (account, url): (String, String) = target
        .query_row("SELECT account, url FROM subscriptions", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();
    assert_eq!(account, "rss-1");
    assert_eq!(url, "https://blog.example/feed.xml");
}

#[test]
fn backup_omits_cached_mail() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    source
        .execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, body, date)
             VALUES('ada@example.com', 'INBOX', '1', 1, 'Secret subject', 'body text', 100)",
            [],
        )
        .unwrap();

    let text = export(&source, false, None, Host::default(), &no_secrets).unwrap();
    assert!(!text.contains("Secret subject"));
    assert!(!text.contains("body text"));

    let target = test_conn();
    let sink = SecretSink::default();
    apply(&target, &parse(&text, None).unwrap(), &sink.saver()).unwrap();
    let messages: i64 = target
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(messages, 0);
}

#[test]
fn plaintext_backup_redacts_proxy_and_oauth_client_secrets() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    store::setting_set(
        &source,
        crate::proxy::SETTING_KEY,
        &json!({ "mode": "http", "host": "proxy.example", "port": 3128, "username": "u", "password": "app-proxy-pass" }),
    )
    .unwrap();

    let text = export(&source, false, None, Host::default(), &no_secrets).unwrap();
    assert!(!text.contains("proxy-pass"));
    assert!(!text.contains("app-proxy-pass"));
    assert!(!text.contains("app-client-secret"));

    // Everything else about the proxy survives, so only the password is retyped.
    let data = parse(&text, None).unwrap();
    assert_eq!(
        data.settings[crate::proxy::SETTING_KEY]["host"],
        "proxy.example"
    );
    assert_eq!(data.settings[crate::proxy::SETTING_KEY]["password"], "");
    assert_eq!(data.accounts[0].config["proxy"]["username"], "u");
    assert_eq!(data.accounts[0].config["proxy"]["password"], "");
    assert!(data.accounts[0].config.get("oauth_client_secret").is_none());
    assert_eq!(data.accounts[0].config["host"], "imap.example.com");
}

#[test]
fn the_envelope_records_which_host_wrote_the_file() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));

    // Desktop and mobile version separately, so the same number can come from
    // either: the platform is what makes it readable.
    let host = Host {
        platform: "android",
        app_version: "0.2.4",
    };
    let text = export(&source, false, None, host, &no_secrets).unwrap();
    assert!(text.contains("\"platform\": \"android\""));
    assert!(text.contains("\"app_version\": \"0.2.4\""));

    // A host with nothing to offer leaves the fields out rather than stamping
    // values that mean nothing.
    let host = Host {
        platform: "  ",
        app_version: "",
    };
    let text = export(&source, false, None, host, &no_secrets).unwrap();
    assert!(!text.contains("app_version"));
    assert!(!text.contains("platform"));
}

#[test]
fn exporting_secrets_without_a_passphrase_is_refused() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));

    let error = export(&source, true, None, Host::default(), &|_| {
        secrets_with("hunter2")
    })
    .unwrap_err();
    assert!(error.to_string().contains("passphrase is required"));

    // An empty passphrase is treated as no passphrase, not as a real one.
    let error = export(&source, true, Some(""), Host::default(), &|_| {
        secrets_with("hunter2")
    })
    .unwrap_err();
    assert!(error.to_string().contains("passphrase is required"));
}

#[test]
fn encrypted_backup_round_trips_secrets_and_hides_them_on_disk() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));

    let text = export(
        &source,
        true,
        Some("correct horse"),
        Host::default(),
        &|_| secrets_with("hunter2"),
    )
    .unwrap();

    // Nothing sensitive is readable in the file, but the envelope still is.
    assert!(!text.contains("hunter2"));
    assert!(!text.contains("imap.example.com"));
    assert!(!text.contains("proxy-pass"));
    assert!(text.contains("\"meron_backup\""));
    assert!(text.contains("\"encrypted\": true"));
    assert!(text.contains("\"has_secrets\": true"));

    let data = parse(&text, Some("correct horse")).unwrap();
    // With secrets included, config keeps the fields a plaintext export strips.
    assert_eq!(data.accounts[0].config["proxy"]["password"], "proxy-pass");
    assert_eq!(
        data.accounts[0].config["oauth_client_secret"],
        "app-client-secret"
    );

    let target = test_conn();
    let sink = SecretSink::default();
    let summary = apply(&target, &data, &sink.saver()).unwrap();
    assert_eq!(summary.accounts, 1);
    assert_eq!(summary.secrets, 1);
    assert_eq!(
        sink.0.lock().unwrap().get("ada@example.com").unwrap(),
        "hunter2"
    );
}

#[test]
fn accounts_with_no_stored_secret_carry_none() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));

    let text = export(&source, true, Some("pw"), Host::default(), &no_secrets).unwrap();
    let data = parse(&text, Some("pw")).unwrap();
    assert!(data.accounts[0].secrets.is_none());

    let target = test_conn();
    let sink = SecretSink::default();
    let summary = apply(&target, &data, &sink.saver()).unwrap();
    assert_eq!(summary.secrets, 0);
    assert!(sink.0.lock().unwrap().is_empty());
}

#[test]
fn an_ai_sorting_key_never_reaches_a_backup() {
    use crate::engine::EngineHost;
    let source = std::sync::Mutex::new(test_conn());
    let key = "sk-or-v1-testvalue1234";
    let host = crate::protocol::MobileHost {
        data_dir: String::new(),
    };
    host.store_named_secret(&source, &crate::ai_sort::key_name("openrouter"), Some(key))
        .unwrap();
    let conn = source.lock().unwrap();
    insert_mail_account(&conn, "ada@example.com", sample_config(), json!({}));
    store::setting_set(
        &conn,
        store::AI_SORT_KEY,
        &json!({ "provider": "openrouter" }),
    )
    .unwrap();
    let load = |account: &str| crate::protocol::load_mobile_secret(&conn, account);

    let plain = export(&conn, false, None, Host::default(), &load).unwrap();
    assert!(!plain.contains(key));
    let sealed = export(&conn, true, Some("pw"), Host::default(), &load).unwrap();
    let data = parse(&sealed, Some("pw")).unwrap();
    let opened = serde_json::to_string(&(&data.accounts, &data.settings)).unwrap();
    assert!(opened.contains("openrouter") && !opened.contains(key));
}
