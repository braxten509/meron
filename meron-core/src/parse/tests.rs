//! HTML preparation and the parser fixtures shared with adjacent suites.
//! These cases cover which parts of an HTML message may reach its reader.
//! Attachment, envelope and preview behavior have separate suites.

//! Exercise decoding, rendering and attachment caching.
//!
//! Part of the shared message parser used by the IMAP and local mail paths.
//! The facade keeps its existing entry points for both platform clients.

use super::*;

fn lenient_from(raw: &str) -> (String, String) {
    let line = format!("From: {raw}\r\n\r\n");
    let (headers, _) = mailparse::parse_headers(line.as_bytes()).unwrap();
    from_of(&headers)
}

#[test]
fn prepare_html_keeps_the_csp_ahead_of_markup_posing_as_a_head() {
    // `<style>` text and attribute values survive sanitizing verbatim.
    for source in [
        "<style>/*<html><head>*/</style><img src=\"https://t.example/p.png\">",
        "<img alt=\"<html><head>\" src=\"https://t.example/p.png\">",
    ] {
        let out = prepare_html(source, false);
        let head = "<!doctype html><html><head><meta charset=\"utf-8\">\
                        <meta http-equiv=\"Content-Security-Policy\"";
        assert!(out.starts_with(head), "{out}");
        assert_eq!(out.matches("Content-Security-Policy").count(), 1, "{out}");
    }
}

#[test]
fn prepare_html_gates_remote_images_via_csp() {
    let off = prepare_html("<p>hi</p>", false);
    assert!(off.contains("Content-Security-Policy"));
    assert!(off.contains("img-src 'self' data:;"));
    assert!(off.contains("media-src 'self' data: blob:;"));
    assert!(!off.contains("https:"));

    let on = prepare_html("<html><head></head><body>hi</body></html>", true);
    assert!(on.contains("img-src 'self' data: http: https:;"));
    assert!(on.contains("media-src 'self' data: blob: http: https:;"));
}

#[test]
fn prepare_html_strips_script_vectors_keeps_styling() {
    let out = prepare_html(
        "<style>p{color:red}</style>\
             <p style=\"font-weight:bold\" onclick=\"steal()\">hi</p>\
             <script>alert(1)</script>\
             <a href=\"javascript:alert(2)\">x</a>\
             <a href=\"https://example.com\">ok</a>\
             <iframe src=\"https://evil\"></iframe>\
             <form action=\"https://evil\"></form>",
        false,
    );
    // Script-bearing markup is gone.
    assert!(!out.contains("<script"));
    assert!(!out.contains("onclick"));
    assert!(!out.contains("javascript:"));
    assert!(!out.contains("<iframe"));
    assert!(!out.contains("<form"));
    // Styling and safe links survive.
    assert!(out.contains("<style>"));
    assert!(out.contains("color:red"));
    assert!(out.contains("font-weight:bold"));
    assert!(out.contains("https://example.com"));
}

