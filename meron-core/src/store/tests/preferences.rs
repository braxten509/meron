//! Exercises store behavior for preferences.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn delete_account_removes_account_scoped_state_only() {
    let conn = test_conn();
    for account in ["acct", "other"] {
        conn.execute(
            "INSERT INTO accounts(id, email) VALUES(?1, ?1)",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folders(account, name) VALUES(?1, 'INBOX')",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid) VALUES(?1, 'INBOX', '1', 1)",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO folder_state(account, folder, uid_next) VALUES(?1, 'INBOX', 2)",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO subscriptions(id, account, url, title) VALUES(?2, ?1, ?3, 'Feed')",
            params![
                account,
                format!("feed-{account}"),
                format!("https://{account}.example")
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO account_secrets(account_id, blob) VALUES(?1, 'secret')",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO observed_mail_identities(account, identity, first_seen_at)
             VALUES(?1, 'message-id:seen@example.com', 1)",
            params![account],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO mail_search_hits(
               token, account, query, scope, position, folder, uid, created_at
             ) VALUES(?2, ?1, 'query', '[\"INBOX\"]', 0, 'INBOX', 1, 1)",
            params![account, format!("token-{account}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO mail_search_snapshots(token, account, query, scope, created_at)
             VALUES(?2, ?1, 'query', '[\"INBOX\"]', 1)",
            params![account, format!("token-{account}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO mail_search_pending(token, account, folder, uid, date)
             VALUES(?2, ?1, 'INBOX', 2, 1)",
            params![account, format!("token-{account}")],
        )
        .unwrap();
    }

    delete_account(&conn, "acct").unwrap();

    for (table, column) in [
        ("accounts", "id"),
        ("folders", "account"),
        ("messages", "account"),
        ("folder_state", "account"),
        ("subscriptions", "account"),
        ("account_secrets", "account_id"),
        ("observed_mail_identities", "account"),
        ("mail_search_hits", "account"),
        ("mail_search_snapshots", "account"),
        ("mail_search_pending", "account"),
    ] {
        let deleted_count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {column} = 'acct'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(deleted_count, 0, "{table} retained deleted account rows");

        let other_count: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {column} = 'other'"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        let expected = if table == "observed_mail_identities" {
            2
        } else {
            1
        };
        assert_eq!(
            other_count, expected,
            "{table} removed another account's row"
        );
    }
}

#[test]
fn prefs_resolve_engine_default_and_persist_without_clobbering() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(db::ACCOUNTS_DDL).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine) VALUES('rss-1', 'rss')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine) VALUES('mail-1', 'mail')",
        [],
    )
    .unwrap();

    // Unset: resolved by engine (RSS on, mail off).
    assert!(load_remote_images(&conn, "rss-1").unwrap());
    assert!(!load_remote_images(&conn, "mail-1").unwrap());
    assert!(!load_remote_images(&conn, "missing").unwrap());

    // A sibling pref must survive an images toggle (json_set is in-place).
    conn.execute(
        "UPDATE accounts SET prefs = json_set(prefs, '$.other', 7) WHERE id = 'mail-1'",
        [],
    )
    .unwrap();
    set_load_remote_images(&conn, "mail-1", true).unwrap();
    assert!(load_remote_images(&conn, "mail-1").unwrap());
    let prefs: String = conn
        .query_row("SELECT prefs FROM accounts WHERE id = 'mail-1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        prefs.contains("\"other\""),
        "sibling pref clobbered: {prefs}"
    );

    // Explicit override of the engine default round-trips.
    set_load_remote_images(&conn, "rss-1", false).unwrap();
    assert!(!load_remote_images(&conn, "rss-1").unwrap());
}

