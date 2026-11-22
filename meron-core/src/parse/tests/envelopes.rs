//! Parsing the subject, sender, recipients and available bodies of a message.
//! Fixtures exercise encoded headers, outgoing envelopes and MIME alternatives.
//! Truncated or undecodable parts must still yield the data that can be recovered.

use super::*;

#[test]
fn message_id_reads_the_header_without_the_body() {
    // What waiting for a Sent copy matches on: the angle brackets come off,
    // a folded header still reads, and a message without the header (or one
    // that is not a message at all) yields nothing rather than a guess.
    let raw = b"From: a@example.com\r\nMessage-ID:\r\n <reply-1@meron>\r\nSubject: Re: Lunch\r\n\r\nbody\r\n";
    assert_eq!(message_id_of(raw), "reply-1@meron");
    assert_eq!(message_id_of(b"From: a@example.com\r\n\r\nbody\r\n"), "");
    assert_eq!(message_id_of(b""), "");
}

#[test]
fn delivery_headers_mark_a_message_as_received() {
    // Written here (or filed to Sent by the server): no delivery headers.
    let sent = b"From: me@example.com\r\nSubject: x\r\n\r\nhi";
    assert!(!parse_message(sent, None).delivered);
    // The copy delivered to us carries the receiving MTA's headers, and
    // keeps them wherever it is filed afterwards.
    let received =
        b"Return-Path: <them@example.com>\r\nFrom: them@example.com\r\nSubject: x\r\n\r\nhi";
    assert!(parse_message(received, None).delivered);
    let routed = b"Delivered-To: me@example.com\r\nFrom: me@example.com\r\nSubject: x\r\n\r\nhi";
    assert!(parse_message(routed, None).delivered);
}

#[test]
fn truncated_base64_body_keeps_what_decodes() {
    // "hello world" is `aGVsbG8gd29ybGQ=`; cut mid-quantum it no longer
    // decodes as a whole.
    let raw = b"From: a@b.com\r\nSubject: x\r\nContent-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8gd29y\r\nbGQ\r\n--b--\r\n";
    let message = parse_message(raw, None);
    assert_eq!(message.body, "hello wor");
}

#[test]
fn undecodable_base64_plain_part_falls_back_to_html() {
    let raw = b"From: a@b.com\r\nSubject: x\r\nContent-Type: multipart/alternative; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\naG\r\n--b\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>hello world</p>\r\n--b--\r\n";
    let message = parse_message(raw, None);
    assert!(message.body_is_rendered);
    assert_eq!(message.body, "hello world");
}

#[test]
fn plain_text_keeps_no_html_source() {
    let raw = b"From: a@b.com\r\nSubject: x\r\nContent-Type: text/plain\r\n\r\nhello world";
    let message = parse_message(raw, None);
    assert!(message.body_html.is_none());
    assert_eq!(message.body, "hello world");
}

#[test]
fn markup_in_the_plain_part_renders_as_html() {
    // A sender that stuffs its HTML into text/plain: the real HTML part
    // renders instead, so neither the reader nor the snippet shows tags.
    let raw = b"From: a@b.com\r\n\
Subject: x\r\n\
Content-Type: multipart/alternative; boundary=sep\r\n\
\r\n\
--sep\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
<div style=\"font-family: Arial\"><p>plain markup</p></div>\r\n\
--sep\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p><strong>html part</strong></p>\r\n\
--sep--\r\n";
    let message = parse_message(raw, None);
    assert_eq!(message.body, "**html part**");
    assert_eq!(message.preview, "html part");

    // Prose that opens on a tag example keeps its plain part, tags and all.
    let raw = b"From: a@b.com\r\n\
Subject: x\r\n\
Content-Type: multipart/alternative; boundary=sep\r\n\
\r\n\
--sep\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
<p>hello</p>\r\n\
\r\n\
Please replace <p> with <div> in this example.\r\n\
--sep\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>&lt;p&gt;hello&lt;/p&gt;</p>\r\n\
--sep--\r\n";
    let message = parse_message(raw, None);
    assert!(message.body.starts_with("<p>hello</p>"));
    assert!(message.body.contains("replace <p> with <div>"));

    // A plain-only message is never reclassified, even when it is all tags.
    let raw = b"From: a@b.com\r\nSubject: x\r\nContent-Type: text/plain\r\n\r\n\
<div style=\"font-family: Arial\"><p>hello</p></div>";
    let message = parse_message(raw, None);
    assert!(message.body_html.is_none());
    assert!(message.body.starts_with("<div style="));
}

