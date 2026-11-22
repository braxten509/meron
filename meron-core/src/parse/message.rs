//! Construct the shared message model from parsed MIME parts.
//!
//! Part of the shared message parser used by the IMAP and local mail paths.
//! The facade keeps its existing entry points for both platform clients.

use super::*;

/// A single message rendered for the conversation/reader view.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    /// Comma-separated `To` recipients (each "Name <addr>" or "addr"), empty when
    /// unknown. Used to detect which of the user's addresses (alias) a message was
    /// delivered to, so replies can default the From accordingly.
    #[serde(default)]
    pub to: String,
    /// Comma-separated `Reply-To` addresses (each "Name <addr>" or "addr"),
    /// empty when the header is absent. Replies should prefer this over From.
    #[serde(default)]
    pub reply_to: String,
    /// Comma-separated `Cc` addresses from the original message, empty when none.
    /// Replies preserve this so other recipients stay on the thread.
    #[serde(default)]
    pub cc: String,
    /// Comma-separated `Bcc` addresses, present only on outgoing copies (Sent /
    /// Drafts) — the sending server strips `Bcc` before delivery, so received
    /// messages never carry it. Empty otherwise.
    #[serde(default)]
    pub bcc: String,
    /// Normalized `Message-ID` ("id@host", no angle brackets). Replies use this
    /// for `In-Reply-To` and the `References` chain so the thread holds together
    /// on the recipient side.
    #[serde(default)]
    pub message_id: String,
    /// Normalized `References` chain (space-separated bare ids, oldest → newest),
    /// when present. Empty when the header is absent (likely a root message).
    #[serde(default)]
    pub references: String,
    /// Whether the message carries headers a receiving MTA adds at delivery
    /// (`Return-Path`, `Delivered-To`, `X-Original-To`) — i.e. this copy was
    /// delivered to us rather than written by us. Direction can't be read off
    /// the From address alone: a shared address configured here as a send-as
    /// alias is also used by colleagues, and their mail matches it. Unlike the
    /// mailbox it currently sits in, this travels with the message when it is
    /// archived or moved. Defaults to false for rows cached before it existed,
    /// which reads as "unknown" and leaves the address match in charge.
    #[serde(default)]
    pub delivered: bool,
    /// Send time as Unix epoch seconds (0 when the `Date` header is absent or
    /// unparseable). The frontend formats it for display in local time.
    pub date: i64,
    pub body: String,
    /// The message's original HTML, or `None` when it was plain text (in which case
    /// `body` already *is* the source). Persisted to the store so a render-logic
    /// change can re-derive `body` without an IMAP refetch, and so the reader's
    /// "HTML mode" tab can show the original email. At parse time it holds the raw
    /// HTML with `cid:` inline images rewritten to `/media/<key>`; the bridge
    /// handler injects a remote-image CSP `<meta>` on read (see `prepare_html`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_html: Option<String>,
    #[serde(default)]
    pub body_is_rendered: bool,
    pub preview: String,
    pub attachments: Vec<Attachment>,
    /// The calendar event a `text/calendar` part carries, when the message is
    /// an invitation (or an update, cancellation or reply to one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<crate::calendar::Invite>,
}

impl Message {
    /// The parts the user would call attachments: every one except those its
    /// HTML body already shows inline (`cid:` logos and signature images,
    /// rewritten to `/media/<key>` at parse time). The reader draws the same
    /// line, and the store's v11 migration mirrors it for rows cached before
    /// the list was stored.
    pub fn file_attachments(&self) -> impl Iterator<Item = &Attachment> {
        let html = self.body_html.as_deref().unwrap_or_default();
        self.attachments
            .iter()
            .filter(move |attachment| match &attachment.key {
                Some(key) => !html.contains(&format!("/media/{key}")),
                None => true,
            })
    }

