//! Decode and normalize sender and recipient addresses.
//!
//! Part of the shared message parser used by the IMAP and local mail paths.
//! The facade keeps its existing entry points for both platform clients.

use super::*;
use base64::Engine as _;

/// Decode a bare RFC 2047 header fragment — one that arrives without its field
/// name, so `MailHeaderMap` cannot be used — by letting
/// `mailparse` parse a synthetic header line — its `get_value()` decodes
/// encoded-words and charsets for us.
pub fn decode_words(raw: &str) -> String {
    let line = format!("Subject:{raw}");
    match parse_header(line.as_bytes()) {
        Ok((header, _)) => header.get_value(),
        Err(_) => raw.to_string(),
    }
}

/// Parse an address header like `mailparse::addrparse_header`, but also decode
/// encoded-words `mailparse` leaves verbatim: ones glued to surrounding text
/// (`Acme=?UTF-8?B?...?=`), which RFC 2047 forbids but some senders emit, and
/// ones after non-ASCII text on the same line, where `mailparse` misreads the
/// preceding character. Display and group names holding an encoded-word are
/// decoded here from the raw value and handed back to `mailparse` as one
/// well-formed encoded-word on a line of its own, so it decodes them once and
/// keeps the decoded text opaque to address syntax.
pub fn addrparse_lenient(header: &MailHeader) -> Option<MailAddrList> {
    let raw = header.get_value_raw();
    let raw = match std::str::from_utf8(raw) {
        Ok(text) => text.to_string(),
        Err(_) => raw.iter().map(|&b| b as char).collect(),
    };
    let rewritten = reencode_names(&raw);
    if rewritten == raw {
        return addrparse_header(header).ok();
    }
    let line = format!("{}: {rewritten}", header.get_key_ref());
    let (rewritten_header, _) = parse_header(line.as_bytes()).ok()?;
    addrparse_header(&rewritten_header).ok()
}

/// Replace each display or group name in a raw address list that holds an
/// encoded-word with its decoded text, re-encoded as a single UTF-8 encoded-word
/// on a folded line of its own. Names with quoted strings or comments, and all
/// addresses, are left as they are.
fn reencode_names(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut segment_start = 0;
    let mut quoted = false;
    let mut comment_depth = 0usize;
    let mut in_angle = false;
    let mut escaped = false;
    let mut skip_to = 0;
    for (i, c) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if i < skip_to {
            continue;
        }
        let top_level = !quoted && comment_depth == 0 && !in_angle;
        if top_level && let Some(len) = encoded_word_at(raw, i) {
            // Punctuation inside an encoded-word is text, not address syntax.
            skip_to = i + len;
            continue;
        }
        match c {
            '\\' if quoted || comment_depth > 0 => escaped = true,
            '"' if comment_depth == 0 && !in_angle => quoted = !quoted,
            _ if quoted => {}
            '(' if !in_angle => comment_depth += 1,
            ')' if comment_depth > 0 => comment_depth -= 1,
            _ if comment_depth > 0 => {}
            '>' if in_angle => {
                in_angle = false;
                out.push_str(&raw[segment_start..=i]);
                segment_start = i + 1;
            }
            _ if in_angle => {}
            '<' | ':' => {
                if c == '<' {
                    in_angle = true;
                }
                out.push_str(&reencode_name(&raw[segment_start..i]));
                out.push(c);
                segment_start = i + 1;
            }
            ',' | ';' => {
                out.push_str(&raw[segment_start..=i]);
                segment_start = i + 1;
            }
            _ => {}
        }
    }
    out.push_str(&raw[segment_start..]);
    out
}

fn reencode_name(name: &str) -> String {
    let flat = flatten_name(name);
    let has_word = flat
        .match_indices("=?")
        .any(|(i, _)| encoded_word_len(&flat[i..]).is_some());
    if !has_word {
        return name.to_string();
    }
    let decoded = decode_words_lenient(flat.trim());
    if decoded.is_empty() {
        return name.to_string();
    }
    format!("\r\n =?UTF-8?B?{}?= ", STANDARD.encode(decoded))
}

