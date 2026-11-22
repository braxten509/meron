//! Header and attachment parsing for the message list.
//!
//! Converts envelope, header and body-structure data for the folder listing.
//! Resolves thread identifiers and attachment metadata before a body is downloaded.

use super::*;

/// Message fields parsed out of the fetched RFC822 header. A named struct
/// rather than a tuple because there are enough of them (and the recipient
/// lists) that positional destructuring at the call sites would be error-prone.
#[derive(Default)]
pub(super) struct HeaderFields {
    pub(super) subject: String,
    pub(super) from_name: String,
    pub(super) from_addr: String,
    pub(super) date: i64,
    pub(super) message_id: String,
    pub(super) in_reply_to: String,
    /// First id of the `References` header, i.e. the thread root.
    pub(super) references_root: String,
    pub(super) to: Vec<Recipient>,
    pub(super) cc: Vec<Recipient>,
}

/// Everything the message list needs, parsed from `RFC822.HEADER`.
///
/// The IMAP `ENVELOPE` carries the same fields, but imap-proto rejects any quoted string holding a byte outside
/// `0x01..=0x7F`, so a single message whose server emitted a raw 8-bit display name (rather than a literal or an RFC 2047
/// encoded-word) fails to parse. async-imap leaves that response in its buffer unconsumed, wedging the connection, and the
/// next sync refetches the same range and wedges again — one message stalls the whole folder forever.
///
/// The header arrives as a literal, copied out by length with no parsing, so it cannot hit that. It is also
/// what we already trusted over the envelope for the subject: imap-proto hands back quoted-string contents
/// without unescaping, so an envelope subject containing double quotes arrived as `\"...\"` and, being part
/// of the compound thread key, split the conversation into a duplicate thread.
pub(super) fn header_fields(header: &[u8]) -> HeaderFields {
    let Ok((headers, _)) = mailparse::parse_headers(header) else {
        return HeaderFields::default();
    };
    let headers = headers.as_slice();
    let mut from = header_addrs(headers, "From");
    let (from_name, from_addr) = if from.is_empty() {
        (String::new(), String::new())
    } else {
        let first = from.remove(0);
        (first.name, first.addr)
    };
    HeaderFields {
        subject: headers.get_first_value("Subject").unwrap_or_default(),
        from_name,
        from_addr,
        date: headers
            .get_first_value("Date")
            .map(|raw| parse::parse_date_to_epoch(&raw))
            .unwrap_or_default(),
        message_id: headers
            .get_first_value("Message-ID")
            .map(|raw| normalize_message_id(&raw))
            .unwrap_or_default(),
        in_reply_to: headers
            .get_first_value("In-Reply-To")
            .as_deref()
            .and_then(first_message_id)
            .unwrap_or_default(),
        references_root: headers
            .get_first_value("References")
            .as_deref()
            .and_then(first_message_id)
            .unwrap_or_default(),
        to: header_addrs(headers, "To"),
        cc: header_addrs(headers, "Cc"),
    }
}

/// Flatten one address header into recipients, expanding RFC 5322 groups and
/// dropping entries that yield no email address. A header that fails to parse
/// contributes nothing rather than failing the message.
fn header_addrs(headers: &[mailparse::MailHeader<'_>], key: &str) -> Vec<Recipient> {
    let Some(header) = headers.get_first_header(key) else {
        return Vec::new();
    };
    let Some(list) = parse::addrparse_lenient(header) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in list.iter() {
        match entry {
            mailparse::MailAddr::Single(info) => push_recipient(&mut out, info),
            mailparse::MailAddr::Group(group) => {
                for info in &group.addrs {
                    push_recipient(&mut out, info);
                }
            }
        }
    }
    out
}

fn push_recipient(out: &mut Vec<Recipient>, info: &mailparse::SingleInfo) {
    if info.addr.is_empty() {
        return;
    }
    out.push(Recipient {
        name: info.display_name.clone().unwrap_or_default(),
        addr: info.addr.clone(),
    });
}

/// The message's send time: its `Date` header when that parses, otherwise the
/// server's INTERNALDATE (arrival time). Some senders omit the header or write
/// one chrono rejects; without the fallback such a message is dated 1970 and
/// sinks to the bottom of every date-ordered list.
pub(super) fn message_date(header_date: i64, fetch: &async_imap::types::Fetch) -> i64 {
    if header_date != 0 {
        return header_date;
    }
    fetch
        .internal_date()
        .map(|date| date.timestamp())
        .unwrap_or_default()
}

/// Attachment metadata without downloading the MIME payload. Inline CID images
/// are left to the cached body's more precise HTML-reference check.
pub(super) fn attachment_files(
    body: &async_imap::imap_proto::types::BodyStructure<'_>,
) -> Vec<crate::store::CardFile> {
    use async_imap::imap_proto::types::BodyStructure;
    let (common, other) = match body {
        BodyStructure::Multipart { bodies, .. } => {
            return bodies.iter().flat_map(attachment_files).collect();
        }
        BodyStructure::Basic { common, other, .. }
        | BodyStructure::Text { common, other, .. }
        | BodyStructure::Message { common, other, .. } => (common, other),
    };
    let disposition = common.disposition.as_ref();
    let explicit_attachment = disposition
        .is_some_and(|header_disposition| header_disposition.ty.eq_ignore_ascii_case("attachment"));
    let filename = disposition
        .and_then(|header_disposition| body_parameter(&header_disposition.params, "filename"))
        .or_else(|| body_parameter(&common.ty.params, "name"));
    if !explicit_attachment && common.ty.ty.eq_ignore_ascii_case("image") && other.id.is_some() {
        return Vec::new();
    }
    let mime = format!("{}/{}", common.ty.ty, common.ty.subtype).to_ascii_lowercase();
    if !parse::is_attachment_part(&mime, explicit_attachment, filename.is_some()) {
        return Vec::new();
    }
    vec![crate::store::CardFile {
        filename: filename.unwrap_or_else(|| "attachment".to_string()),
        mime,
    }]
}

/// Reuse the MIME parser's RFC 2231 charset/continuation handling, and decode
/// the RFC 2047 words some servers return in otherwise ordinary name params.
pub(super) fn body_parameter(
    params: &async_imap::imap_proto::types::BodyParams<'_>,
    name: &str,
) -> Option<String> {
    let params = params.as_ref()?;
    if let Some((_, value)) = params
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
    {
        return Some(parse::decode_words(&format!(" {value}")));
    }
    let prefix = format!("{name}*");
    let mut header = String::from("attachment");
    for (key, value) in params {
        if key.to_ascii_lowercase().starts_with(&prefix) {
            header.push_str(&format!("; {key}=\"{value}\""));
        }
    }
    mailparse::parse_content_disposition(&header)
        .params
        .get(name)
        .map(|value| parse::decode_words(&format!(" {value}")))
}

/// FETCH item list. Adds `X-GM-THRID` on Gmail so messages thread by Gmail's
/// server-side thread id, and `BODY.PEEK[]` when the full message is wanted.
pub(super) fn fetch_items(gmail: bool, body: bool) -> &'static str {
    match (gmail, body) {
        (true, true) => {
            "(UID FLAGS INTERNALDATE RFC822.HEADER BODYSTRUCTURE X-GM-MSGID X-GM-THRID BODY.PEEK[])"
        }
        (true, false) => {
            "(UID FLAGS INTERNALDATE RFC822.HEADER BODYSTRUCTURE X-GM-MSGID X-GM-THRID)"
        }
        (false, true) => "(UID FLAGS INTERNALDATE RFC822.HEADER BODYSTRUCTURE BODY.PEEK[])",
        (false, false) => "(UID FLAGS INTERNALDATE RFC822.HEADER BODYSTRUCTURE)",
    }
}

