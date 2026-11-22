//! Attachment storage and recovery during message parsing.
//! Fixtures include embedded images, calendar alternatives and failed writes.
//! The attachment summary distinguishes visible files from inline content.

use super::*;

#[test]
fn writes_inline_image_to_disk_and_keys_it() {
    // "AQID" is base64 for the bytes [1, 2, 3].
    let raw = b"From: a@b.com\r\n\
Subject: x\r\n\
Content-Type: multipart/mixed; boundary=sep\r\n\
\r\n\
--sep\r\n\
Content-Type: text/html\r\n\
\r\n\
<p>hi</p>\r\n\
--sep\r\n\
Content-Type: image/png\r\n\
Content-Transfer-Encoding: base64\r\n\
Content-Disposition: inline; filename=\"a.png\"\r\n\
\r\n\
AQID\r\n\
--sep--\r\n";
    let root = std::env::temp_dir().join(format!("meron-media-test-{}", std::process::id()));
    let media_context = MediaCtx {
        root: root.clone(),
        account: "acct@host".to_string(),
        folder: "[Gmail]/All Mail".to_string(),
        uid: 42,
    };
    let message = parse_message(raw, Some(&media_context));
    assert_eq!(message.attachments.len(), 1);
    let key = message.attachments[0].key.as_deref().expect("image keyed");
    // Unsafe path chars in account/folder are flattened to `_`.
    assert_eq!(key, "acct_host/_Gmail__All_Mail/42/0.png");
    let bytes = std::fs::read(root.join(key)).expect("file written");
    assert_eq!(bytes, vec![1, 2, 3]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rewrites_cid_refs_in_html_source_to_media_keys() {
    let raw = b"From: a@b.com\r\n\
Subject: x\r\n\
Content-Type: multipart/related; boundary=sep\r\n\
\r\n\
--sep\r\n\
Content-Type: text/html\r\n\
\r\n\
<p><img src=\"cid:logo123\"></p>\r\n\
--sep\r\n\
Content-Type: image/png\r\n\
Content-Transfer-Encoding: base64\r\n\
Content-ID: <logo123>\r\n\
Content-Disposition: inline; filename=\"a.png\"\r\n\
\r\n\
AQID\r\n\
--sep--\r\n";
    let root = std::env::temp_dir().join(format!("meron-cid-test-{}", std::process::id()));
    let media_context = MediaCtx {
        root: root.clone(),
        account: "acct".to_string(),
        folder: "inbox".to_string(),
        uid: 7,
    };
    let message = parse_message(raw, Some(&media_context));
    let html = message.body_html.as_deref().expect("html kept");
    assert!(html.contains("/media/acct/inbox/7/0.png"));
    assert!(!html.contains("cid:logo123"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cached_media_available_requires_keyed_files() {
    let root = std::env::temp_dir().join(format!(
        "meron-media-availability-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("acct/inbox/1")).unwrap();
    std::fs::write(root.join("acct/inbox/1/0.png"), [1, 2, 3]).unwrap();

    let mut message = Message::default();
    message.attachments.push(Attachment {
        filename: "a.png".to_string(),
        mime: "image/png".to_string(),
        size: 3,
        key: Some("acct/inbox/1/0.png".to_string()),
        oversized: false,
    });
    assert!(cached_media_available(&root, &message));

    std::fs::remove_file(root.join("acct/inbox/1/0.png")).unwrap();
    assert!(!cached_media_available(&root, &message));
    assert_eq!(missing_media_count(&root, &message), 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn has_attachments_ignores_images_the_html_shows_inline() {
    let attachment = |key: &str| Attachment {
        filename: "x".to_string(),
        mime: "image/png".to_string(),
        size: 3,
        key: Some(key.to_string()),
        oversized: false,
    };
    let mut message = Message {
        body_html: Some("<img src=\"/media/acct/inbox/1/0.png\">".to_string()),
        ..Default::default()
    };
    assert!(!message.has_attachments());

    message.attachments.push(attachment("acct/inbox/1/0.png"));
    assert!(
        !message.has_attachments(),
        "a cid: logo is not an attachment"
    );

    message.attachments.push(attachment("acct/inbox/1/1.pdf"));
    assert!(message.has_attachments());

    // Plain-text mail has no HTML to show anything inline.
    let mut plain = Message::default();
    plain.attachments.push(attachment("acct/inbox/2/0.png"));
    assert!(plain.has_attachments());
}

#[test]
fn named_calendar_alternative_is_not_a_duplicate_attachment() {
    let raw = "Content-Type: multipart/mixed; boundary=outer

--outer
Content-Type: multipart/alternative; boundary=inner

--inner
Content-Type: text/plain; name=body.txt

Invitation
--inner
Content-Type: text/calendar; name=invite.ics
Content-Disposition: inline; filename=invite.ics

BEGIN:VCALENDAR
END:VCALENDAR
--inner--
--outer
Content-Type: text/calendar; name=invite.ics
Content-Disposition: attachment; filename=invite.ics

BEGIN:VCALENDAR
END:VCALENDAR
--outer--
"
    .replace('\n', "\r\n");
    let message = parse_message(raw.as_bytes(), None);
    let files: Vec<_> = message.file_attachments().collect();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].filename, "invite.ics");
    assert_eq!(files[0].mime, "text/calendar");
}

#[test]
fn a_failed_write_is_recovered_and_an_oversized_file_is_not_written() {
    let root = std::env::temp_dir().join(format!("meron-unwritable-{}", std::process::id()));
    let _ = std::fs::remove_file(&root);
    std::fs::write(&root, b"not a directory").unwrap();
    let media_context = MediaCtx {
        root: root.clone(),
        account: "acc".into(),
        folder: "INBOX".into(),
        uid: 1,
    };
    let raw = b"Message-ID: <one@test>\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=file.bin\r\n\r\ndata";
    let message = parse_message(raw, Some(&media_context));
    assert_eq!(message.attachments.len(), 1);
    assert_eq!(message.attachments[0].key, None);
    assert!(!message.attachments[0].oversized);
    assert_eq!(missing_media_count(&root, &message), 1);
    std::fs::remove_file(&root).unwrap();
    assert_eq!(
        write_media_with_cap(
            &media_context,
            0,
            "file.bin",
            "application/octet-stream",
            b"data",
            3
        ),
        None
    );
    assert!(!root.exists());
}

#[test]
fn keyless_attachments_trigger_media_recovery_unless_oversized() {
    // A row cached before attachments were persisted has a key-less file;
    // it must report unavailable so the caller refetches and writes it.
    // One left off disk for its size would only be left off again.
    let root = std::env::temp_dir().join(format!("meron-keyless-{}", std::process::id()));
    let mut message = Message::default();
    message.attachments.push(Attachment {
        filename: "invite.ics".to_string(),
        mime: "text/calendar".to_string(),
        size: 3072,
        key: None,
        oversized: false,
    });
    assert!(!cached_media_available(&root, &message));
    assert_eq!(missing_media_count(&root, &message), 1);
    message.attachments[0].oversized = true;
    assert!(cached_media_available(&root, &message));
    assert_eq!(missing_media_count(&root, &message), 0);
}