/// A raw display name as `mailparse` reads it before decoding: folds joined,
/// comments dropped (the whitespace around them kept), quoted strings unwrapped
/// with their escapes resolved. Encoded-words are copied through whole, so
/// punctuation inside one stays text.
fn flatten_name(name: &str) -> String {
    let unfolded = name
        .lines()
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = String::with_capacity(unfolded.len());
    let mut quoted = false;
    let mut comment_depth = 0usize;
    let mut chars = unfolded.char_indices();
    while let Some((i, c)) = chars.next() {
        if comment_depth == 0
            && let Some(len) = encoded_word_at(&unfolded, i)
        {
            out.push_str(&unfolded[i..i + len]);
            while chars.offset() < i + len {
                chars.next();
            }
            continue;
        }
        match c {
            '\\' if quoted || comment_depth > 0 => {
                if let Some((_, next)) = chars.next()
                    && comment_depth == 0
                {
                    out.push(next);
                }
            }
            '"' if comment_depth == 0 => quoted = !quoted,
            '(' if !quoted => comment_depth += 1,
            ')' if comment_depth > 0 => comment_depth -= 1,
            _ if comment_depth > 0 => {}
            _ => out.push(c),
        }
    }
    out
}

/// Byte length of an encoded-word starting at byte `i` of `text`, if one does.
fn encoded_word_at(text: &str, i: usize) -> Option<usize> {
    text[i..]
        .starts_with("=?")
        .then(|| encoded_word_len(&text[i..]))
        .flatten()
}

/// Decode every well-formed encoded-word in `text`, wherever it sits, dropping
/// whitespace between adjacent encoded-words as RFC 2047 requires.
fn decode_words_lenient(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    let mut after_word = false;
    while let Some(start) = rest.find("=?") {
        let decoded = encoded_word_len(&rest[start..]).and_then(|len| {
            let word = &rest[start..start + len];
            let decoded = decode_words(&format!(" {word}"));
            (decoded != word).then_some((len, decoded))
        });
        let Some((len, decoded)) = decoded else {
            out.push_str(&rest[..start + 2]);
            rest = &rest[start + 2..];
            after_word = false;
            continue;
        };
        let gap = &rest[..start];
        if !(after_word && gap.trim().is_empty()) {
            out.push_str(gap);
        }
        out.push_str(&decoded);
        rest = &rest[start + len..];
        after_word = true;
    }
    out.push_str(rest);
    out
}

/// Byte length of the `=?charset?enc?text?=` encoded-word at the start of `s`.
fn encoded_word_len(s: &str) -> Option<usize> {
    let mut parts = s[2..].splitn(3, '?');
    let charset = parts.next()?;
    let enc = parts.next()?;
    let tail = parts.next()?;
    if charset.is_empty() || !matches!(enc, "B" | "b" | "Q" | "q") {
        return None;
    }
    let end = tail.find("?=")?;
    if tail[..end].contains(char::is_whitespace) {
        return None;
    }
    Some(2 + charset.len() + 1 + enc.len() + 1 + end + 2)
}

/// Split a `From`-style value ("Display Name <addr@host>") into name and address.
pub fn split_address(raw: &str) -> (String, String) {
    if let Ok(list) = addrparse(raw)
        && let Some(MailAddr::Single(info)) = list.first()
    {
        let name = info.display_name.clone().unwrap_or_default();
        return (name, info.addr.clone());
    }
    (String::new(), raw.to_string())
}

/// The first `From` address of `headers` as (name, address).
pub(super) fn from_of(headers: &[MailHeader]) -> (String, String) {
    let Some(header) = headers.get_first_header("From") else {
        return (String::new(), String::new());
    };
    if let Some(list) = addrparse_lenient(header)
        && let Some(MailAddr::Single(info)) = list.first()
    {
        return (
            info.display_name.clone().unwrap_or_default(),
            info.addr.clone(),
        );
    }
    (String::new(), header.get_value())
}
