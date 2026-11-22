//! App-wide and per-account mobile settings.
//! The proxy cases exercise both levels of configuration.
//! Read-back assertions cover preferences written by the protocol.

use super::*;

#[test]
fn mobile_protocol_persists_app_and_account_proxies() {
    let _guard = crate::proxy::TEST_GLOBAL_LOCK
        .write()
        .unwrap_or_else(|error| error.into_inner());
    let data_directory = unique_data_directory("proxy");
    let unique = unique_test_suffix();
    let email = format!("proxy+{unique}@example.com");
    let directory_text = data_directory.to_str().unwrap();
    let add_request = format!(
        r#"{{"id":90,"method":"account.addPassword","params":{{"email":"{email}","imap_host":"imap.example.com","smtp_host":"smtp.example.com","username":"{email}","password":"secret"}}}}"#
    );
    assert!(
        invoke_mobile_protocol_json(&add_request, Some(directory_text))
            .get("error")
            .is_none()
    );

    // A fresh install has no proxy, and a new account follows the app-wide one.
    let empty = invoke_mobile_protocol_json(
        r#"{"id":91,"method":"app.proxyGet","params":{}}"#,
        Some(directory_text),
    );
    assert_eq!(empty["result"]["proxy"], serde_json::Value::Null);
    let listed = invoke_mobile_protocol_json(
        r#"{"id":92,"method":"account.list","params":{}}"#,
        Some(directory_text),
    );
    assert_eq!(listed["result"]["accounts"][0]["proxy"]["mode"], "global");

    let set = invoke_mobile_protocol_json(
        r#"{"id":93,"method":"app.proxySet","params":{"proxy":{"mode":"socks5","host":"127.0.0.1","port":9050,"username":"u","password":"p"}}}"#,
        Some(directory_text),
    );
    assert_eq!(set["result"]["ok"], true);
    let stored = invoke_mobile_protocol_json(
        r#"{"id":94,"method":"app.proxyGet","params":{}}"#,
        Some(directory_text),
    );
    assert_eq!(stored["result"]["proxy"]["mode"], "socks5");
    assert_eq!(stored["result"]["proxy"]["port"], 9050);
    // Saving publishes it, so connections made right after use it without a restart.
    assert_eq!(
        crate::proxy::global().map(|configuration| configuration.host),
        Some("127.0.0.1".to_string())
    );

    let account_set = format!(
        r#"{{"id":95,"method":"account.setProxy","params":{{"id":"{email}","proxy":{{"mode":"direct"}}}}}}"#
    );
    assert_eq!(
        invoke_mobile_protocol_json(&account_set, Some(directory_text))["result"]["ok"],
        true
    );
    let listed = invoke_mobile_protocol_json(
        r#"{"id":96,"method":"account.list","params":{}}"#,
        Some(directory_text),
    );
    assert_eq!(listed["result"]["accounts"][0]["proxy"]["mode"], "direct");

    crate::proxy::set_global(None);
    let _ = std::fs::remove_dir_all(data_directory);
}

#[test]
fn mobile_protocol_reads_back_settings_it_wrote() {
    let directory = unique_data_directory("mobile-prefs");
    let data_directory = directory.to_str().unwrap();

    for (key, value) in [
        (r#""mobile.app.appearance_mode_v1""#, r#""Indigo""#),
        (r#""mobile.app.message_font_scale_v1""#, "115"),
        (r#""mobile.app.background_sync_enabled_v1""#, "false"),
        (
            r#""mobile.app.hidden_navigation_accounts_v1""#,
            r#"["a@example.com"]"#,
        ),
    ] {
        let request = format!(
            r#"{{"id":1,"method":"app.prefsSet","params":{{"key":{key},"value":{value}}}}}"#
        );
        let value = invoke_mobile_protocol_json(&request, Some(data_directory));
        assert!(value.get("error").is_none(), "{value}");
    }

    let get = json!({
        "id": 2,
        "method": "app.prefsGet",
        "params": {
            "keys": [
                "mobile.app.appearance_mode_v1",
                "mobile.app.message_font_scale_v1",
                "mobile.app.background_sync_enabled_v1",
                "mobile.app.hidden_navigation_accounts_v1",
                "mobile.app.never_set_v1",
            ],
        },
    });
    let value = invoke_mobile_protocol_json(&get.to_string(), Some(data_directory));
    assert!(value.get("error").is_none(), "{value}");
    let prefs = &value["result"]["prefs"];
    // Types survive the round trip rather than all becoming strings.
    assert_eq!(prefs["mobile.app.appearance_mode_v1"], "Indigo");
    assert_eq!(prefs["mobile.app.message_font_scale_v1"], 115);
    assert_eq!(prefs["mobile.app.background_sync_enabled_v1"], false);
    assert_eq!(
        prefs["mobile.app.hidden_navigation_accounts_v1"][0],
        "a@example.com"
    );
    // A key never written is absent, not defaulted — that is what lets the host
    // keep its own cached value instead of being reset to a default.
    assert!(prefs.get("mobile.app.never_set_v1").is_none(), "{prefs}");

    let _ = std::fs::remove_dir_all(directory);
}