#[test]
fn prepare_html_marks_only_its_own_quotes() {
    // A sender can't fold part of a message by writing the marker itself.
    let forged = prepare_html(r#"<p data-meron-quote="">hidden?</p><p>tail</p>"#, false);
    assert!(!forged.contains("data-meron-quote"), "{forged}");
    let reply = prepare_html(
        r#"<p>Yes</p><div class="gmail_quote">On Mon, Jane wrote:<blockquote>Lunch?</blockquote></div>"#,
        false,
    );
    assert!(
        reply.contains(r#"class="gmail_quote" data-meron-quote="""#),
        "{reply}"
    );
    // Apple Mail / Thunderbird: the `type="cite"` the detector keys on has
    // to survive sanitizing.
    let cite = prepare_html(
        r#"<p>Yes</p><div class="moz-cite-prefix">On 9/1/25 Jane wrote:</div><blockquote type="cite"><p>Lunch?</p></blockquote>"#,
        false,
    );
    assert!(
        cite.contains(r#"<blockquote type="cite" data-meron-quote="">"#),
        "{cite}"
    );
    assert!(
        cite.contains(r#"class="moz-cite-prefix" data-meron-quote="""#),
        "{cite}"
    );
}

#[test]
fn prepare_html_drops_head_title_text() {
    let out = prepare_html(
        "<html><head><title>Secure your schedule</title></head><body><p>hi</p></body></html>",
        false,
    );
    // The subject must not leak into the body as a stray line of text.
    assert!(!out.contains("Secure your schedule"));
    assert!(out.contains("hi"));
}

#[test]
fn prepare_html_hoists_body_colors() {
    let style = prepare_html(
        "<html><body style=\"margin: 0; background-color: #f5f4f2;\"><p>hi</p></body></html>",
        false,
    );
    assert!(style.contains("<meta name=\"meron-body-bg\" content=\"#f5f4f2\">"));

    // A dark canvas travels with the text color that makes it readable.
    let dark = prepare_html(
        "<html><body bgcolor=\"#000000\" text=\"white\"><p>hi</p></body></html>",
        false,
    );
    assert!(dark.contains("<meta name=\"meron-body-bg\" content=\"#000000\">"));
    assert!(dark.contains("<meta name=\"meron-body-fg\" content=\"white\">"));

    // A style declaration outranks the presentational attribute.
    let both = prepare_html(
        "<body bgcolor=\"#ffffff\" style=\"background: #101010\"><p>hi</p></body>",
        false,
    );
    assert!(both.contains("content=\"#101010\""));

    // A `<body>` that only appears inside a comment isn't the document's.
    let commented = prepare_html(
        "<!-- <body bgcolor=\"#123456\"> --><body bgcolor=\"#abcdef\"><p>hi</p></body>",
        false,
    );
    assert!(commented.contains("content=\"#abcdef\""));
    assert!(!commented.contains("#123456"));

    assert!(!prepare_html("<body><p>hi</p></body>", false).contains("meron-body-"));
}

#[test]
fn prepare_html_hoists_colors_not_arbitrary_css() {
    // Only real colors travel: an image layer would become a background-image
    // (and a network fetch) once the frontend interpolates it into CSS.
    for value in [
        "background: url(probe.png)",
        "background: linear-gradient(#fff, #000)",
        "background-color: a&quot;><script>",
        "background-color: expression(alert(1))",
    ] {
        let out = prepare_html(&format!("<body style=\"{value}\"><p>hi</p></body>"), false);
        assert!(!out.contains("meron-body-bg"), "accepted {value}");
    }

    // A second declaration is just another declaration, not an injection.
    let pair = prepare_html(
        "<body style=\"background-color: #fff; color: red\"><p>hi</p></body>",
        false,
    );
    assert!(pair.contains("<meta name=\"meron-body-bg\" content=\"#fff\">"));
    assert!(pair.contains("<meta name=\"meron-body-fg\" content=\"red\">"));

    // `!important` belongs to the declaration, and decides which one wins.
    let important = prepare_html(
        "<body style=\"background: #000000 !important; background: #ffffff; color: white\"><p>hi</p></body>",
        false,
    );
    assert!(important.contains("<meta name=\"meron-body-bg\" content=\"#000000\">"));
    assert!(important.contains("<meta name=\"meron-body-fg\" content=\"white\">"));

    // CSS allows whitespace after the `!`, and a comment anywhere a space is.
    for style in [
        "background: #000000 ! important; background: #ffffff",
        "background: #000000 !IMPORTANT; background: #ffffff",
        "background: #000000 !/* whoops */important; background: #ffffff",
        // Non-ASCII after the `!` must not land mid-code-point.
        "background: #000000; background: #ffffff !ééééé; background: #000000",
        "background: #ffffff; /* background: #ffffff */ background: #000000",
    ] {
        let out = prepare_html(&format!("<body style=\"{style}\"><p>hi</p></body>"), false);
        assert!(
            out.contains("<meta name=\"meron-body-bg\" content=\"#000000\">"),
            "lost the winning declaration in {style}"
        );
    }

    // A trailing layer is not a color, however the value starts.
    for value in [
        "background: rgb(0,0,0) url(probe.png)",
        "background-color: rgb(0 0 0 / url(x))",
        "background: hsl(0, 0%, 0%) no-repeat",
    ] {
        let out = prepare_html(&format!("<body style=\"{value}\"><p>hi</p></body>"), false);
        assert!(!out.contains("meron-body-bg"), "accepted {value}");
    }

    // The last declaration wins, as it does in the cascade: a shorthand that
    // names an image resets an earlier color rather than losing to it.
    let reset = prepare_html(
        "<body style=\"background-color: #fff; background: url(x.png); color: white\"><p>hi</p></body>",
        false,
    );
    assert!(!reset.contains("meron-body-bg"));
    assert!(reset.contains("<meta name=\"meron-body-fg\" content=\"white\">"));

    // Importance travels with the declaration it was written on.
    let important = prepare_html(
        "<body style=\"background: #000 !important; color: white\"><p>hi</p></body>",
        false,
    );
    assert!(
        important.contains("<meta name=\"meron-body-important\" content=\"background-color\">")
    );
    assert!(
        !prepare_html("<body style=\"color: white\"><p>hi</p></body>", false)
            .contains("meron-body-important")
    );

    // `#fff`, `rgb(...)` and a bare keyword are the accepted forms — including
    // the angle units a hue may carry.
    for value in [
        "#fff",
        "#ffffffaa",
        "rgb(255, 0, 0)",
        "white",
        "hsl(180deg 50% 50%)",
        "hsl(0.5turn 50% 50%)",
        "hsl(-90 50% 50%)",
    ] {
        let out = prepare_html(
            &format!("<body style=\"background-color: {value}\"><p>hi</p></body>"),
            false,
        );
        assert!(
            out.contains(&format!("content=\"{value}\"")),
            "dropped {value}"
        );
    }
}

#[test]
fn prepare_html_marks_a_gmail_reply_with_signatures_above_and_below_the_quote() {
    // Gmail's reply with the signature under the quote, which itself holds
    // the quoted message's own signature.
    let out = prepare_html(
        r#"<div dir="ltr">x1</div><br><div class="gmail_quote gmail_quote_container"><div dir="ltr" class="gmail_attr">On Thu, Sep 10, 2026 at 5:11 PM rnons &lt;<a href="mailto:a@b.c">a@b.c</a>&gt; wrote:<br></div><blockquote class="gmail_quote"><div dir="ltr"><div>t2</div><span class="gmail_signature_prefix">-- </span><br><div dir="ltr" class="gmail_signature">2t</div></div></blockquote></div><div><br clear="all"></div><span class="gmail_signature_prefix">-- </span><br><div dir="ltr" class="gmail_signature"><div dir="ltr">s2</div></div>"#,
        false,
    );
    assert!(
        out.contains(r#"<div class="gmail_quote gmail_quote_container" data-meron-quote="">"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<div class="gmail_signature" data-meron-quote="">"#),
        "{out}"
    );
}

#[test]
fn prepare_html_strips_sender_claimed_meron_hooks() {
    // The frames read their own `meron-*` ids and classes back out of the
    // document; a sender must not be able to plant one.
    let out = prepare_html(
        "<style id=\"meron-reader-style\">body{color:#333}</style>\
             <mark class=\"meron-search-hit\">x</mark>\
             <div class=\"card meron-copy-code\">y</div>",
        false,
    );
    assert!(!out.contains("meron-"));
    // Only the hooks are dropped, not the elements or their other markup.
    assert!(out.contains("<style>"));
    assert!(out.contains("<mark>"));
}

#[test]
fn prepare_html_keeps_data_image_but_strips_data_href() {
    let out = prepare_html(
        "<img src=\"data:image/png;base64,AAAA\">\
             <a href=\"data:text/html,<script>alert(1)</script>\">x</a>\
             <img src=\"/media/acct/inbox/1/0.png\">",
        true,
    );
    assert!(out.contains("data:image/png;base64,AAAA"));
    assert!(out.contains("/media/acct/inbox/1/0.png"));
    // The data:text/html navigation vector is dropped, href and all.
    assert!(!out.contains("data:text/html"));
}

mod attachments;
mod envelopes;
mod previews;
