//! RSS entries parsed from feed markup.
//! Fixtures exercise stable identifiers, discovery links, inline media and text extraction.
//! These cases define what the feed reader can recover before storing an entry.

use super::*;

#[test]
fn item_identity_prefers_guid_then_link_then_fallback() {
    let with_guid = parse_item(
        &entry("guid-1", "https://example.com/a", "A", 1700000000),
        1,
    );
    let changed_link = parse_item(
        &entry("guid-1", "https://example.com/changed", "A", 1700000000),
        1,
    );
    assert_eq!(
        with_guid.as_ref().unwrap().item_key,
        changed_link.as_ref().unwrap().item_key,
        "GUID should be stable across link changes"
    );

    let link_only = parse_item(&entry("", "https://example.com/a", "A", 0), 1);
    let same_link = parse_item(&entry("", "https://example.com/a", "Different", 0), 1);
    assert_eq!(
        link_only.as_ref().unwrap().item_key,
        same_link.as_ref().unwrap().item_key,
        "link should be stable when GUID is absent"
    );

    let fallback = parse_item(&entry("", "", "A", 1700000000), 1);
    assert!(!fallback.unwrap().item_key.is_empty());
}

#[test]
fn normalize_adds_scheme_and_requires_host() {
    assert_eq!(
        normalize_feed_url("example.com/feed").unwrap(),
        "https://example.com/feed"
    );
    assert!(normalize_feed_url("   ").is_err());
    assert!(normalize_feed_url("ftp://example.com").is_err());
}

#[test]
fn discovers_feed_link_from_html_head() {
    let html = r#"<html><head><link rel="alternate" type="application/rss+xml" href="/feed.xml"></head><body></body></html>"#;
    assert_eq!(
        discover_feed_url("https://example.com/", html).as_deref(),
        Some("https://example.com/feed.xml")
    );
}

#[test]
fn sniff_image_ext_recognizes_images_and_rejects_error_bodies() {
    assert_eq!(
        sniff_image_ext(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]),
        Some("jpg")
    );
    assert_eq!(sniff_image_ext(b"\x89PNG\r\n\x1a\n....."), Some("png"));
    assert_eq!(sniff_image_ext(b"GIF89a....."), Some("gif"));
    assert_eq!(
        sniff_image_ext(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
        Some("webp")
    );
    assert_eq!(sniff_image_ext(b"<svg xmlns=\"...\">"), Some("svg"));
    // The HTTP-200 text bodies that previously poisoned the cache.
    assert_eq!(sniff_image_ext(b"URL signature expired"), None);
    assert_eq!(sniff_image_ext(b""), None);
}

#[test]
fn clean_feed_text_strips_html() {
    assert_eq!(
        clean_feed_text("<p>Hello <strong>world</strong>.</p>"),
        "Hello world."
    );
    assert_eq!(
        clean_feed_text("<p>Line 1</p><p>Line 2</p>"),
        "Line 1\nLine 2"
    );
    assert_eq!(clean_feed_text("Hello<br>world"), "Hello\nworld");
}

#[test]
fn extract_image_urls_collects_dedupes_and_skips_data_uris() {
    let html = r#"<p>caption</p>
            <img src="https://cdn.example.com/a/1.jpg">
            <img src="https://cdn.example.com/a/2.png">
            <img src="https://cdn.example.com/a/1.jpg">
            <img src="data:image/png;base64,AAAA">
            <img src="https://cdn.example.com/open/rss" width="1" height="1">
            <img src="https://cdn.example.com/open/rss2" width="0" height="0">
            <img>"#;
    assert_eq!(
        extract_image_urls(html),
        vec![
            "https://cdn.example.com/a/1.jpg".to_string(),
            "https://cdn.example.com/a/2.png".to_string(),
        ]
    );
    assert!(extract_image_urls("  ").is_empty());
}

#[test]
fn extract_video_urls_reads_video_and_nested_source_elements() {
    let html = r#"<p>caption</p>
            <video controls><source src="https://cdn.example.com/v/1.mp4"></video>
            <video src="https://cdn.example.com/v/2.webm" controls></video>
            <video><source src="https://cdn.example.com/v/1.mp4"></video>
            <video><source src="data:video/mp4;base64,AAAA"></video>"#;
    assert_eq!(
        extract_video_urls(html),
        vec![
            "https://cdn.example.com/v/1.mp4".to_string(),
            "https://cdn.example.com/v/2.webm".to_string(),
        ]
    );
    assert!(extract_video_urls("  ").is_empty());
}

#[test]
fn first_line_collapses_whitespace_and_truncates() {
    assert_eq!(first_line("  hello   world \n again "), "hello world again");
    let long = "x".repeat(300);
    let preview = first_line(&long);
    assert_eq!(preview.chars().count(), 223); // 220 chars + "..."
    assert!(preview.ends_with("..."));
}

#[test]
fn mime_guesses_from_extension() {
    assert_eq!(image_mime("https://x/a.PNG?v=2"), "image/png");
    assert_eq!(image_mime("https://x/a.webp"), "image/webp");
    assert_eq!(image_mime("https://x/a.bin"), "image/jpeg"); // default
    assert_eq!(video_mime("https://x/a.webm"), "video/webm");
    assert_eq!(video_mime("https://x/a"), "video/mp4"); // default
}
