//! Validation of a backup envelope before restore.
//! Tests supply versions, encryption parameters, passphrases and malformed input.
//! Errors must remain useful to callers deciding whether to request a passphrase.

use super::*;

#[test]
fn encrypted_backup_without_a_passphrase_asks_for_one() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    let text = export(&source, true, Some("pw"), Host::default(), &|_| {
        secrets_with("hunter2")
    })
    .unwrap();

    let error = parse_error(&text, None);
    assert!(needs_passphrase(&error), "{error}");

    // A blank passphrase is the same as none, so the UI re-prompts.
    let error = parse_error(&text, Some(""));
    assert!(needs_passphrase(&error), "{error}");
}

#[test]
fn wrong_passphrase_is_reported_as_such() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    let text = export(&source, true, Some("pw"), Host::default(), &|_| {
        secrets_with("hunter2")
    })
    .unwrap();

    let error = parse_error(&text, Some("not-pw"));
    assert!(error.contains("wrong passphrase"), "{error}");
    // "Wrong passphrase" must not send the UI back to the prompt-for-one path.
    assert!(!needs_passphrase(&error));
}

#[test]
fn tampering_with_the_ciphertext_fails_authentication() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    let text = export(&source, true, Some("pw"), Host::default(), &|_| {
        secrets_with("hunter2")
    })
    .unwrap();

    let mut envelope: Value = serde_json::from_str(&text).unwrap();
    let ciphertext = envelope["ciphertext"].as_str().unwrap().to_string();
    let mut raw = STANDARD.decode(&ciphertext).unwrap();
    raw[0] ^= 0x01;
    envelope["ciphertext"] = json!(STANDARD.encode(&raw));

    let error = parse_error(&envelope.to_string(), Some("pw"));
    assert!(error.contains("wrong passphrase"), "{error}");
}

#[test]
fn a_passphrase_on_a_plaintext_backup_is_ignored() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    let text = export(&source, false, None, Host::default(), &no_secrets).unwrap();

    let data = parse(&text, Some("unnecessary")).unwrap();
    assert_eq!(data.accounts.len(), 1);
}

#[test]
fn non_backup_files_are_rejected() {
    for text in ["", "not json", "{}", r#"{"hello":"world"}"#, "[]"] {
        let error = parse_error(text, None);
        assert!(
            error.contains("not a Meron backup file") || error.contains("no data"),
            "{text:?} gave {error}"
        );
    }
}

#[test]
fn a_newer_format_version_is_refused() {
    let text = json!({ "meron_backup": FORMAT_VERSION + 1, "encrypted": false, "data": {} });
    let error = parse_error(&text.to_string(), None);
    assert!(error.contains("newer version of Meron"), "{error}");
}

/// The iteration count is read from the envelope before the passphrase can be
/// checked, so an absurd one must be refused rather than worked through.
#[test]
fn an_absurd_iteration_count_is_refused_instead_of_performed() {
    for iterations in [
        json!(MAX_KDF_ITERATIONS + 1),
        json!(u64::MAX),
        json!(4_000_000_000u64),
        // 2^32 + our own count: a truncating cast would read this as normal.
        json!(4_294_967_296u64 + KDF_ITERATIONS as u64),
        json!(0),
    ] {
        let text = json!({
            "meron_backup": FORMAT_VERSION,
            "encrypted": true,
            "cipher": "aes-256-gcm",
            "kdf": { "salt": STANDARD.encode([0u8; SALT_LEN]), "iterations": iterations },
            "nonce": STANDARD.encode([0u8; NONCE_LEN]),
            "ciphertext": STANDARD.encode([0u8; 32]),
        });
        let error = parse_error(&text.to_string(), Some("pw"));
        assert!(
            error.contains("unreasonable amount of work"),
            "{iterations}: {error}"
        );
    }
}

#[test]
fn a_malformed_salt_is_refused() {
    for salt in [vec![0u8; 0], vec![0u8; 8], vec![0u8; 64]] {
        let text = json!({
            "meron_backup": FORMAT_VERSION,
            "encrypted": true,
            "cipher": "aes-256-gcm",
            "kdf": { "salt": STANDARD.encode(&salt), "iterations": KDF_ITERATIONS },
            "nonce": STANDARD.encode([0u8; NONCE_LEN]),
            "ciphertext": STANDARD.encode([0u8; 32]),
        });
        let error = parse_error(&text.to_string(), Some("pw"));
        assert!(
            error.contains("malformed `kdf.salt`"),
            "{} bytes: {error}",
            salt.len()
        );
    }
}

/// The count we actually write has to survive the bound that rejects the rest.
#[test]
fn our_own_iteration_count_still_opens() {
    let source = test_conn();
    insert_mail_account(&source, "ada@example.com", sample_config(), json!({}));
    let text = export(&source, true, Some("pw"), Host::default(), &|_| {
        secrets_with("hunter2")
    })
    .unwrap();

    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["kdf"]["iterations"],
        KDF_ITERATIONS
    );
    let data = parse(&text, Some("pw")).unwrap();
    assert_eq!(data.accounts.len(), 1);
}

#[test]
fn an_encrypted_backup_with_an_unknown_cipher_is_refused() {
    let text = json!({
        "meron_backup": FORMAT_VERSION,
        "encrypted": true,
        "cipher": "rot13",
        "kdf": { "salt": STANDARD.encode([0u8; SALT_LEN]) },
        "nonce": STANDARD.encode([0u8; NONCE_LEN]),
        "ciphertext": STANDARD.encode([0u8; 32]),
    });
    let error = parse_error(&text.to_string(), Some("pw"));
    assert!(error.contains("unsupported backup cipher"), "{error}");
}
