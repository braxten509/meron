//! Backup and restore through the mobile protocol.
//! The plaintext fixture covers settings restored as mobile preferences.
//! The encrypted backup also carries account secrets.

use super::*;

#[test]
fn mobile_protocol_round_trips_a_plaintext_backup() {
    let source_directory = unique_data_directory("backup-source");
    let unique = unique_test_suffix();
    let email = format!("backup+{unique}@example.com");
    let source = source_directory.to_str().unwrap();
    let add = format!(
        r#"{{"id":80,"method":"account.addPassword","params":{{"email":"{email}","display_name":"Work","sender_name":"Ada","imap_host":"imap.example.com","imap_port":993,"smtp_host":"smtp.example.com","smtp_port":465,"username":"{email}","password":"pa55w0rd-plain","tls":true}}}}"#
    );
    assert!(
        invoke_mobile_protocol_json(&add, Some(source))
            .get("error")
            .is_none()
    );

    let exported = invoke_mobile_protocol_json(
        r#"{"id":81,"method":"backup.export","params":{}}"#,
        Some(source),
    );
    assert!(exported.get("error").is_none(), "{exported}");
    let text = exported["result"]["backup"].as_str().unwrap().to_string();
    // Without secrets the password never reaches the file.
    assert!(!text.contains("pa55w0rd-plain"), "{text}");

    // Restore into a second, empty data dir.
    let target_directory = unique_data_directory("backup-target");
    let target = target_directory.to_str().unwrap();
    let import = json!({
        "id": 82,
        "method": "backup.import",
        "params": { "backup": text },
    });
    let imported = invoke_mobile_protocol_json(&import.to_string(), Some(target));
    assert!(imported.get("error").is_none(), "{imported}");
    assert_eq!(imported["result"]["accounts"], 1);
    assert_eq!(imported["result"]["skipped"], 0);
    assert_eq!(imported["result"]["secrets"], 0);

    let listed = invoke_mobile_protocol_json(
        r#"{"id":83,"method":"account.list","params":{}}"#,
        Some(target),
    );
    assert_eq!(listed["result"]["accounts"][0]["id"], email.as_str());
    assert_eq!(
        listed["result"]["accounts"][0]["imap_host"],
        "imap.example.com"
    );
    // No secret came across, so the restored account has to be reconnected.
    assert_eq!(listed["result"]["accounts"][0]["needs_reconnect"], true);

    let _ = std::fs::remove_dir_all(source_directory);
    let _ = std::fs::remove_dir_all(target_directory);
}

#[test]
fn mobile_settings_restored_from_a_backup_are_readable_as_prefs() {
    let source_directory = unique_data_directory("mobile-prefs-backup-source");
    let source = source_directory.to_str().unwrap();
    let set = json!({
        "id": 1,
        "method": "app.prefsSet",
        "params": { "key": "mobile.app.app_language_v1", "value": "ja" },
    });
    assert!(
        invoke_mobile_protocol_json(&set.to_string(), Some(source))
            .get("error")
            .is_none()
    );

    let exported = invoke_mobile_protocol_json(
        r#"{"id":2,"method":"backup.export","params":{}}"#,
        Some(source),
    );
    let text = exported["result"]["backup"].as_str().unwrap().to_string();

    let target_directory = unique_data_directory("mobile-prefs-backup-target");
    let target = target_directory.to_str().unwrap();
    let import = json!({ "id": 3, "method": "backup.import", "params": { "backup": text } });
    let imported = invoke_mobile_protocol_json(&import.to_string(), Some(target));
    assert!(imported.get("error").is_none(), "{imported}");

    let get = json!({
        "id": 4,
        "method": "app.prefsGet",
        "params": { "keys": ["mobile.app.app_language_v1"] },
    });
    let value = invoke_mobile_protocol_json(&get.to_string(), Some(target));
    assert_eq!(value["result"]["prefs"]["mobile.app.app_language_v1"], "ja");

    let _ = std::fs::remove_dir_all(source_directory);
    let _ = std::fs::remove_dir_all(target_directory);
}

#[test]
fn mobile_protocol_round_trips_an_encrypted_backup_with_secrets() {
    let source_directory = unique_data_directory("backup-enc-source");
    let unique = unique_test_suffix();
    let email = format!("enc+{unique}@example.com");
    let source = source_directory.to_str().unwrap();
    let add = format!(
        r#"{{"id":84,"method":"account.addPassword","params":{{"email":"{email}","display_name":"Work","imap_host":"imap.example.com","smtp_host":"smtp.example.com","username":"{email}","password":"hunter2"}}}}"#
    );
    assert!(
        invoke_mobile_protocol_json(&add, Some(source))
            .get("error")
            .is_none()
    );

    let export = json!({
        "id": 85,
        "method": "backup.export",
        "params": { "include_secrets": true, "passphrase": "correct horse" },
    });
    let exported = invoke_mobile_protocol_json(&export.to_string(), Some(source));
    assert!(exported.get("error").is_none(), "{exported}");
    let text = exported["result"]["backup"].as_str().unwrap().to_string();
    assert!(!text.contains("hunter2"), "{text}");

    let target_directory = unique_data_directory("backup-enc-target");
    let target = target_directory.to_str().unwrap();

    // No passphrase: a prompt request, not a failure.
    let bare = json!({ "id": 86, "method": "backup.import", "params": { "backup": text } });
    let value = invoke_mobile_protocol_json(&bare.to_string(), Some(target));
    assert!(value.get("error").is_none(), "{value}");
    assert_eq!(value["result"]["needs_passphrase"], true);

    // Wrong passphrase: a real error.
    let wrong = json!({
        "id": 87,
        "method": "backup.import",
        "params": { "backup": text, "passphrase": "nope" },
    });
    let value = invoke_mobile_protocol_json(&wrong.to_string(), Some(target));
    let message = value["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains("wrong passphrase"), "{value}");

    // Right passphrase: the secret comes back, so the account is usable.
    let right = json!({
        "id": 88,
        "method": "backup.import",
        "params": { "backup": text, "passphrase": "correct horse" },
    });
    let value = invoke_mobile_protocol_json(&right.to_string(), Some(target));
    assert!(value.get("error").is_none(), "{value}");
    assert_eq!(value["result"]["accounts"], 1);
    assert_eq!(value["result"]["secrets"], 1);

    let listed = invoke_mobile_protocol_json(
        r#"{"id":89,"method":"account.list","params":{}}"#,
        Some(target),
    );
    assert_eq!(listed["result"]["accounts"][0]["needs_reconnect"], false);

    let _ = std::fs::remove_dir_all(source_directory);
    let _ = std::fs::remove_dir_all(target_directory);
}
