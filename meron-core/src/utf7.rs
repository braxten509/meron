//! Modified UTF-7 for IMAP mailbox names (RFC 3501 §5.1.3).
//!
//! Servers that do not negotiate UTF8=ACCEPT report and accept mailbox names in this encoding, so a
//! folder called `gds-ää-envoyés` arrives over the wire as `gds-&AOQA5A--envoy&AOk-s`. The wire
//! form stays the canonical name everywhere (store rows, message `folder` columns, saved Kanban
//! columns, SELECT/CREATE); only what the user reads is decoded.

use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;

/// Decode a mailbox name for display. Names with no `&` — the overwhelming
/// majority — are returned as-is, and anything malformed falls back to the raw
/// name rather than erroring: a mangled label beats a missing folder.
pub fn decode(name: &str) -> String {
    if !name.contains('&') {
        return name.to_string();
    }
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(end) = after.find('-') else {
            // Unterminated shift: not modified UTF-7, keep the raw name.
            return name.to_string();
        };
        if end == 0 {
            out.push('&');
        } else {
            match decode_run(&after[..end]) {
                Some(text) => out.push_str(&text),
                None => return name.to_string(),
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Encode a mailbox name for the wire. ASCII-only names without `&` are
/// unchanged, so this is a no-op for the common case.
pub fn encode(name: &str) -> String {
    if name.is_ascii() && !name.contains('&') {
        return name.to_string();
    }
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();
    for ch in name.chars() {
        if ch == '&' {
            flush(&mut pending, &mut out);
            out.push_str("&-");
        } else if matches!(ch, ' '..='~') {
            flush(&mut pending, &mut out);
            out.push(ch);
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(ch.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// Decode one `&…-` run: modified BASE64 (`,` for `/`, no padding) over UTF-16BE.
fn decode_run(run: &str) -> Option<String> {
    let standard: String = run
        .chars()
        .map(|ch| if ch == ',' { '/' } else { ch })
        .collect();
    let bytes = STANDARD_NO_PAD.decode(standard).ok()?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

fn flush(pending: &mut Vec<u16>, out: &mut String) {
    if pending.is_empty() {
        return;
    }
    let mut bytes = Vec::with_capacity(pending.len() * 2);
    for unit in pending.drain(..) {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    let encoded = STANDARD_NO_PAD.encode(bytes).replace('/', ",");
    out.push('&');
    out.push_str(&encoded);
    out.push('-');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_round_trip_untouched() {
        for name in ["INBOX", "Archives/2021", "Sent Items", ""] {
            assert_eq!(decode(name), name);
            assert_eq!(encode(name), name);
        }
    }

    #[test]
    fn decodes_outlook_style_names() {
        // A shifted run ends at the first `-`, so the literal hyphen that follows
        // is doubled on the wire.
        assert_eq!(decode("gds-&AOQA5A--envoy&AOk-s"), "gds-ää-envoyés");
        assert_eq!(decode("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode("INBOX/&ZeVnLIqe-"), "INBOX/日本語");
    }

    #[test]
    fn literal_ampersand_is_shift_terminated() {
        assert_eq!(decode("R&-D"), "R&D");
        assert_eq!(encode("R&D"), "R&-D");
    }

    #[test]
    fn round_trips_non_ascii() {
        for name in ["gds-ää-envoyés", "日本語", "Reçus/Été 2026", "a&b ü"] {
            assert_eq!(decode(&encode(name)), name);
        }
    }

    #[test]
    fn encodes_astral_plane_as_surrogate_pair() {
        let encoded = encode("📮");
        assert!(
            encoded.starts_with('&') && encoded.ends_with('-'),
            "{encoded}"
        );
        assert_eq!(decode(&encoded), "📮");
    }

    #[test]
    fn malformed_input_falls_back_to_the_raw_name() {
        // Unterminated shift, and base64 that is not valid UTF-16.
        assert_eq!(decode("&AOQ"), "&AOQ");
        assert_eq!(decode("&####-"), "&####-");
    }
}
