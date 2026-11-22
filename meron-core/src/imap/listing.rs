//! Mailbox windows and incremental listing.

use super::*;

/// SELECT `folder` and return `(UIDVALIDITY, UIDNEXT, uids)`, where `uids` holds
/// the most recent `limit` messages newest first — the same window
/// [`fetch_recent`] covers, but established without fetching any message.
///
/// Used by the poison-message recovery path, which needs the window as UIDs it
/// can subdivide. A `UID SEARCH` reply is a list of bare numbers, so it cannot
/// itself hit the parse failure that sent us down this path.
pub async fn recent_uids(
    session: &mut Session,
    folder: &str,
    limit: u32,
) -> Result<(u32, u32, Vec<u32>)> {
    let mailbox = session.select(folder).await.context("SELECT")?;
    let uidvalidity = mailbox.uid_validity.unwrap_or(0);
    let uid_next = mailbox.uid_next.unwrap_or(0);
    let set: HashSet<u32> = session.uid_search("ALL").await.context("UID SEARCH ALL")?;
    let mut uids: Vec<u32> = set.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    uids.truncate(limit.max(1) as usize);
    Ok((uidvalidity, uid_next, uids))
}

/// A folder's UID markers and complete UID set, read by one SELECT and one
/// `UID SEARCH ALL`.
pub struct FolderListing {
    pub uidvalidity: u32,
    pub uid_next: u32,
    /// EXISTS from the SELECT.
    pub exists: u32,
    pub uids: HashSet<u32>,
}

impl FolderListing {
    /// The most recent `limit` UIDs, newest first: the window [`fetch_recent`]
    /// reads.
    pub fn window(&self, limit: u32) -> Vec<u32> {
        let mut uids: Vec<u32> = self.uids.iter().copied().collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.truncate(limit.max(1) as usize);
        uids
    }

    /// The listing agrees with the SELECT that preceded it. A mismatch means
    /// the mailbox changed in between, or the connection is desynced (see
    /// [`sync_flags`]); either way the caller should not trust it as a window.
    pub fn consistent(&self) -> bool {
        self.uids.len() == self.exists as usize
    }
}

pub async fn folder_listing(session: &mut Session, folder: &str) -> Result<FolderListing> {
    let mailbox = session.select(folder).await.context("SELECT")?;
    let uids: HashSet<u32> = session.uid_search("ALL").await.context("UID SEARCH ALL")?;
    Ok(FolderListing {
        uidvalidity: mailbox.uid_validity.unwrap_or(0),
        uid_next: mailbox.uid_next.unwrap_or(0),
        exists: mailbox.exists,
        uids,
    })
}

/// What [`fetch_recent_incremental`] read.
pub struct IncrementalBatch {
    /// UID markers plus, in `messages`, only the window's messages the cache
    /// did not hold, read in full.
    pub batch: RecentBatch,
    /// The window's UIDs, newest first: the messages [`fetch_recent`] would
    /// have read in full.
    pub window: Vec<u32>,
    /// Every UID in the folder, listed after `batch.uid_next` was read.
    pub server_uids: HashSet<u32>,
}

/// [`fetch_recent`] for a folder synced before: lists the folder's UIDs and
/// reads in full only the window's messages missing from `known` (the UIDs
/// the cache holds). Nothing new means no message FETCH at all.
///
/// Answers `None`, having read no message, when the full read is needed
/// instead: UIDVALIDITY is unknown or changed, or the listing contradicts the
/// SELECT. Flags of the window's cached messages are the caller's to refresh
/// (CONDSTORE, or [`fetch_flags_by_uid`]).
pub async fn fetch_recent_incremental(
    session: &mut Session,
    folder: &str,
    limit: u32,
    prior_validity: u32,
    known: &HashSet<u32>,
) -> Result<Option<IncrementalBatch>> {
    let listing = folder_listing(session, folder).await?;
    if listing.uidvalidity == 0 || listing.uidvalidity != prior_validity || !listing.consistent() {
        return Ok(None);
    }
    let window = listing.window(limit);
    let missing: Vec<u32> = window
        .iter()
        .copied()
        .filter(|uid| !known.contains(uid))
        .collect();
    let messages = uid_fetch_headers(session, &missing).await?;
    Ok(Some(IncrementalBatch {
        batch: RecentBatch {
            uidvalidity: listing.uidvalidity,
            uid_next: listing.uid_next,
            messages,
        },
        window,
        server_uids: listing.uids,
    }))
}

/// The changeable state of `uids` in `folder` — `(uid, seen, starred, tags)`,
/// the shape of [`FlagSync::changes`] — read with a FLAGS-only FETCH.
pub async fn fetch_flags_by_uid(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
) -> Result<Vec<(u32, bool, bool, Vec<String>, Option<Option<String>>)>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    session.select(folder).await.context("SELECT")?;
    let mut out = Vec::with_capacity(uids.len());
    for uid_set in uid_set_chunks(uids, MAX_UID_SET_LEN) {
        let mut stream = session
            .uid_fetch(uid_set, "(UID FLAGS)")
            .await
            .context("UID FETCH FLAGS")?;
        while let Some(item) = stream.next().await {
            let fetch = item.context("UID FETCH FLAGS item")?;
            let Some(uid) = fetch.uid else {
                continue;
            };
            let seen = fetch
                .flags()
                .any(|flag| matches!(flag, async_imap::types::Flag::Seen));
            let starred = fetch
                .flags()
                .any(|flag| matches!(flag, async_imap::types::Flag::Flagged));
            out.push((
                uid,
                seen,
                starred,
                fetch_tags(&fetch),
                fetch_meron_mark(&fetch),
            ));
        }
    }
    Ok(out)
}

/// Fetch the most recent `limit` messages in `folder` as envelope summaries,
/// newest first. This is the capability Delta Chat core refuses to give us:
/// reading mail that already existed before setup.
pub async fn fetch_recent(session: &mut Session, folder: &str, limit: u32) -> Result<RecentBatch> {
    let mailbox = session.select(folder).await.context("SELECT")?;
    let uidvalidity = mailbox.uid_validity.unwrap_or(0);
    let uid_next = mailbox.uid_next.unwrap_or(0);
    let total = mailbox.exists;
    if total == 0 {
        return Ok(RecentBatch {
            uidvalidity,
            uid_next,
            messages: Vec::new(),
        });
    }
    let limit = limit.max(1);
    let start = total.saturating_sub(limit).saturating_add(1).max(1);
    let set = format!("{start}:{total}");

    let gmail = supports_gmail_ext(session).await;
    let mut out = Vec::new();
    let mut stream = session
        .fetch(set, fetch_items(gmail, false))
        .await
        .context("FETCH")?;
    while let Some(item) = stream.next().await {
        let fetch = item.context("FETCH item")?;
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
    drop(stream);
    out.reverse();
    Ok(RecentBatch {
        uidvalidity,
        uid_next,
        messages: out,
    })
}
