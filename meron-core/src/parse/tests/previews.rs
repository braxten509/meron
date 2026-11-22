//! Message previews produced from plain text and HTML.
//! The fixtures include formatted links, layout tables, decorative images and escaped delimiters.
//! The preview must stay readable when a long body is truncated.

use super::*;

#[test]
fn preview_drops_markup_noise() {
    // Gmail's text/plain conventions, as sent by the Play Console.
    let body = "Your update is live\n\n[image: Google Play Console Logo] <https://play.google.com/console/>\n\nHello, Your update to Meron is live in the store.\n\n[image: Icon of your app Meron] *Meron*";
    assert_eq!(
        preview_of(body),
        "Your update is live Hello, Your update to Meron is live in the store. Meron"
    );
}

#[test]
fn preview_keeps_link_text_and_plain_punctuation() {
    let body = "# Heading\n\n---\n\n> quoted\n- See [the docs](https://example.com) and `code`.\nWrite to <bob@example.com>. 3 < 4 and [Attachment: a.pdf]";
    assert_eq!(
        preview_of(body),
        "Heading quoted See the docs and code. Write to bob@example.com. 3 < 4 and [Attachment: a.pdf]"
    );
}

#[test]
fn preview_keeps_unpaired_emphasis_and_nested_link_parens() {
    let body = "2 * 3 and *.rs stay; a lone ` too.\nSee [docs](https://example.com/Foo_(bar)) and *this* **too**.";
    assert_eq!(
        preview_of(body),
        "2 * 3 and *.rs stay; a lone ` too. See docs and this too."
    );
}

#[test]
fn preview_leaves_escaped_delimiters_alone() {
    let body = "Use \\*literal\\* and \\`ticks\\`, but \\\\*this* is markup.";
    assert_eq!(
        preview_of(body),
        "Use \\*literal\\* and \\`ticks\\`, but \\\\this is markup."
    );
}

#[test]
fn preview_truncates_long_bodies() {
    let body = "word ".repeat(100);
    let preview = preview_of(&body);
    assert_eq!(preview.chars().count(), 201);
    assert!(preview.ends_with('…'));
}

#[test]
fn converts_html_to_clean_text() {
    let html = "<html><head><style>body { color: red; }</style></head><body><h1>Hello World</h1><p>This is a <a href=\"https://example.com\">link</a>.<br>New line &amp; &lt;escaped&gt;.</p><div>Another block</div></body></html>";
    let text = html_to_text(html);
    assert_eq!(
        text,
        "Hello World\n\nThis is a [link](https://example.com).\nNew line & <escaped>.\n\nAnother block"
    );
}

#[test]
fn converts_html_images_to_image_markers() {
    let html = r#"<p><strong>Alice Example</strong> created an issue <a href="https://example.com/i">(example/project#266)</a></p><p><img alt="Screenshot.png" src="https://example.com/Screenshot.png"></p>"#;
    let text = html_to_text(html);
    assert_eq!(
        text,
        "**Alice Example** created an issue [(example/project#266)](https://example.com/i)\n\n[image: Screenshot.png]"
    );
}

#[test]
fn keeps_link_on_named_linked_image() {
    // A named image that is the sole content of an anchor (e.g. a Google logo
    // linking to a destination) keeps its link as `[image: alt](url)`.
    let html = r#"<p><a href="https://google.com"><img alt="Google" src="cid:logo"></a></p>"#;
    let text = html_to_text(html);
    assert_eq!(text, "[image: Google](https://google.com)");
}

#[test]
fn drops_decorative_icon_images() {
    // Empty-alt image (e.g. GitHub's issue-type icon) is dropped, leaving the
    // surrounding link intact rather than a stray "Image (view on web)".
    let html = r#"<p><a href="https://x/i"><img alt="" src="https://x/icon.png"> <strong>Alice Example</strong> created an issue (example/project#266)</a></p>"#;
    let text = html_to_text(html);
    assert_eq!(
        text,
        "[**Alice Example** created an issue (example/project#266)](https://x/i)"
    );
}

#[test]
fn captures_html_source_for_rerender() {
    let raw =
        b"From: a@b.com\r\nSubject: x\r\nContent-Type: text/html\r\n\r\n<p><strong>hi</strong></p>";
    let message = parse_message(raw, None);
    let html = message.body_html.as_deref().expect("html kept");
    assert!(html.contains("<strong>hi</strong>"));
    // Re-rendering the stored HTML reproduces the body.
    assert_eq!(render_body(html), message.body);
    assert_eq!(message.body, "**hi**");
}

#[test]
fn strips_style_blocks_with_attributes() {
    let html = "<html><head><style type=\"text/css\">body { color: red; }</style></head><body>Hello World</body></html>";
    let text = html_to_text(html);
    assert_eq!(text, "Hello World");
}

#[test]
fn flattens_layout_tables_into_blocks() {
    // Email layout tables must not become markdown tables: paragraphs in a
    // cell stay separate blocks, and there are no `|`/`---` table artifacts.
    let html = "<table><tr><td><p>First paragraph.</p><p>Second paragraph.</p></td></tr></table>";
    let text = html_to_text(html);
    assert_eq!(text, "First paragraph.\n\nSecond paragraph.");
    assert!(!text.contains('|'));
}

#[test]
fn normalizes_whitespace_and_thin_spaces() {
    let input = "ZHANG XINGYAO\n\n\u{200a}\n\nAmount\n\n6,383.25 USD\n\n\u{200b}\n\nDate";
    let normalized = normalize_text(input);
    assert_eq!(
        normalized,
        "ZHANG XINGYAO\n\nAmount\n\n6,383.25 USD\n\nDate"
    );
}

#[test]
fn drops_empty_linked_images() {
    // A linked image with no alt text (e.g. social media icon links in Airwallex emails)
    // should have the entire empty link dropped rather than leaving a stray `[](url)`.
    let html = r#"<p><a href="https://facebook.com/airwallex"><img alt="" src="https://example.com/Facebook.png"></a></p>"#;
    let text = html_to_text(html);
    assert_eq!(text, "");
}