#[test]
fn plain_markup_needs_tags_to_dominate() {
    assert!(plain_is_markup(
        "<div style=\"font-family: Arial\">\n<p style=\"color: #1a2b3c\">Connect</p>\n</div>"
    ));
    // Tag examples in prose, even when the message ends on one.
    assert!(!plain_is_markup(
        "<p>hello</p>\n\nPlease replace <p> with <div> in this example."
    ));
    assert!(!plain_is_markup(
        "<p>hello</p>\n\nPlease replace <p> with <div>"
    ));
    assert!(!plain_is_markup("hello <b>world</b>"));
}

#[test]
fn multipart_alternative_prefers_plain_body_and_keeps_html() {
    let raw = b"From: a@b.com\r\n\
Subject: x\r\n\
Content-Type: multipart/alternative; boundary=sep\r\n\
\r\n\
--sep\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
plain part\r\n\
--sep\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p><strong>html part</strong></p>\r\n\
--sep--\r\n";
    let message = parse_message(raw, None);
    assert_eq!(message.body, "plain part");
    assert!(
        message
            .body_html
            .as_deref()
            .unwrap_or_default()
            .contains("<strong>html part</strong>")
    );
}

#[test]
fn decodes_rfc2047_subject() {
    // "=?UTF-8?B?SGVsbMO2?=" is base64 for "Hellö".
    assert_eq!(decode_words(" =?UTF-8?B?SGVsbMO2?="), "Hellö");
    assert_eq!(decode_words(" Plain subject"), "Plain subject");
}

#[test]
fn decodes_encoded_words_glued_to_text() {
    // "GyRCJUYlOSVIGyhC" is ISO-2022-JP "テスト"; "Q2Fmw6k=" is UTF-8 "Café".
    assert_eq!(
        lenient_from("Acme=?iso-2022-jp?B?GyRCJUYlOSVIGyhC?= <news@example.com>"),
        ("Acmeテスト".to_string(), "news@example.com".to_string())
    );
    // Adjacent encoded-words join without the whitespace between them, even
    // when the first is glued to text.
    assert_eq!(
        lenient_from("Acme=?UTF-8?Q?Foo?= =?UTF-8?Q?Bar?= <a@example.com>").0,
        "AcmeFooBar"
    );
    assert_eq!(
        lenient_from("=?UTF-8?B?Q2Fmw6k=?= =?UTF-8?Q?_Bar?= Inc <a@example.com>").0,
        "Café Bar Inc"
    );
    // A name whose decoded text looks like an encoded-word is not decoded twice.
    // "PT9VVEYtOD9RP0NFTz89" is base64 for "=?UTF-8?Q?CEO?=".
    assert_eq!(
        lenient_from("=?UTF-8?B?PT9VVEYtOD9RP0NFTz89?= <a@example.com>").0,
        "=?UTF-8?Q?CEO?="
    );
    // Folded across lines.
    assert_eq!(
        lenient_from("Acme=?UTF-8?Q?Foo?=\r\n =?UTF-8?Q?Bar?= <a@example.com>").0,
        "AcmeFooBar"
    );
    // Glued on both sides, and two glued words back to back.
    assert_eq!(
        lenient_from("Acme=?UTF-8?Q?Foo?=Inc <a@example.com>").0,
        "AcmeFooInc"
    );
    assert_eq!(
        lenient_from("=?UTF-8?Q?Foo?==?UTF-8?Q?Bar?= <a@example.com>").0,
        "FooBar"
    );
    // Punctuation inside an encoded-word stays text, not address syntax.
    assert_eq!(
        lenient_from("=?UTF-8?Q?Doe,_Jane?= <jane@example.com>"),
        ("Doe, Jane".to_string(), "jane@example.com".to_string())
    );
    assert_eq!(
        lenient_from("Acme=?UTF-8?Q?_(News)?= <a@example.com>").0,
        "Acme (News)"
    );
    assert_eq!(
        lenient_from("=?UTF-8?Q?Acme_(News)?= <a@example.com>").0,
        "Acme (News)"
    );
    // Glued words inside a quoted name, or a name with a comment.
    assert_eq!(
        lenient_from("\"Acme=?UTF-8?Q?Foo?=\" <a@example.com>").0,
        "AcmeFoo"
    );
    assert_eq!(
        lenient_from("\"Acme \\\"Q\\\"=?UTF-8?Q?Foo?=\" <a@example.com>").0,
        "Acme \"Q\"Foo"
    );
    assert_eq!(
        lenient_from("Acme=?UTF-8?Q?Foo?= (News) <a@example.com>").0,
        "AcmeFoo"
    );
    assert_eq!(
        lenient_from("Acme=?UTF-8?Q?Foo?= (Ne\\)ws) Inc <a@example.com>").0,
        "AcmeFoo  Inc"
    );
    // Non-ASCII text before the encoded-word, glued or not.
    assert_eq!(
        lenient_from("Café=?UTF-8?Q?Foo?= <a@example.com>").0,
        "CaféFoo"
    );
    assert_eq!(
        lenient_from("Café =?UTF-8?Q?Foo?= <a@example.com>").0,
        "Café Foo"
    );
    // Encoded-word lookalikes inside an address are left alone.
    assert_eq!(
        lenient_from("User <user=?UTF-8?Q?foo?=@example.com>"),
        (
            "User".to_string(),
            "user=?UTF-8?Q?foo?=@example.com".to_string()
        )
    );
    assert_eq!(lenient_from("a =? b ?= c <a@example.com>").0, "a =? b ?= c");
    assert_eq!(lenient_from("Plain Name <a@example.com>").0, "Plain Name");
}

