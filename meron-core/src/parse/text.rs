//! Convert message HTML to compact readable text.
//!
//! Part of the shared message parser used by the IMAP and local mail paths.
//! The facade keeps its existing entry points for both platform clients.

use super::*;

pub fn html_to_text(html: &str) -> String {
    let options = ConversionOptions::builder()
        .extract_metadata(false)
        .skip_images(false)
        // Render bare links as `[text](url)` rather than `<url>` autolinks — the
        // reader's link tokenizer handles the former, but leaves the angle
        // brackets of the latter as stray text.
        .autolinks(false)
        .strip_tags(vec!["style".to_string(), "script".to_string()])
        .build();
    let clean_html = flatten_tables(&strip_invisible_html(html));
    let mut markdown = convert(&clean_html, Some(options.clone()))
        .ok()
        .and_then(|result| result.content)
        .unwrap_or_default();
    if markdown.trim().is_empty()
        && let Some(body) = body_inner_html(&clean_html)
    {
        markdown = convert(&format!("<div>{body}</div>"), Some(options))
            .ok()
            .and_then(|result| result.content)
            .unwrap_or_default();
    }
    normalize_text(&compact_image_links(&strip_heading_markers(
        &rewrite_image_markdown(&markdown),
    )))
}

fn is_whitespace_or_invisible(c: char) -> bool {
    c.is_whitespace()
        || c == '\u{200b}' // zero-width space
        || c == '\u{200c}' // zero-width non-joiner
        || c == '\u{200d}' // zero-width joiner
        || c == '\u{feff}' // zero-width no-break space
}

pub(super) fn normalize_text(text: &str) -> String {
    let mut final_lines = Vec::new();
    let mut consecutive_empty = 0;

    for line in text.lines() {
        let trimmed = line.trim_end_matches(is_whitespace_or_invisible);
        if trimmed.is_empty() {
            consecutive_empty += 1;
            if consecutive_empty <= 1 {
                final_lines.push("");
            }
        } else {
            consecutive_empty = 0;
            final_lines.push(trimmed);
        }
    }

    // Trim leading/trailing empty lines
    let mut start = 0;
    while start < final_lines.len() && final_lines[start].is_empty() {
        start += 1;
    }
    let mut end = final_lines.len();
    while end > start && final_lines[end - 1].is_empty() {
        end -= 1;
    }

    final_lines[start..end].join("\n").trim().to_string()
}

fn rewrite_image_markdown(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(start) = rest.find("![") {
        out.push_str(&rest[..start]);
        let after_marker = &rest[start + 2..];
        let Some(alt_end) = after_marker.find("](") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let alt = &after_marker[..alt_end];
        let after_alt = &after_marker[alt_end + 2..];
        let Some(url_end) = after_alt.find(')') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let label = alt.trim();
        // Drop decorative/icon images (no alt text) entirely — e.g. GitHub's
        // issue-type icon or tracking pixels. Keeping them produced noise, and
        // when they sit inside an anchor (`[![](icon) text](url)`) they broke the
        // surrounding link. Named images become a plain `[image: alt]` marker
        // (Gmail's plain-text convention) rather than a link — we don't auto-load
        // remote images, and inline ones already show as thumbnails below.
        let tail = &after_alt[url_end + 1..];
        if label.is_empty() {
            if out.ends_with('[') && tail.starts_with("](") {
                // This is an empty image that is the sole content of a link: `[![](src)](url)`.
                // We want to drop the entire link since it has no readable text.
                out.pop(); // Pop the leading `[`
                if let Some(anchor_url_end) = tail[2..].find(')') {
                    rest = &tail[2 + anchor_url_end + 1..];
                } else {
                    rest = tail;
                }
            } else {
                // Normal decorative/icon image: just drop it.
                // Also swallow a single trailing space the icon left behind, so a
                // dropped leading icon doesn't open its link with `[ text`.
                rest = tail.strip_prefix(' ').unwrap_or(tail);
            }
        } else if out.ends_with('[') && tail.starts_with("](") {
            // Linked image (`[![alt](src)](url)`): the image is the sole content
            // of an anchor (e.g. a logo that links somewhere). Emit the marker as
            // the link's text — the surrounding `[` ... `](url)` already wrap it —
            // so the reader renders a clickable `[image: alt](url)` link instead
            // of dropping the destination.
            out.push_str(&format!("image: {label}"));
            rest = tail;
        } else {
            out.push_str(&format!("[image: {label}]"));
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

fn strip_heading_markers(markdown: &str) -> String {
    markdown
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                trimmed.trim_start_matches('#').trim_start()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn compact_image_links(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut previous_image_link = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        let image_link = trimmed.starts_with("[image: ");
        if image_link && previous_image_link {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(trimmed);
        } else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line);
        }
        previous_image_link = image_link;
    }
    out
}

fn strip_invisible_html(html: &str) -> String {
    strip_html_block(&strip_html_block(html, "style"), "script")
}

/// Neutralize HTML table layout so the markdown converter doesn't render it as a markdown table.
///
/// Emails (Google, newsletters, Outlook) use tables purely for layout; rendering them as tables
/// crams whole paragraphs into single cells, strips block structure (text runs together), and emits
/// `| --- |` noise the reader can't render. Cells/rows become `<div>` to keep block separation;
/// table containers are dropped. Tag-only rewrite — inner content is preserved.
fn flatten_tables(html: &str) -> String {
    const DROP: &[&str] = &[
        "table", "thead", "tbody", "tfoot", "colgroup", "col", "caption",
    ];
    const BLOCK: &[&str] = &["tr", "td", "th"];

    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let after = &rest[lt..];
        let Some(gt) = after.find('>') else {
            out.push_str(after);
            return out;
        };
        let inner = &after[1..gt];
        let closing = inner.trim_start().starts_with('/');
        let name: String = inner
            .trim_start()
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();

        if DROP.contains(&name.as_str()) {
            // drop the tag, keep content
        } else if BLOCK.contains(&name.as_str()) {
            out.push_str(if closing { "</div>" } else { "<div>" });
        } else {
            out.push_str(&after[..=gt]);
        }
        rest = &after[gt + 1..];
    }
    out.push_str(rest);
    out
}

fn body_inner_html(html: &str) -> Option<&str> {
    let lower = html.to_ascii_lowercase();
    let body_start = lower.find("<body")?;
    let open_end = lower[body_start..].find('>')? + body_start + 1;
    let close_start = lower[open_end..].find("</body>")? + open_end;
    Some(&html[open_end..close_start])
}

fn strip_html_block(html: &str, tag: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let open_pattern = format!("<{tag}");
    let close_pattern = format!("</{tag}>");
    loop {
        let lower = rest.to_ascii_lowercase();
        let Some(start) = lower.find(&open_pattern) else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        let Some(end_rel) = lower[start..].find(&close_pattern) else {
            return out;
        };
        let end = start + end_rel + close_pattern.len();
        rest = &rest[end..];
    }
}
