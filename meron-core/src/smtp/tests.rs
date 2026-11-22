//! Regression tests for smtp.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::{as_smtp_cert_error, bare_addr, bare_id, build_message, parse_addrs, split_name_addr};

/// A send and a sync fail the same way inside rustls; only the marker tells
/// the UI which server to show and which pin to write. Without the re-tag a
/// refused submission server would pin the IMAP server instead.
#[test]
fn certificate_failures_from_submission_carry_the_smtp_marker() {
    let refused = crate::tls::UntrustedCertificate::from_io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        rustls::Error::InvalidCertificate(rustls::CertificateError::Other(rustls::OtherError(
            std::sync::Arc::new(std::io::Error::other("CaUsedAsEndEntity")),
        ))),
    ));
    let tagged = as_smtp_cert_error(refused).to_string();
    assert!(
        tagged.starts_with(crate::tls::UNTRUSTED_SMTP_CERT_MARKER),
        "{tagged}"
    );
    assert!(tagged.contains("CaUsedAsEndEntity"), "{tagged}");

    // Anything else is passed through untouched.
    let offline = anyhow::anyhow!("tcp connect: connection refused");
    assert_eq!(
        as_smtp_cert_error(offline).to_string(),
        "tcp connect: connection refused"
    );
}

#[test]
fn parse_addrs_splits_and_trims() {
    assert_eq!(
        parse_addrs("a@x.com, Bob <b@y.com> ,, c@z.com"),
        vec!["a@x.com", "Bob <b@y.com>", "c@z.com"]
    );
    assert!(parse_addrs("").is_empty());
}

#[test]
fn parse_addrs_keeps_quoted_and_bracketed_commas() {
    assert_eq!(
        parse_addrs("\"Doe, Jane\" <j@x.com>, a@y.com"),
        vec!["\"Doe, Jane\" <j@x.com>", "a@y.com"]
    );
    // Unterminated quote: the rest of the string stays one entry rather
    // than producing bogus half-recipients.
    assert_eq!(
        parse_addrs("\"Doe, Jane <j@x.com>"),
        vec!["\"Doe, Jane <j@x.com>"]
    );
}

#[test]
fn envelope_recipients_dedupes_case_insensitively() {
    let to = vec!["Alice <a@x.com>".to_string(), "b@y.com".to_string()];
    let cc = vec!["A@X.COM".to_string()];
    let bcc = vec!["b@y.com".to_string(), "c@z.com".to_string()];
    assert_eq!(
        super::envelope_recipients(&to, &cc, &bcc),
        vec!["a@x.com", "b@y.com", "c@z.com"]
    );
}

#[test]
fn build_message_rejects_undecodable_attachment() {
    let atts = vec![super::AttachmentInput {
        filename: "broken.bin".into(),
        mime: "application/octet-stream".into(),
        data: "not!!valid@@base64".into(),
        inline_id: String::new(),
    }];
    let error = build_message(
        "Alice",
        "alice@x.com",
        "bob@y.com",
        "",
        "",
        false,
        "Subject",
        "body",
        "",
        &atts,
        "",
        "",
        "",
        "",
    )
    .expect_err("must fail instead of sending without the attachment");
    assert!(error.to_string().contains("broken.bin"), "{error:#}");
}

#[test]
fn build_message_addresses_quoted_comma_recipient() {
    let raw = build_message(
        "Alice",
        "alice@x.com",
        "\"Doe, Jane\" <j@x.com>",
        "",
        "",
        false,
        "Subject",
        "body",
        "",
        &[],
        "",
        "",
        "",
        "",
    )
    .expect("build_message");
    let raw = String::from_utf8(raw).expect("utf8");
    assert!(raw.contains("<j@x.com>"), "{raw}");
    // The display name must stay attached to the one recipient, not split
    // into a second bogus address.
    assert!(!raw.contains("<Doe>"), "{raw}");
}

#[test]
fn split_name_addr_handles_named_quoted_and_bare() {
    assert_eq!(
        split_name_addr("Alice Example <a@x.com>"),
        ("Alice Example".to_string(), "a@x.com".to_string())
    );
    assert_eq!(
        split_name_addr("\"Quoted Name\" <a@x.com>"),
        ("Quoted Name".to_string(), "a@x.com".to_string())
    );
    assert_eq!(
        split_name_addr(" a@x.com "),
        (String::new(), "a@x.com".to_string())
    );
    assert_eq!(
        split_name_addr("\"Doe, Jane\" <j@x.com>"),
        ("Doe, Jane".to_string(), "j@x.com".to_string())
    );
    assert_eq!(
        split_name_addr("\"Ada \\\"Lovelace\\\"\" <a@x.com>"),
        ("Ada \"Lovelace\"".to_string(), "a@x.com".to_string())
    );
}

