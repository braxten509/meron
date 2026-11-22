//! Read MIME body alternatives and decode their text.
//!
//! Part of the shared message parser used by the IMAP and local mail paths.
//! The facade keeps its existing entry points for both platform clients.

use super::*;

#[derive(Default)]
pub(super) struct BodySources {
    pub(super) plain: Option<String>,
    pub(super) html: Option<String>,
}

/// Pick body sources, walking multipart trees. Multipart/alternative usually
/// carries both text/plain and text/html; the plain part is the source for the
/// conversation/plain reader, and HTML is kept for the HTML reader.
pub(super) fn body_sources(part: &mailparse::ParsedMail) -> BodySources {
    let mut plain = find_text_part(part, "text/plain");
    let html = find_text_part(part, "text/html");
    // Some senders put their HTML into the text/plain alternative as well.
    // Shown as text it is a wall of tags (and a notification snippet reading
    // `<div style=...`), so fall back to rendering the real HTML part. Only
    // then: that part is what the HTML tab shows, so nothing is lost, while a
    // plain-only message is always kept as the sender wrote it.
    if html.is_some() && plain.as_deref().is_some_and(plain_is_markup) {
        plain = None;
    }
    if plain.is_some() || html.is_some() {
        return BodySources { plain, html };
    }

    let ctype = part.ctype.mimetype.to_ascii_lowercase();
    if part.subparts.is_empty() && ctype.starts_with("text/") {
        let content = text_body(part).unwrap_or_default();
        if content.contains("<html")
            || content.contains("<body")
            || content.contains("<p>")
            || content.contains("</div>")
        {
            return BodySources {
                plain: None,
                html: Some(content),
            };
        }
        return BodySources {
            plain: Some(content),
            html: None,
        };
    }
    BodySources::default()
}

/// Whether a text/plain part is really an HTML document: it opens and closes
/// on a tag, and tags make up most of it. Prose that quotes tags as examples
/// (`<p>hello</p>` then an explanation) is mostly text and stays plain.
pub(super) fn plain_is_markup(plain: &str) -> bool {
    let trimmed = plain.trim();
    if !trimmed.starts_with('<') || !trimmed.ends_with('>') {
        return false;
    }
    let (mut markup, mut total) = (0usize, 0usize);
    let mut in_tag = false;
    for c in trimmed.chars() {
        if c == '<' {
            in_tag = true;
        }
        if !c.is_whitespace() {
            total += 1;
            if in_tag {
                markup += 1;
            }
        }
        if c == '>' {
            in_tag = false;
        }
    }
    markup * 2 >= total
}

fn find_text_part(part: &ParsedMail, mime: &str) -> Option<String> {
    if part.ctype.mimetype.eq_ignore_ascii_case(mime) {
        return text_body(part);
    }
    for sub in &part.subparts {
        if let Some(body) = find_text_part(sub, mime) {
            return Some(body);
        }
    }
    None
}

/// Decoded text of a leaf part. A base64 body cut off mid-quantum (a truncated
/// message, a sender that drops the padding) fails to decode as a whole, which
/// would leave the reader with no body at all; decode the complete quanta
/// instead and show what did arrive.
pub(super) fn text_body(part: &ParsedMail) -> Option<String> {
    if let Ok(body) = part.get_body() {
        return Some(body);
    }
    let Body::Base64(encoded) = part.get_body_encoded() else {
        return None;
    };
    let mut cleaned: Vec<u8> = encoded
        .get_raw()
        .iter()
        .copied()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    cleaned.truncate(cleaned.len() / 4 * 4);
    // Nothing complete arrived: leave the part out so an HTML alternative can
    // stand in for it.
    if cleaned.is_empty() {
        return None;
    }
    // Re-parse as a part of its own so `mailparse` applies the charset.
    let mut raw = format!(
        "Content-Type: {}; charset={}\r\nContent-Transfer-Encoding: base64\r\n\r\n",
        part.ctype.mimetype, part.ctype.charset
    )
    .into_bytes();
    raw.extend_from_slice(&cleaned);
    parse_mail(&raw).ok()?.get_body().ok()
}