#[test]
fn allowed_senders_load_remote_images_while_the_account_toggle_stays_off() {
    let conn = test_conn();
    for id in ["mail-1", "mail-2"] {
        conn.execute(
            "INSERT INTO accounts(id, engine) VALUES(?1, 'mail')",
            params![id],
        )
        .unwrap();
    }

    let policy = remote_image_policy(&conn, "mail-1").unwrap();
    assert!(!policy.all);
    assert!(!policy.allows("news@example.com"));

    // The allowlist is app-wide: one entry lifts the block in every account,
    // while each account's own toggle stays off. Stored forms are normalized on
    // read, so a "Name <addr>" entry and casing still match.
    setting_set(
        &conn,
        REMOTE_IMAGE_SENDERS_KEY,
        &json!(["News <News@Example.com>", "news@example.com", ""]),
    )
    .unwrap();
    assert_eq!(
        remote_image_senders(&conn).unwrap(),
        vec!["news@example.com".to_string()]
    );
    for id in ["mail-1", "mail-2"] {
        let policy = remote_image_policy(&conn, id).unwrap();
        assert!(!policy.all);
        assert!(policy.allows("News@Example.COM"));
        assert!(!policy.allows("someone@else.test"));
        assert!(!policy.allows(""));
    }

    // The account-wide toggle still allows every sender on its own.
    set_load_remote_images(&conn, "mail-1", true).unwrap();
    assert!(
        remote_image_policy(&conn, "mail-1")
            .unwrap()
            .allows("anyone@example.com")
    );
}

#[test]
fn conversation_html_defaults_on_and_respects_explicit_overrides() {
    let conn = Connection::open_in_memory().unwrap();
    db::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, prefs) VALUES
                ('unset', 'mail', 'custom', '{}'),
                ('plain', 'mail', 'custom', '{\"conversation_html\":false}'),
                ('html', 'mail', 'custom', '{\"conversation_html\":true}')",
        [],
    )
    .unwrap();

    let accounts = list_accounts(&conn).unwrap();
    let by_id = |id: &str| {
        accounts
            .iter()
            .find(|account| account["id"] == id)
            .expect("account present")
    };

    assert_eq!(by_id("unset")["conversation_html"], true);
    assert_eq!(by_id("plain")["conversation_html"], false);
    assert_eq!(by_id("html")["conversation_html"], true);
}

#[test]
fn chat_wallpaper_round_trips_and_can_be_cleared() {
    let conn = Connection::open_in_memory().unwrap();
    db::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, prefs) VALUES('acct', 'mail', 'custom', '{}')",
        [],
    )
    .unwrap();

    let wallpaper = json!({"kind":"preset","presetId":"grid"});
    set_account_pref_json(&conn, "acct", "chat_wallpaper", Some(wallpaper.clone())).unwrap();
    let accounts = list_accounts(&conn).unwrap();
    assert_eq!(accounts[0]["chat_wallpaper"], wallpaper);

    conn.execute(
        "UPDATE accounts SET prefs = json_set(prefs, '$.other', 7) WHERE id = 'acct'",
        [],
    )
    .unwrap();
    set_account_pref_json(&conn, "acct", "chat_wallpaper", None).unwrap();
    let prefs: String = conn
        .query_row("SELECT prefs FROM accounts WHERE id = 'acct'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let value: Value = serde_json::from_str(&prefs).unwrap();
    assert_eq!(value["chat_wallpaper"], Value::Null);
    assert_eq!(value["other"], 7);
}

#[test]
fn signature_pref_round_trips_and_can_be_cleared() {
    let conn = Connection::open_in_memory().unwrap();
    db::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, prefs) VALUES('acct', 'mail', 'custom', '{}')",
        [],
    )
    .unwrap();

    // Absent means "follow the app-wide signature".
    assert_eq!(list_accounts(&conn).unwrap()[0]["signature"], Value::Null);

    let signature =
        AccountSignature::from_param(Some(&json!({"mode":"custom","html":"<p>Ping</p>"})))
            .unwrap()
            .unwrap();
    set_account_pref_json(&conn, "acct", "signature", Some(json!(signature))).unwrap();
    assert_eq!(
        list_accounts(&conn).unwrap()[0]["signature"],
        json!({"mode":"custom","html":"<p>Ping</p>"})
    );

    set_account_pref_json(&conn, "acct", "signature", None).unwrap();
    assert_eq!(list_accounts(&conn).unwrap()[0]["signature"], Value::Null);
}