#[test]
fn bare_addr_strips_display_name() {
    assert_eq!(bare_addr("Alice <a@x.com>"), "a@x.com");
    assert_eq!(bare_addr("a@x.com"), "a@x.com");
}

#[test]
fn bare_id_strips_angle_brackets() {
    assert_eq!(bare_id("<id@host>"), "id@host");
    assert_eq!(bare_id(" id@host "), "id@host");
}

fn build(message_id: &str, bcc: &str, include_bcc: bool) -> String {
    let raw = build_message(
        "Alice",
        "alice@x.com",
        "bob@y.com",
        "",
        bcc,
        include_bcc,
        "Subject",
        "body",
        "",
        &[],
        "parent@x.com",
        "root@x.com parent@x.com",
        "",
        message_id,
    )
    .expect("build_message");
    String::from_utf8(raw).expect("utf8")
}

#[test]
fn build_message_emits_single_bracketed_message_id() {
    // Bare and pre-bracketed ids must both come out as a single <id@host>.
    for input in ["itest@x.com", "<itest@x.com>"] {
        let raw = build(input, "", false);
        assert!(raw.contains("Message-ID: <itest@x.com>"), "raw: {raw}");
        assert!(!raw.contains("<<"), "double-bracketed Message-ID in: {raw}");
    }
}

#[test]
fn build_message_threads_via_reply_headers() {
    let raw = build("", "", false);
    assert!(raw.contains("In-Reply-To: <parent@x.com>"), "raw: {raw}");
    assert!(
        raw.contains("References: <root@x.com> <parent@x.com>"),
        "raw: {raw}"
    );
}

#[test]
fn build_message_keeps_bcc_only_for_kept_copies() {
    let kept = build("", "secret@z.com", true);
    assert!(
        kept.contains("secret@z.com"),
        "kept copy must carry Bcc: {kept}"
    );
    let wire = build("", "secret@z.com", false);
    assert!(
        !wire.contains("secret@z.com"),
        "wire copy must not leak Bcc: {wire}"
    );
}

#[test]
fn inline_image_send_parses_back_with_media_refs() {
    // Full send-side/receive-side roundtrip for composer inline images:
    // the built MIME's `cid:` refs must come back from parse_message with
    // every `cid:` rewritten to the served `/media/<key>` path, so a
    // message sent from Meron renders its inline images in Meron too.
    use base64::Engine as _;
    let atts = vec![
        super::AttachmentInput {
            filename: "pasted-image-1.png".into(),
            mime: "image/png".into(),
            data: base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]),
            inline_id: "meron-image-1784002518563-a1b2c3d@meron".into(),
        },
        super::AttachmentInput {
            filename: "pasted-image-2.png".into(),
            mime: "image/png".into(),
            data: base64::engine::general_purpose::STANDARD.encode([9u8, 8, 7]),
            inline_id: "meron-image-1784002609378-x9y8z7w@meron".into(),
        },
    ];
    let html = r#"<p>one</p><img src="cid:meron-image-1784002518563-a1b2c3d@meron" alt="pasted-image-1.png"><p>two</p><img src="cid:meron-image-1784002609378-x9y8z7w@meron" alt="pasted-image-2.png"><p>bye</p>"#;
    let raw = build_message(
        "Alice",
        "alice@x.com",
        "bob@y.com",
        "",
        "",
        false,
        "Subject",
        "plain fallback",
        html,
        &atts,
        "",
        "",
        "",
        "",
    )
    .expect("build_message");

    let root = std::env::temp_dir().join(format!("meron-inline-roundtrip-{}", std::process::id()));
    let context = crate::parse::MediaCtx {
        root: root.clone(),
        account: "acct".into(),
        folder: "Sent".into(),
        uid: 1,
    };
    let parsed_message = crate::parse::parse_message(&raw, Some(&context));
    let _ = std::fs::remove_dir_all(&root);

    let html_out = parsed_message.body_html.expect("html kept");
    assert!(
        !html_out.contains("cid:"),
        "unrewritten cid ref: {html_out}"
    );
    assert!(html_out.contains("/media/acct/Sent/1/0.png"), "{html_out}");
    assert!(html_out.contains("/media/acct/Sent/1/1.png"), "{html_out}");
    assert_eq!(parsed_message.attachments.len(), 2);
}
