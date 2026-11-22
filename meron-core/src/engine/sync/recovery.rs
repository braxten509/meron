//! Recovery in the mail synchronization engine.
//!
//! Coordinates the corresponding IMAP and store operations during folder updates.
//! The parent module exposes these operations to foreground and background sync.

use super::*;

/// [`imap::fetch_recent`] for one folder, recovering from messages whose FETCH
/// response cannot be parsed instead of failing the whole batch. Prefer this to
/// calling `imap::fetch_recent` inside a session closure.
///
/// Read-only, so it must not share a session with a mutating command: a wedged
/// connection has to be discarded, and a mutation that already reached the
/// server must never be retried. Run the mutation first, then call this.
pub async fn fetch_recent_resilient(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> anyhow::Result<imap::RecentBatch> {
    let attempt = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            Box::pin(async move { imap::fetch_recent(session, &folder, limit).await })
        })
        .await;
    match attempt {
        Ok(batch) => Ok(batch),
        Err(err) if imap::is_unparseable_response(&err) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "account={account} folder={folder}: unparseable FETCH response, \
                 re-reading the window message by message: {err:#}"
            );
            recover_recent_batch(engine, account, folder, limit).await
        }
        Err(err) => Err(err),
    }
}

/// Re-read the recent window of `folder` while working around messages whose
/// FETCH response cannot be parsed. Establishes the window as UIDs (a
/// `UID SEARCH` reply is immune to the failure), then reads it in ranges that
/// exclude the offending messages.
pub(in crate::engine) async fn recover_recent_batch(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> anyhow::Result<imap::RecentBatch> {
    let (uidvalidity, uid_next, uids) = engine
        .with_read_session(account, |session| {
            let folder = folder.to_string();
            Box::pin(async move { imap::recent_uids(session, &folder, limit).await })
        })
        .await?;
    let messages = read_isolating(engine, account, folder, uids).await?;
    Ok(imap::RecentBatch {
        uidvalidity,
        uid_next,
        messages,
    })
}

/// [`fetch_headers_isolating_unparseable`], logging what it had to skip.
pub(super) async fn read_isolating(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: Vec<u32>,
) -> anyhow::Result<Vec<imap::MessageHeader>> {
    let (messages, skipped) =
        fetch_headers_isolating_unparseable(engine, account, folder, uids).await?;
    if !skipped.is_empty() {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.sync",
            "account={account} folder={folder}: {} of {} messages skipped as unreadable: {skipped:?}",
            skipped.len(),
            messages.len() + skipped.len()
        );
    }
    Ok(messages)
}

/// Ceiling on FETCH attempts one poison-message recovery may spend. A run with
/// a single bad message costs about `2 * log2(window)` attempts; this only
/// bites when a folder is riddled with them, where the right answer is to store
/// what we have rather than reconnect dozens of times.
pub(in crate::engine) const ISOLATION_ATTEMPT_LIMIT: usize = 32;

/// Fetch headers for `uids`, working around messages whose FETCH response the
/// IMAP parser rejects (see [`imap::is_unparseable_response`]).
///
/// Such a response wedges the connection it arrived on, so it cannot be skipped
/// mid-stream — the session has to go and the range has to be re-fetched
/// without it. This halves any failing range until the offending message sits
/// alone, drops just that one, and keeps its neighbours. Every attempt runs
/// through `with_read_session`, which discards the failed session and hands the
/// retry a fresh connection.
///
/// Returns the headers it could read plus the UIDs it gave up on. A genuine
/// network or server error aborts, since retrying narrower ranges would not
/// help.
pub(in crate::engine) async fn fetch_headers_isolating_unparseable(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: Vec<u32>,
) -> anyhow::Result<(Vec<imap::MessageHeader>, Vec<u32>)> {
    fetch_headers_isolating_until(engine, account, folder, uids, None).await
}

/// A catch-up deadline returns completed headers and isolated poison UIDs;
/// cancelled or unread chunks remain eligible for the next sync.
pub(super) async fn fetch_headers_isolating_until(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    uids: Vec<u32>,
    deadline: Option<tokio::time::Instant>,
) -> anyhow::Result<(Vec<imap::MessageHeader>, Vec<u32>)> {
    let mut pending = vec![uids];
    let mut out: Vec<imap::MessageHeader> = Vec::new();
    let mut skipped: Vec<u32> = Vec::new();
    let mut attempts = 0usize;

    while let Some(chunk) = pending.pop() {
        if chunk.is_empty() {
            continue;
        }
        if attempts >= ISOLATION_ATTEMPT_LIMIT {
            let abandoned: usize = pending.iter().map(Vec::len).sum::<usize>() + chunk.len();
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "account={account} folder={folder}: giving up isolating unparseable \
                 messages after {attempts} fetches, {abandoned} UIDs left unread"
            );
            break;
        }
        attempts += 1;
        let operation = engine.with_read_session(account, |session| {
            let folder = folder.to_string();
            let chunk = chunk.clone();
            Box::pin(async move { imap::fetch_headers_by_uid(session, &folder, &chunk).await })
        });
        let result = if let Some(deadline) = deadline {
            match tokio::time::timeout_at(deadline, operation).await {
                Ok(result) => result,
                Err(_) => break,
            }
        } else {
            operation.await
        };
        match result {
            Ok(headers) => out.extend(headers),
            Err(err) if !imap::is_unparseable_response(&err) => return Err(err),
            Err(err) if chunk.len() == 1 => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={folder}: skipping uid={}, its FETCH \
                     response could not be parsed: {err:#}",
                    chunk[0]
                );
                skipped.push(chunk[0]);
            }
            Err(_) => {
                // Push the tail first so the halves are attempted in UID order.
                let mid = chunk.len() / 2;
                pending.push(chunk[mid..].to_vec());
                pending.push(chunk[..mid].to_vec());
            }
        }
    }

    out.sort_unstable_by_key(|header| std::cmp::Reverse(header.uid));
    Ok((out, skipped))
}