#[test]
fn an_unreadable_pref_does_not_reset_the_others() {
    let conn = Connection::open_in_memory().unwrap();
    db::run_migrations(&conn).unwrap();
    // `signature` carries a mode this build cannot read, next to prefs that are
    // perfectly valid: only the signature falls back to its default.
    conn.execute(
        r#"INSERT INTO accounts(id, engine, provider, prefs)
           VALUES('acct', 'mail', 'custom',
                  '{"muted":true,"paused":true,"included_in_unified":false,
                    "conversation_html":false,"signature":"<p>Ping</p>"}')"#,
        [],
    )
    .unwrap();

    let account = list_accounts(&conn).unwrap().remove(0);
    assert_eq!(account["muted"], true);
    assert_eq!(account["paused"], true);
    assert_eq!(account["included_in_unified"], false);
    assert_eq!(account["conversation_html"], false);
    assert_eq!(account["signature"], Value::Null);
}

#[test]
fn signature_param_validates_its_mode_and_size() {
    assert!(AccountSignature::from_param(None).unwrap().is_none());
    assert!(
        AccountSignature::from_param(Some(&Value::Null))
            .unwrap()
            .is_none()
    );
    assert!(AccountSignature::from_param(Some(&json!({"mode":"sometimes"}))).is_err());
    assert!(AccountSignature::from_param(Some(&json!("<p>Ping</p>"))).is_err());
    assert!(
        AccountSignature::from_param(Some(&json!({
            "mode": "custom",
            "html": "x".repeat(MAX_SIGNATURE_HTML + 1),
        })))
        .is_err()
    );

    // The html is kept in every mode, so toggling back restores what was written.
    let kept = AccountSignature::from_param(Some(&json!({"mode":"none","html":" <p>Ping</p> "})))
        .unwrap()
        .unwrap();
    assert_eq!(kept.mode, SignatureMode::None);
    assert_eq!(kept.html, "<p>Ping</p>");
}

#[test]
fn save_sent_copy_pref_round_trips_and_can_be_cleared() {
    let conn = Connection::open_in_memory().unwrap();
    db::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, prefs) VALUES('acct', 'mail', 'custom', '{}')",
        [],
    )
    .unwrap();

    assert_eq!(save_sent_copy_pref(&conn, "acct").unwrap(), None);
    set_account_pref_json(&conn, "acct", "save_sent_copy", Some(json!(false))).unwrap();
    assert_eq!(save_sent_copy_pref(&conn, "acct").unwrap(), Some(false));
    assert_eq!(list_accounts(&conn).unwrap()[0]["save_sent_copy"], false);

    conn.execute(
        "UPDATE accounts SET prefs = json_set(prefs, '$.other', 7) WHERE id = 'acct'",
        [],
    )
    .unwrap();
    set_account_pref_json(&conn, "acct", "save_sent_copy", None).unwrap();
    assert_eq!(save_sent_copy_pref(&conn, "acct").unwrap(), None);
    let prefs: String = conn
        .query_row("SELECT prefs FROM accounts WHERE id = 'acct'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let value: Value = serde_json::from_str(&prefs).unwrap();
    assert_eq!(value["save_sent_copy"], Value::Null);
    assert_eq!(value["other"], 7);
}

#[test]
fn rss_sync_interval_defaults_and_persists_without_clobbering() {
    let conn = Connection::open_in_memory().unwrap();
    db::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider) VALUES('rss-1', 'rss', 'rss')",
        [],
    )
    .unwrap();

    let accounts = list_accounts(&conn).unwrap();
    assert_eq!(
        accounts[0]["rss_sync_interval_minutes"],
        DEFAULT_RSS_SYNC_INTERVAL_MINUTES
    );

    conn.execute(
        "UPDATE accounts SET prefs = json_set(prefs, '$.other', 7) WHERE id = 'rss-1'",
        [],
    )
    .unwrap();
    set_account_pref_u64(&conn, "rss-1", "rss_sync_interval_minutes", 30).unwrap();

    let accounts = list_accounts(&conn).unwrap();
    assert_eq!(accounts[0]["rss_sync_interval_minutes"], 30);
    let prefs: String = conn
        .query_row("SELECT prefs FROM accounts WHERE id = 'rss-1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(
        prefs.contains("\"other\""),
        "sibling pref clobbered: {prefs}"
    );
}
