//! Read dates, recipients and identity fields from mail envelopes.
//!
//! Part of the shared message parser used by the IMAP and local mail paths.
//! The facade keeps its existing entry points for both platform clients.

use super::*;

/// Parse an email `Date` header to Unix epoch seconds, returning 0 when it is
/// empty or unparseable. Mail dates are RFC 2822 (the IMAP envelope and the
/// `Date:` header both use it); RFC 3339/ISO is accepted as a fallback for the
/// occasional non-conformant sender. Storing an epoch lets the DB sort by date
/// directly and the frontend format it in local time.
pub fn parse_date_to_epoch(raw: &str) -> i64 {
    let raw = raw.trim();
    if raw.is_empty() {
        return 0;
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(raw) {
        return dt.timestamp();
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
        return dt.timestamp();
    }
    0
}

/// The envelope of an outgoing message, as much of it as identifies the copy
/// the server files in Sent. Headers only: the body — and the megabytes of
/// attachments behind it — is never parsed.
///
/// It exists because the `Message-ID` we sent is not always the one that comes
/// back (Proton Bridge replaces it with an id of its own,
/// `@protonmail.internalid`), so recognising our own copy has to fall back to
/// what the provider does keep: same sender, same subject, same recipients,
/// sent at about the same moment. The reader pairs optimistic bubbles to server
/// copies by the same rule.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct SentEnvelope {
    /// Normalized `Message-ID`, blank when it could not be read.
    pub message_id: String,
    pub subject: String,
    pub from_addr: String,
    /// Bare `To` + `Cc` addresses, lowercased and sorted so comparison does not
    /// depend on header order. `Bcc` is deliberately absent: the sending server
    /// strips it, so the copy that comes back need not carry it.
    pub recipients: Vec<String>,
    /// `Date` as Unix epoch seconds, 0 when unreadable.
    pub date: i64,
}

/// Read the [`SentEnvelope`] of a raw outgoing message.
pub fn sent_envelope_of(raw: &[u8]) -> SentEnvelope {
    let Ok((headers, _)) = mailparse::parse_headers(raw) else {
        return SentEnvelope::default();
    };
    let (_, from_addr) = split_address(&headers.get_first_value("From").unwrap_or_default());
    let mut recipients = bare_addresses(&headers.get_all_values("To"));
    recipients.extend(bare_addresses(&headers.get_all_values("Cc")));
    SentEnvelope {
        message_id: normalize_msgid(&headers.get_first_value("Message-ID").unwrap_or_default()),
        subject: headers.get_first_value("Subject").unwrap_or_default(),
        from_addr,
        recipients: normalize_addresses(recipients),
        date: parse_date_to_epoch(&headers.get_first_value("Date").unwrap_or_default()),
    }
}

/// Lowercase, de-duplicate and sort a set of addresses so two spellings of the
/// same recipient list compare equal.
pub fn normalize_addresses(addrs: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = addrs
        .into_iter()
        .map(|addr| addr.trim().to_lowercase())
        .filter(|addr| !addr.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The bare addresses of one address-header's values, groups expanded.
fn bare_addresses(values: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let Ok(list) = addrparse(value) else { continue };
        for addr in list.iter() {
            match addr {
                mailparse::MailAddr::Single(info) => out.push(info.addr.clone()),
                mailparse::MailAddr::Group(group) => {
                    out.extend(group.addrs.iter().map(|info| info.addr.clone()))
                }
            }
        }
    }
    out
}

/// Strip angle brackets / whitespace from a single Message-ID header value,
/// leaving the bare `id@host`. Returns empty for malformed input.
pub(super) fn normalize_msgid(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim()
        .to_string()
}

/// Normalize a `References:` header into space-separated bare ids (no angle
/// brackets), preserving order. The header is whitespace-separated `<id>` tokens.
pub(super) fn normalize_references(raw: &str) -> String {
    raw.split_whitespace()
        .map(|tok| tok.trim_start_matches('<').trim_end_matches('>').trim())
        .filter(|tok| !tok.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Flatten one or more header values (e.g. multiple `Cc:` lines) into a single
/// comma-separated address list, with RFC 2047 encoded-words decoded. Group
/// syntax and nested groups are flattened to their member addresses; if parsing
/// fails entirely the raw header value is preserved so the user still sees it.
pub(super) fn collect_address_list(headers: &[&MailHeader]) -> String {
    let mut out: Vec<String> = Vec::new();
    for header in headers {
        let value = header.get_value();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        match addrparse_lenient(header) {
            Some(list) => {
                for entry in list.iter() {
                    push_addr(entry, &mut out);
                }
            }
            None => out.push(trimmed.to_string()),
        }
    }
    out.join(", ")
}

fn push_addr(entry: &MailAddr, out: &mut Vec<String>) {
    match entry {
        MailAddr::Single(info) => {
            let formatted = match info.display_name.as_deref() {
                Some(name) if !name.trim().is_empty() => {
                    format!("{} <{}>", name.trim(), info.addr)
                }
                _ => info.addr.clone(),
            };
            if !formatted.is_empty() {
                out.push(formatted);
            }
        }
        MailAddr::Group(group) => {
            for addr in &group.addrs {
                let formatted = match addr.display_name.as_deref() {
                    Some(name) if !name.trim().is_empty() => {
                        format!("{} <{}>", name.trim(), addr.addr)
                    }
                    _ => addr.addr.clone(),
                };
                if !formatted.is_empty() {
                    out.push(formatted);
                }
            }
        }
    }
}