    pub fn has_attachments(&self) -> bool {
        self.file_attachments().next().is_some()
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub mime: String,
    pub size: usize,
    /// Relative media key (`account/folder/uid/index.ext`) for the bytes written
    /// to disk and served at `/media/<key>`; null only when no media context was
    /// provided (tests, previews), the write failed, or the file is
    /// `oversized`.
    pub key: Option<String>,
    /// Left off disk on purpose: larger than the whole media cache, so it
    /// could not outlive a prune. Downloading the message again would not
    /// change that, which is what tells it from a key-less attachment whose
    /// bytes were never written.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub oversized: bool,
}

/// The Message-ID of a raw message, headers only — the body (and any megabyte
/// of attachments behind it) is never parsed.
pub fn message_id_of(raw: &[u8]) -> String {
    match mailparse::parse_headers(raw) {
        Ok((headers, _)) => {
            normalize_msgid(&headers.get_first_value("Message-ID").unwrap_or_default())
        }
        Err(_) => String::new(),
    }
}

/// Parse a full RFC822 message into a reader-view summary. Inline images are
/// written under `media` (when provided) and referenced by key; passing `None`
/// (tests, previews) skips disk writes and leaves every attachment key null.
pub fn parse_message(raw: &[u8], media: Option<&MediaCtx>) -> Message {
    let message = parse_message_unpruned(raw, media);
    if let Some(ctx) = media {
        prune_media_cache(&ctx.root);
    }
    message
}

/// Batch fetches prune once off the async runtime after parsing their messages.
pub(crate) fn parse_message_unpruned(raw: &[u8], media: Option<&MediaCtx>) -> Message {
    let mail = match parse_mail(raw) {
        Ok(mail) => mail,
        Err(_) => return Message::default(),
    };
    let headers = &mail.headers;
    let subject = headers.get_first_value("Subject").unwrap_or_default();
    let (from_name, from_addr) = from_of(headers);
    let to = collect_address_list(&headers.get_all_headers("To"));
    let reply_to = collect_address_list(&headers.get_all_headers("Reply-To"));
    let cc = collect_address_list(&headers.get_all_headers("Cc"));
    let bcc = collect_address_list(&headers.get_all_headers("Bcc"));
    let message_id = normalize_msgid(&headers.get_first_value("Message-ID").unwrap_or_default());
    let references =
        normalize_references(&headers.get_first_value("References").unwrap_or_default());
    let date = parse_date_to_epoch(&headers.get_first_value("Date").unwrap_or_default());
    let delivered = ["Return-Path", "Delivered-To", "X-Original-To"]
        .iter()
        .any(|name| headers.get_first_value(name).is_some());
    let sources = body_sources(&mail);
    let invite = calendar_invite(&mail);

    let mut attachments = Vec::new();
    let mut cid_keys: Vec<(String, String)> = Vec::new();
    collect_attachments(&mail, &mut attachments, &mut cid_keys, media);
    // Prefer the MIME text/plain alternative for the conversation and Plain
    // reader view. Keep HTML separately for the HTML reader; when a message is
    // HTML-only, fall back to converting HTML so there is still readable text.
    let (body, body_html, body_is_rendered) = match (sources.plain, sources.html) {
        (Some(plain), Some(html)) => {
            let plain = normalize_text(&plain);
            let html = rewrite_cid_refs(&html, &cid_keys);
            (plain, Some(html), false)
        }
        (None, Some(html)) => {
            let html = rewrite_cid_refs(&html, &cid_keys);
            (render_body(&html), Some(html), true)
        }
        (Some(plain), None) => {
            let plain = normalize_text(&plain);
            (plain, None, false)
        }
        (None, None) => (String::new(), None, false),
    };
    let preview = preview_of(&body);

    Message {
        subject,
        from_name,
        from_addr,
        to,
        reply_to,
        cc,
        bcc,
        message_id,
        references,
        delivered,
        date,
        body,
        body_html,
        body_is_rendered,
        preview,
        attachments,
        invite,
    }
}

/// The event of the message's calendar part. iMIP puts it in a `text/calendar`
/// alternative (Outlook: that alone; Google: that plus an `application/ics`
/// copy as a file), so look at every calendar leaf, and take one that names
/// its iTIP method on the Content-Type first — that is the one the sender
/// means to be acted on.
fn calendar_invite(mail: &ParsedMail) -> Option<crate::calendar::Invite> {
    let mut parts = Vec::new();
    collect_calendar_parts(mail, &mut parts);
    parts.sort_by_key(|part| !part.ctype.params.contains_key("method"));
    parts
        .into_iter()
        .filter_map(|part| text_body(part))
        .find_map(|ics| crate::calendar::parse_invite(&ics))
}

fn collect_calendar_parts<'a>(part: &'a ParsedMail<'a>, out: &mut Vec<&'a ParsedMail<'a>>) {
    if !part.subparts.is_empty() {
        for sub in &part.subparts {
            collect_calendar_parts(sub, out);
        }
        return;
    }
    if is_calendar_mime(&part.ctype.mimetype) {
        out.push(part);
    }
}

pub(crate) fn is_calendar_mime(mime: &str) -> bool {
    mime.eq_ignore_ascii_case("text/calendar") || mime.eq_ignore_ascii_case("application/ics")
}