/// Whether the server advertises Gmail's `X-GM-EXT-1` extension, i.e. supports
/// `X-GM-THRID`. Covers Gmail/Workspace via both OAuth and app passwords. Issues
/// one CAPABILITY round-trip; negligible next to the connect-time TLS + auth.
pub(super) async fn supports_gmail_ext(session: &mut Session) -> bool {
    match capabilities_once(session).await {
        Ok(caps) => caps.has_str("X-GM-EXT-1"),
        Err(_) => false,
    }
}

/// Whether the server advertises CONDSTORE (RFC 7162), i.e. accepts `SELECT ... (CONDSTORE)` and
/// `FETCH ... (CHANGEDSINCE n)`. Absent it, the flag reconciliation in [`sync_flags`] must not be
/// attempted at all: a server that rejects the parameter without a properly tagged response leaves
/// the connection unusable (see the note there).
pub(super) async fn supports_condstore(session: &mut Session) -> bool {
    match capabilities_once(session).await {
        Ok(caps) => caps.has_str("CONDSTORE"),
        Err(_) => false,
    }
}

pub(super) fn thread_key(
    gmail_thrid: Option<u64>,
    message_id: &str,
    in_reply_to: &str,
    references_root: &str,
    uid: u32,
) -> String {
    // Gmail computes threads server-side (References + In-Reply-To + subject
    // similarity), matching what the user sees in the Gmail UI. Prefer it when
    // the server exposes X-GM-THRID; namespace it so it never collides with a
    // Message-ID-based key.
    if let Some(thrid) = gmail_thrid {
        return format!("gmthrid:{thrid}");
    }
    if !references_root.is_empty() {
        return references_root.to_string();
    }
    if !in_reply_to.is_empty() {
        return in_reply_to.to_string();
    }
    if !message_id.is_empty() {
        return message_id.to_string();
    }
    format!("uid:{uid}")
}

/// Strip angle brackets and whitespace from an RFC Message-ID, preserving the original casing.
///
/// Message-IDs are case-sensitive opaque tokens (RFC 5322) and this value is echoed back on the
/// wire in reply `In-Reply-To`/`References` headers, where a case change breaks the recipient's
/// threading (e.g. Gmail matches ids byte-for-byte). Store lookups that compare ids wrap both sides
/// in `lower()` instead.
pub(super) fn normalize_message_id(value: &str) -> String {
    if let Some(start) = value.find('<')
        && let Some(end) = value[start..].find('>')
    {
        let id = &value[start + 1..start + end];
        return id.trim().to_string();
    }
    value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string()
}

/// The first RFC message id in a header value that may hold several, such as `References` (thread root first) or an `In-Reply-To` carrying more than one.
///
/// Angle brackets decide where the id starts, not whitespace: RFC 822 allowed a leading phrase, and
/// senders still emit `In-Reply-To: Your message of Tuesday <parent@example.com>`. Splitting on
/// whitespace first would take `Your` as the id and, absent `References`, thread every such message
/// together. Only a value with no brackets at all falls back to its first whitespace-separated token.
pub(super) fn first_message_id(value: &str) -> Option<String> {
    if value.contains('<') {
        let id = normalize_message_id(value);
        return (!id.is_empty()).then_some(id);
    }
    value
        .split_whitespace()
        .map(normalize_message_id)
        .find(|id| !id.is_empty())
}