#[test]
fn splits_display_name_and_addr() {
    let (name, addr) = split_address("Maya Chen <maya@example.com>");
    assert_eq!(name, "Maya Chen");
    assert_eq!(addr, "maya@example.com");

    let (name, addr) = split_address("bare@example.com");
    assert_eq!(name, "");
    assert_eq!(addr, "bare@example.com");
}

#[test]
fn parses_multipart_message_summary() {
    let raw = b"From: Maya Chen <maya@example.com>\r\n\
Subject: =?UTF-8?B?TGF1bmNoIG5vdGVz?=\r\n\
Date: Mon, 18 May 2026 09:42:00 +0000\r\n\
Content-Type: multipart/alternative; boundary=sep\r\n\
\r\n\
--sep\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
The launch notes are ready for review.\r\n\
--sep\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p><strong>The launch notes</strong> are ready for review.</p>\r\n\
--sep--\r\n";
    let message = parse_message(raw, None);
    assert_eq!(message.subject, "Launch notes");
    assert_eq!(message.from_name, "Maya Chen");
    assert_eq!(message.from_addr, "maya@example.com");
    assert_eq!(message.body, "The launch notes are ready for review.");
    assert!(
        message
            .body_html
            .as_deref()
            .unwrap_or_default()
            .contains("<strong>The launch notes</strong>")
    );
    assert!(message.preview.starts_with("The launch notes are ready"));
}

#[test]
fn reads_the_envelope_of_an_outgoing_message() {
    let raw = concat!(
        "From: Me <Me@Example.com>\r\n",
        "To: \"B\" <b@example.com>, c@example.com\r\n",
        "Cc: A@Example.com\r\n",
        "Bcc: hidden@example.com\r\n",
        "Subject: =?utf-8?q?Re=3A_lunch?=\r\n",
        "Message-ID: <reply-1@meron>\r\n",
        "Date: Tue, 14 Nov 2023 22:13:20 +0000\r\n",
        "\r\n",
        "body\r\n",
    );
    let envelope = sent_envelope_of(raw.as_bytes());
    assert_eq!(envelope.message_id, "reply-1@meron");
    assert_eq!(envelope.subject, "Re: lunch");
    assert_eq!(envelope.from_addr, "Me@Example.com");
    // To + Cc, lowercased and sorted; Bcc stays out — the sending server
    // strips it, so the copy that comes back never carries it.
    assert_eq!(
        envelope.recipients,
        vec![
            "a@example.com".to_string(),
            "b@example.com".to_string(),
            "c@example.com".to_string()
        ]
    );
    assert_eq!(envelope.date, 1_700_000_000);
}

#[test]
fn an_unparseable_message_has_a_blank_envelope() {
    assert_eq!(
        sent_envelope_of(b"\x00not a message"),
        SentEnvelope::default()
    );
}
