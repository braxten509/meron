//! Account creation and settings through the mobile protocol.
//! Reconnect must retain the account's certificate pins and proxy.
//! The cases also cover credential validation, aliases and account ordering.

use super::*;

#[test]
fn mobile_protocol_allocates_stable_format_message_identity() {
    let data_dir = unique_data_dir("allocate-identity");
    let value = invoke_mobile_protocol_json(
        r#"{"id":44,"method":"mail.allocateIdentity","params":{"account_id":"me@example.com","draft":true}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    let id = value["result"]["message_id"].as_str().unwrap();
    assert!(id.starts_with("meron-draft-"), "{value}");
    assert!(id.ends_with("@example.com"), "{value}");
    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_persists_password_account_metadata() {
    let data_dir = unique_data_dir("account");
    let unique = unique_test_suffix();
    let email = format!("me+{unique}@example.com");
    let request = format!(
        r#"{{"id":60,"method":"account.addPassword","params":{{"email":"{email}","display_name":"Me","sender_name":"Sender","imap_host":"imap.example.com","imap_port":993,"smtp_host":"smtp.example.com","smtp_port":587,"username":"{email}","password":"secret","tls":true}}}}"#
    );
    let added = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    assert_eq!(added["id"], 60);
    assert_eq!(added["result"]["account"]["id"], email.as_str());
    assert_eq!(added["result"]["account"]["email"], email.as_str());
    assert_eq!(added["result"]["account"]["display_name"], "Me");
    assert_eq!(added["result"]["account"]["sender_name"], "Sender");
    assert_eq!(added["result"]["account"]["provider"], "custom");
    assert_eq!(added["result"]["account"]["auth_type"], "password");
    assert_eq!(added["result"]["account"]["imap_host"], "imap.example.com");
    assert_eq!(added["result"]["account"]["imap_port"], 993);
    assert_eq!(added["result"]["account"]["smtp_host"], "smtp.example.com");
    assert_eq!(added["result"]["account"]["smtp_port"], 587);
    assert_eq!(added["result"]["account"]["tls"], true);
    assert_eq!(added["result"]["account"]["needs_reconnect"], false);

    let listed = invoke_mobile_protocol_json(
        r#"{"id":61,"method":"account.list","params":{}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(listed["id"], 61);
    assert_eq!(listed["result"]["accounts"][0]["id"], email.as_str());
    assert_eq!(listed["result"]["accounts"][0]["needs_reconnect"], false);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_reconnect_keeps_the_account_proxy_and_certificate_pins() {
    let data_dir = unique_data_dir("account-reconnect");
    let dir = data_dir.to_str().unwrap().to_string();
    let unique = unique_test_suffix();
    let email = format!("bridge+{unique}@example.com");
    let add = format!(
        r#"{{"id":70,"method":"account.addPassword","params":{{"email":"{email}","imap_host":"127.0.0.1","imap_port":1143,"smtp_host":"127.0.0.1","smtp_port":1025,"username":"{email}","password":"secret","tls":false,"starttls":true,"smtp_tls":false,"smtp_starttls":true,"cert_pin":"AA11","smtp_cert_pin":"BB22","proxy":{{"mode":"socks5","host":"127.0.0.1","port":9050}}}}}}"#
    );
    let added = invoke_mobile_protocol_json(&add, Some(&dir));
    assert_eq!(added["result"]["account"]["cert_pin"], "aa11", "{added}");
    assert_eq!(added["result"]["account"]["proxy"]["mode"], "socks5");

    // The reconnect: same servers and a fresh password, nothing else.
    let reconnect = format!(
        r#"{{"id":71,"method":"account.addPassword","params":{{"email":"{email}","imap_host":"127.0.0.1","imap_port":1143,"smtp_host":"127.0.0.1","smtp_port":1025,"username":"{email}","password":"new-secret","tls":false,"starttls":true,"smtp_tls":false,"smtp_starttls":true}}}}"#
    );
    let reconnected = invoke_mobile_protocol_json(&reconnect, Some(&dir));
    let account = &reconnected["result"]["account"];
    assert_eq!(account["proxy"]["mode"], "socks5", "{reconnected}");
    assert_eq!(account["proxy"]["port"], 9050, "{reconnected}");
    assert_eq!(account["cert_pin"], "aa11", "{reconnected}");
    assert_eq!(account["smtp_cert_pin"], "bb22", "{reconnected}");

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_honors_explicit_password_account_security_modes() {
    let data_dir = unique_data_dir("account-security");
    let unique = unique_test_suffix();
    let email = format!("self-hosted+{unique}@example.com");
    let request = format!(
        r#"{{"id":62,"method":"account.addPassword","params":{{"email":"{email}","imap_host":"mail.example.com","imap_port":143,"smtp_host":"mail.example.com","smtp_port":2525,"username":"{email}","password":"secret","tls":false,"starttls":true,"smtp_tls":false,"smtp_starttls":false}}}}"#
    );

    let added = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    let account = &added["result"]["account"];
    assert_eq!(account["tls"], false);
    assert_eq!(account["starttls"], true);
    assert_eq!(account["smtp_tls"], false);
    assert_eq!(account["smtp_starttls"], false);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_only_overrides_inference_with_unambiguous_security_modes() {
    let data_dir = unique_data_dir("account-partial-security");
    let unique = unique_test_suffix();
    let email = format!("partial-security+{unique}@example.com");
    let request = format!(
        r#"{{"id":63,"method":"account.addPassword","params":{{"email":"{email}","imap_host":"mail.example.com","imap_port":143,"smtp_host":"mail.example.com","smtp_port":587,"username":"{email}","password":"secret","tls":true,"starttls":false,"smtp_tls":true}}}}"#
    );

    let added = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    let account = &added["result"]["account"];
    assert_eq!(account["tls"], true);
    assert_eq!(account["starttls"], false);
    assert_eq!(account["smtp_tls"], true);
    assert_eq!(account["smtp_starttls"], false);

    let email = format!("partial-starttls+{unique}@example.com");
    let request = format!(
        r#"{{"id":64,"method":"account.addPassword","params":{{"email":"{email}","imap_host":"mail.example.com","imap_port":993,"smtp_host":"mail.example.com","smtp_port":587,"username":"{email}","password":"secret","tls":true,"smtp_starttls":false}}}}"#
    );
    let added = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    let account = &added["result"]["account"];
    assert_eq!(account["smtp_tls"], false);
    assert_eq!(account["smtp_starttls"], true);

    let email = format!("legacy-tls+{unique}@example.com");
    let request = format!(
        r#"{{"id":65,"method":"account.addPassword","params":{{"email":"{email}","imap_host":"mail.example.com","imap_port":143,"smtp_host":"mail.example.com","smtp_port":587,"username":"{email}","password":"secret","tls":true}}}}"#
    );
    let added = invoke_mobile_protocol_json(&request, Some(data_dir.to_str().unwrap()));
    let account = &added["result"]["account"];
    assert_eq!(account["tls"], false);
    assert_eq!(account["starttls"], true);
    assert_eq!(account["smtp_tls"], false);
    assert_eq!(account["smtp_starttls"], true);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_updates_account_settings_and_aliases() {
    let data_dir = unique_data_dir("account-settings");
    let unique = unique_test_suffix();
    let email = format!("settings+{unique}@example.com");
    let data_dir_str = data_dir.to_str().unwrap();
    let add_request = format!(
        r#"{{"id":70,"method":"account.addPassword","params":{{"email":"{email}","display_name":"Old","sender_name":"Old Sender","imap_host":"imap.example.com","smtp_host":"smtp.example.com","username":"{email}","password":"secret"}}}}"#
    );
    assert!(
        invoke_mobile_protocol_json(&add_request, Some(data_dir_str))
            .get("error")
            .is_none()
    );

    let commands = [
        format!(
            r#"{{"id":71,"method":"account.setName","params":{{"id":"{email}","name":"Personal"}}}}"#
        ),
        format!(
            r#"{{"id":72,"method":"account.setSenderName","params":{{"id":"{email}","name":"Sender"}}}}"#
        ),
        format!(
            r#"{{"id":73,"method":"account.setAvatar","params":{{"id":"{email}","avatar_url":"https://example.com/avatar.png"}}}}"#
        ),
        format!(
            r#"{{"id":74,"method":"account.setChatWallpaper","params":{{"id":"{email}","wallpaper":{{"kind":"preset","presetId":"grid"}}}}}}"#
        ),
        format!(
            r#"{{"id":75,"method":"account.setImages","params":{{"id":"{email}","enabled":true}}}}"#
        ),
        format!(
            r#"{{"id":76,"method":"account.setConversationHtml","params":{{"id":"{email}","enabled":false}}}}"#
        ),
        format!(
            r#"{{"id":77,"method":"account.setUnified","params":{{"id":"{email}","enabled":false}}}}"#
        ),
        format!(
            r#"{{"id":78,"method":"account.setMuted","params":{{"id":"{email}","enabled":true}}}}"#
        ),
        format!(
            r#"{{"id":79,"method":"account.setPaused","params":{{"id":"{email}","enabled":true}}}}"#
        ),
        format!(
            r#"{{"id":80,"method":"account.setRSSSyncInterval","params":{{"id":"{email}","minutes":30}}}}"#
        ),
        format!(
            r#"{{"id":81,"method":"account.setAliases","params":{{"id":"{email}","aliases":[{{"email":"alias@example.com","name":"Alias"}},{{"email":"alias@example.com","name":"Duplicate"}},{{"email":"   ","name":"Blank"}}]}}}}"#
        ),
    ];
    for command in commands {
        let value = invoke_mobile_protocol_json(&command, Some(data_dir_str));
        assert!(value.get("error").is_none(), "{value}");
    }

    let listed = invoke_mobile_protocol_json(
        r#"{"id":82,"method":"account.list","params":{}}"#,
        Some(data_dir_str),
    );
    let account = &listed["result"]["accounts"][0];
    assert_eq!(account["display_name"], "Personal", "{listed}");
    assert_eq!(account["sender_name"], "Sender", "{listed}");
    assert_eq!(
        account["avatar_url"], "https://example.com/avatar.png",
        "{listed}"
    );
    assert_eq!(account["chat_wallpaper"]["kind"], "preset", "{listed}");
    assert_eq!(account["chat_wallpaper"]["presetId"], "grid", "{listed}");
    assert_eq!(account["load_remote_images"], true, "{listed}");
    assert_eq!(account["conversation_html"], false, "{listed}");
    assert_eq!(account["included_in_unified"], false, "{listed}");
    assert_eq!(account["muted"], true, "{listed}");
    assert_eq!(account["paused"], true, "{listed}");
    assert_eq!(account["aliases"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(
        account["aliases"][0]["email"], "alias@example.com",
        "{listed}"
    );
    assert_eq!(account["aliases"][0]["name"], "Alias", "{listed}");

    let remove = format!(r#"{{"id":83,"method":"account.remove","params":{{"id":"{email}"}}}}"#);
    assert!(
        invoke_mobile_protocol_json(&remove, Some(data_dir_str))
            .get("error")
            .is_none()
    );
    let empty = invoke_mobile_protocol_json(
        r#"{"id":84,"method":"account.list","params":{}}"#,
        Some(data_dir_str),
    );
    assert_eq!(empty["result"]["accounts"].as_array().unwrap().len(), 0);

    seed_rss_account(&data_dir, "rss-settings", "Feeds");
    let interval = invoke_mobile_protocol_json(
        r#"{"id":85,"method":"account.setRSSSyncInterval","params":{"id":"rss-settings","minutes":30}}"#,
        Some(data_dir_str),
    );
    assert!(interval.get("error").is_none(), "{interval}");
    let rss_listed = invoke_mobile_protocol_json(
        r#"{"id":84,"method":"account.list","params":{}}"#,
        Some(data_dir_str),
    );
    assert_eq!(
        rss_listed["result"]["accounts"][0]["rss_sync_interval_minutes"], 30,
        "{rss_listed}"
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_reorders_accounts() {
    let data_dir = unique_data_dir("account-reorder");
    let data_dir_str = data_dir.to_str().unwrap();
    for email in [
        "first@example.com",
        "second@example.com",
        "third@example.com",
    ] {
        let add_request = format!(
            r#"{{"id":90,"method":"account.addPassword","params":{{"email":"{email}","display_name":"{email}","imap_host":"imap.example.com","smtp_host":"smtp.example.com","username":"{email}","password":"secret"}}}}"#
        );
        assert!(
            invoke_mobile_protocol_json(&add_request, Some(data_dir_str))
                .get("error")
                .is_none()
        );
    }

    let reorder = invoke_mobile_protocol_json(
        r#"{"id":91,"method":"account.reorder","params":{"accounts":["third@example.com","first@example.com","second@example.com"]}}"#,
        Some(data_dir_str),
    );
    assert_eq!(reorder["result"]["ok"], true, "{reorder}");

    let listed = invoke_mobile_protocol_json(
        r#"{"id":92,"method":"account.list","params":{}}"#,
        Some(data_dir_str),
    );
    let accounts = listed["result"]["accounts"].as_array().unwrap();
    let ids: Vec<_> = accounts
        .iter()
        .map(|account| account["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![
            "third@example.com",
            "first@example.com",
            "second@example.com"
        ]
    );

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_appends_new_accounts() {
    let data_dir = unique_data_dir("account-append");
    let data_dir_str = data_dir.to_str().unwrap();
    for (email, name) in [
        ("z-first@example.com", "Z First"),
        ("a-second@example.com", "A Second"),
        ("m-third@example.com", "M Third"),
    ] {
        let add_request = format!(
            r#"{{"id":93,"method":"account.addPassword","params":{{"email":"{email}","display_name":"{name}","imap_host":"imap.example.com","smtp_host":"smtp.example.com","username":"{email}","password":"secret"}}}}"#
        );
        assert!(
            invoke_mobile_protocol_json(&add_request, Some(data_dir_str))
                .get("error")
                .is_none()
        );
    }
    let listed = invoke_mobile_protocol_json(
        r#"{"id":95,"method":"account.list","params":{}}"#,
        Some(data_dir_str),
    );
    let accounts = listed["result"]["accounts"].as_array().unwrap();
    let labels: Vec<_> = accounts
        .iter()
        .map(|account| account["display_name"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["Z First", "A Second", "M Third"]);

    let _ = std::fs::remove_dir_all(data_dir);
}

#[test]
fn mobile_protocol_rejects_invalid_password_account() {
    let data_dir = unique_data_dir("invalid-account");
    let value = invoke_mobile_protocol_json(
        r#"{"id":62,"method":"account.addPassword","params":{"email":"missing-at","imap_host":"imap.example.com","smtp_host":"smtp.example.com","username":"me"}}"#,
        Some(data_dir.to_str().unwrap()),
    );
    assert_eq!(value["id"], 62);
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid email")
    );
    let _ = std::fs::remove_dir_all(data_dir);
}
