//! Message envelopes and bodies in the local cache.
//! Body updates must preserve header dates, recipients and message IDs.
//! The invalidation case covers discarding a cached body.

use super::*;

#[test]
fn save_cached_message_preserves_envelope_recipients_in_json() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    let header = MessageHeader {
        uid: 42,
        subject: "Hi".into(),
        from_name: "Sender".into(),
        from_addr: "sender@example.com".into(),
        date: 1_779_580_800, // 2026-05-24
        to: vec![crate::imap::Recipient {
            name: "Me".into(),
            addr: "me@example.com".into(),
        }],
        ..Default::default()
    };
    upsert_messages(&conn, "acct", "INBOX", &[header]).unwrap();

    save_cached_message(
        &conn,
        "acct",
        "INBOX",
        42,
        &Message {
            subject: "Hi".into(),
            from_name: "Sender".into(),
            from_addr: "sender@example.com".into(),
            body: "hello".into(),
            ..Default::default()
        },
    )
    .unwrap();

    let contacts = suggest_contacts(&conn, "acct", "me@", 10).unwrap();
    assert_eq!(
        contacts.iter().map(|c| c.addr.as_str()).collect::<Vec<_>>(),
        vec!["me@example.com"]
    );
    let message = get_cached_message(&conn, "acct", "INBOX", 42)
        .unwrap()
        .unwrap();
    assert_eq!(message.to, "Me <me@example.com>");
}

#[test]
fn save_cached_message_keeps_the_header_date_when_the_body_has_none() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    let header = MessageHeader {
        uid: 7,
        subject: "No Date header".into(),
        date: 1_790_645_280, // the sync's INTERNALDATE fallback
        ..Default::default()
    };
    upsert_messages(&conn, "acct", "INBOX", &[header]).unwrap();

    let body = Message {
        subject: "No Date header".into(),
        body: "hello".into(),
        ..Default::default()
    };
    save_cached_message(&conn, "acct", "INBOX", 7, &body).unwrap();
    let message = get_cached_message(&conn, "acct", "INBOX", 7)
        .unwrap()
        .unwrap();
    assert_eq!(message.date, 1_790_645_280);

    // A body that does carry a date still wins.
    let dated = Message {
        date: 1_790_600_000,
        ..body
    };
    save_cached_message(&conn, "acct", "INBOX", 7, &dated).unwrap();
    let message = get_cached_message(&conn, "acct", "INBOX", 7)
        .unwrap()
        .unwrap();
    assert_eq!(message.date, 1_790_600_000);
}

#[test]
fn body_cache_invalidation_works() {
    let conn = test_conn();

    // Save an HTML-only message (body_is_rendered = true)
    let html = "<p>rendered plain</p>";
    let msg_html = Message {
        subject: "HTML only".into(),
        body: "stale render".into(),
        body_html: Some(html.into()),
        body_is_rendered: true,
        ..Default::default()
    };
    save_cached_message(&conn, "acct", "INBOX", 101, &msg_html).unwrap();

    // Save a plain-text message (body_is_rendered = false)
    let msg_plain = Message {
        subject: "Plain only".into(),
        body: "original plain".into(),
        body_is_rendered: false,
        ..Default::default()
    };
    save_cached_message(&conn, "acct", "INBOX", 102, &msg_plain).unwrap();

    // Verify they are initially saved with their bodies
    let retrieved_html = get_cached_message(&conn, "acct", "INBOX", 101)
        .unwrap()
        .unwrap();
    assert_eq!(retrieved_html.body, "stale render");
    assert!(retrieved_html.body_is_rendered);

    let retrieved_plain = get_cached_message(&conn, "acct", "INBOX", 102)
        .unwrap()
        .unwrap();
    assert_eq!(retrieved_plain.body, "original plain");
    assert!(!retrieved_plain.body_is_rendered);

    // Force cache invalidation by setting the meta version in DB to an older one
    db::meta_set(&conn, "body_cache_version", "old_version").unwrap();

    // Call the invalidator
    db::invalidate_body_cache_if_needed(&conn).unwrap();

    // The html-only body is re-rendered in place (not nulled), so the FTS
    // `body` column stays populated across the bump.
    let html_body_in_db: Option<String> = conn
        .query_row("SELECT body FROM messages WHERE uid = 101", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(html_body_in_db, Some(crate::parse::render_body(html)));

    // The plain-only body is left untouched.
    let plain_body_in_db: Option<String> = conn
        .query_row("SELECT body FROM messages WHERE uid = 102", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(plain_body_in_db, Some("original plain".to_string()));

    // FTS still matches the re-rendered html-only message after the bump.
    let hits = search_messages(&conn, "acct", "INBOX", "rendered", 10, None).unwrap();
    assert!(hits.iter().any(|m| m.subject == "HTML only"));
}

#[test]
fn upsert_messages_preserves_message_id_casing_in_json() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    // The stored ids feed reply In-Reply-To/References headers verbatim, and
    // receivers (Gmail, GitHub) match Message-IDs case-sensitively — a synced
    // row must never lowercase them.
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[MessageHeader {
            uid: 1,
            subject: "Wont download videos".into(),
            date: 100,
            thread_key: "nonbili/NouTube/issues/253@github.com".into(),
            message_id: "nonbili/NouTube/issues/253@github.com".into(),
            ..Default::default()
        }],
    )
    .unwrap();

    let (message_id, thread_key): (String, String) = conn
        .query_row(
            "SELECT json_extract(json, '$.message_id'), thread_key FROM messages WHERE uid = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(message_id, "nonbili/NouTube/issues/253@github.com");
    assert_eq!(thread_key, "nonbili/NouTube/issues/253@github.com");
}
