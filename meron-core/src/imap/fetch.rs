//! Fetching message data and building headers.
//!
//! Reads message data by UID or Message-ID from a selected mailbox.
//! Converts server responses into the header and bulk metadata used by the store.

use super::*;

/// A message located by Message-ID and fully fetched: its header row (with a
/// computed `thread_key` and source `folder` populated) plus the parsed body.
/// Used by the on-demand ancestor fetch that fills thread gaps when a reply or
/// draft references messages outside the locally-synced window.
pub struct FetchedMessage {
    pub header: MessageHeader,
    pub message: parse::Message,
}

/// Look up messages by Message-ID and fetch the metadata needed by their callers.
///
/// Locate messages in `folder` whose `Message-ID` matches any id in `ids` (bare, no angle brackets) and
/// fetch flags + envelope + full body for each match in a single round-trip. Returns one `FetchedMessage`
/// per located UID; ids with no match here are simply absent from the result so the caller can try the next
/// folder.
///
/// The `thread_key` is computed exactly as the recent-sync path does, so a fetched ancestor groups into the
/// same thread as the reply that referenced it.
pub async fn fetch_by_message_ids(
    session: &mut Session,
    folder: &str,
    ids: &[String],
    media_root: &std::path::Path,
    account: &str,
) -> Result<Vec<FetchedMessage>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    session.select(folder).await.context("SELECT")?;
    let set: HashSet<u32> = session
        .uid_search(message_id_search_criteria(ids))
        .await
        .context("UID SEARCH Message-ID")?;
    if set.is_empty() {
        return Ok(Vec::new());
    }
    let uids = set.into_iter().collect::<Vec<_>>();
    let gmail = supports_gmail_ext(session).await;
    let mut out = Vec::new();
    for uid_set in uid_set_chunks(&uids, MAX_UID_SET_LEN) {
        let mut stream = session
            .uid_fetch(uid_set, fetch_items(gmail, true))
            .await
            .context("UID FETCH Message-ID matches")?;
        while let Some(item) = stream.next().await {
            let fetch = item.context("UID FETCH item")?;
            let uid = match fetch.uid {
                Some(uid) => uid,
                None => continue,
            };
            let raw = match fetch.body() {
                Some(body) => body.to_vec(),
                None => continue,
            };
            let seen = fetch
                .flags()
                .any(|flag| matches!(flag, async_imap::types::Flag::Seen));
            let starred = fetch
                .flags()
                .any(|flag| matches!(flag, async_imap::types::Flag::Flagged));
            let ef = fetch.header().map(header_fields).unwrap_or_default();
            let thread_key = thread_key(
                fetch.gmail_thread_id().copied(),
                &ef.message_id,
                &ef.in_reply_to,
                &ef.references_root,
                uid,
            );
            let media = parse::MediaCtx {
                root: media_root.to_path_buf(),
                account: account.to_string(),
                folder: folder.to_string(),
                uid,
            };
            let mut message = parse::parse_message(&raw, Some(&media));
            let date = message_date(ef.date, &fetch);
            if message.date == 0 {
                message.date = date;
            }
            out.push(FetchedMessage {
                header: MessageHeader {
                    uid,
                    folder: folder.to_string(),
                    subject: ef.subject,
                    from_name: ef.from_name,
                    from_addr: ef.from_addr,
                    date,
                    seen,
                    starred,
                    server_tags: Some(fetch_tags(&fetch)),
                    ai_sorted: fetch_ai_sorted(&fetch),
                    meron_mark: fetch_meron_mark(&fetch),
                    ai_shown: false,
                    thread_key,
                    message_id: ef.message_id,
                    gmail_msg_id: fetch.gmail_msg_id().copied(),
                    in_reply_to: ef.in_reply_to,
                    to: ef.to,
                    cc: ef.cc,
                    recipient_overflow: 0,
                    files: fetch.bodystructure().map(attachment_files),
                },
                message,
            });
        }
    }
    Ok(out)
}

/// Build a SEARCH criteria matching any `HEADER MESSAGE-ID` in `ids`. IMAP SEARCH
/// has no native set membership, so terms are chained with the binary prefix
/// `OR` operator (`OR a b`, then `OR (OR a b) c`, …).
pub(super) fn message_id_search_criteria(ids: &[String]) -> String {
    let mut terms = ids
        .iter()
        .map(|id| format!("HEADER MESSAGE-ID {}", imap_quote(id)));
    let Some(first) = terms.next() else {
        return "ALL".to_string();
    };
    terms.fold(first, |expr, term| format!("OR {expr} {term}"))
}

pub async fn fetch_headers_by_uid(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
) -> Result<Vec<MessageHeader>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    session.select(folder).await.context("SELECT")?;
    uid_fetch_headers(session, uids).await
}

/// [`fetch_headers_by_uid`] in the already selected mailbox.
pub(super) async fn uid_fetch_headers(
    session: &mut Session,
    uids: &[u32],
) -> Result<Vec<MessageHeader>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let gmail = supports_gmail_ext(session).await;
    let mut out = Vec::new();
    for uid_set in uid_set_chunks(uids, MAX_UID_SET_LEN) {
        let mut stream = session
            .uid_fetch(uid_set, fetch_items(gmail, false))
            .await
            .context("UID FETCH search headers")?;
        while let Some(item) = stream.next().await {
            let fetch = item.context("UID FETCH search header item")?;
            let uid = match fetch.uid {
                Some(uid) => uid,
                None => continue,
            };
            let seen = fetch
                .flags()
                .any(|flag| matches!(flag, async_imap::types::Flag::Seen));
            let starred = fetch
                .flags()
                .any(|flag| matches!(flag, async_imap::types::Flag::Flagged));
            let ef = fetch.header().map(header_fields).unwrap_or_default();
            let thread_key = thread_key(
                fetch.gmail_thread_id().copied(),
                &ef.message_id,
                &ef.in_reply_to,
                &ef.references_root,
                uid,
            );
            out.push(MessageHeader {
                uid,
                subject: ef.subject,
                from_name: ef.from_name,
                from_addr: ef.from_addr,
                date: message_date(ef.date, &fetch),
                seen,
                starred,
                server_tags: Some(fetch_tags(&fetch)),
                ai_sorted: fetch_ai_sorted(&fetch),
                meron_mark: fetch_meron_mark(&fetch),
                ai_shown: false,
                thread_key,
                message_id: ef.message_id,
                gmail_msg_id: fetch.gmail_msg_id().copied(),
                in_reply_to: ef.in_reply_to,
                folder: String::new(),
                to: ef.to,
                cc: ef.cc,
                recipient_overflow: 0,
                files: fetch.bodystructure().map(attachment_files),
            });
        }
    }
    out.sort_unstable_by(|a, b| b.uid.cmp(&a.uid));
    Ok(out)
}

/// The list header of a message held as raw RFC822 rather than on a server
/// (local folders), read the same way a header FETCH is, so stored mail threads
/// and lists exactly like synced mail.
pub fn header_from_raw(raw: &[u8], uid: u32, seen: bool, starred: bool) -> MessageHeader {
    let ef = header_fields(raw);
    let thread_key = thread_key(
        None,
        &ef.message_id,
        &ef.in_reply_to,
        &ef.references_root,
        uid,
    );
    MessageHeader {
        uid,
        subject: ef.subject,
        from_name: ef.from_name,
        from_addr: ef.from_addr,
        date: ef.date,
        seen,
        starred,
        thread_key,
        message_id: ef.message_id,
        in_reply_to: ef.in_reply_to,
        to: ef.to,
        cc: ef.cc,
        ..Default::default()
    }
}

/// Each of `uids`' Date header as epoch seconds (INTERNALDATE when absent or
/// unparseable), the same date [`fetch_headers_by_uid`] reports. Fetches only
/// that one header field, so a search can order a large hit set by date without
/// downloading every header. UIDs the server no longer has are left out.
pub async fn fetch_dates_by_uid(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
) -> Result<Vec<(u32, i64)>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    session.select(folder).await.context("SELECT")?;
    let mut out = Vec::with_capacity(uids.len());
    for uid_set in uid_set_chunks(uids, MAX_UID_SET_LEN) {
        let mut stream = session
            .uid_fetch(
                uid_set,
                "(UID INTERNALDATE BODY.PEEK[HEADER.FIELDS (DATE)])",
            )
            .await
            .context("UID FETCH search dates")?;
        while let Some(item) = stream.next().await {
            let fetch = item.context("UID FETCH search date item")?;
            let Some(uid) = fetch.uid else {
                continue;
            };
            let date = fetch
                .header()
                .and_then(|header| mailparse::parse_headers(header).ok())
                .and_then(|(headers, _)| headers.get_first_value("Date"))
                .map(|raw| parse::parse_date_to_epoch(&raw))
                .unwrap_or_default();
            out.push((uid, message_date(date, &fetch)));
        }
    }
    Ok(out)
}

/// Bulk or automated mail signals (List-Unsubscribe, List-Id, Precedence,
/// Auto-Submitted) for these UIDs, read with one header-only fetch. Only UIDs
/// with a signal are in the map.
pub async fn fetch_bulk_signals(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
) -> Result<HashMap<u32, &'static str>> {
    let mut out = HashMap::new();
    if uids.is_empty() {
        return Ok(out);
    }
    session.select(folder).await.context("SELECT")?;
    let query = format!(
        "(UID BODY.PEEK[HEADER.FIELDS ({})])",
        crate::ai_sort::BULK_HEADERS
    );
    for uid_set in uid_set_chunks(uids, MAX_UID_SET_LEN) {
        let mut stream = session
            .uid_fetch(uid_set, &query)
            .await
            .context("UID FETCH bulk headers")?;
        while let Some(item) = stream.next().await {
            let fetch = item.context("UID FETCH bulk header item")?;
            let Some(uid) = fetch.uid else {
                continue;
            };
            if let Some(signal) = fetch.header().and_then(crate::ai_sort::bulk_header_signal) {
                out.insert(uid, signal);
            }
        }
    }
    Ok(out)
}
