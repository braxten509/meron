//! Fetching full messages and caching inline media.
//!
//! Caches MIME parts while fetching messages for display and offline use.
//! Prefetch chooses recent UIDs before downloading complete messages in batches.

use super::*;

/// Fetch and parse one full message by UID, assuming `folder` is already selected on `session`.
///
/// Shared by the on-demand reader (which selects then calls this once) and the body prefetcher
/// (which selects once and pulls many UIDs over a single connection).
///
/// `peek` selects `BODY.PEEK[]` over `RFC822`: both return the full message, but `RFC822`
/// implicitly sets `\Seen` server-side while `BODY.PEEK[]` does not. The prefetcher must peek so
/// background warming never marks unread mail as read.
pub async fn fetch_full_message(
    session: &mut Session,
    uid: u32,
    media: &parse::MediaCtx,
    peek: bool,
) -> Result<Option<parse::Message>> {
    let item = if peek {
        "(INTERNALDATE BODY.PEEK[])"
    } else {
        "(INTERNALDATE RFC822)"
    };
    let mut stream = session
        .uid_fetch(uid.to_string(), item)
        .await
        .context("UID FETCH")?;
    let mut message: Option<parse::Message> = None;
    while let Some(item) = stream.next().await {
        let fetch = item.context("UID FETCH item")?;
        if let Some(body) = fetch.body() {
            let mut parsed = parse::parse_message(body, Some(media));
            parsed.date = message_date(parsed.date, &fetch);
            message = Some(parsed);
            break;
        }
    }
    drop(stream);
    Ok(message)
}

/// How many messages one `UID FETCH` pulls. One round trip per message dominates
/// when the bodies themselves are small; a short batch still bounds how long a
/// caller holds the connection before an interactive read can use it.
pub const BODY_FETCH_BATCH: usize = 4;

/// Select `folder` and fetch full bodies for `uids`, parsing each into a `Message`.
///
/// Used by the thread reader and the body prefetcher. `peek` so reading doesn't flip server-side
/// `\Seen`.
///
/// Each message goes to `on_message` as it arrives rather than into a returned list, so a caller
/// keeps what was downloaded before an error or a deadline cut the rest short.
pub async fn fetch_bodies(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
    media_root: std::path::PathBuf,
    account: &str,
    on_message: &mut (dyn FnMut(u32, parse::Message) + Send),
) -> Result<()> {
    // One recursive walk per batch, including one cut short by an error, a
    // deadline or cancellation: what arrived before that is already on disk.
    let mut prune = PruneMediaOnDrop(Some(media_root.clone()));
    let result =
        fetch_bodies_unpruned(session, folder, uids, media_root, account, on_message).await;
    // A fetch that ran to its end waits for the prune, so a caller counting
    // attachment files afterwards sees the ones that are going to stay.
    if let Some(root) = prune.0.take() {
        let _ = tokio::task::spawn_blocking(move || parse::prune_media_cache(&root)).await;
    }
    result
}

/// Prunes the media cache when a body fetch is dropped where it stands, by a
/// caller's timeout or cancellation, and so never reaches its own prune.
struct PruneMediaOnDrop(Option<std::path::PathBuf>);

impl Drop for PruneMediaOnDrop {
    fn drop(&mut self) {
        let Some(root) = self.0.take() else {
            return;
        };
        // On a blocking worker when there is a runtime to lend one.
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => drop(runtime.spawn_blocking(move || parse::prune_media_cache(&root))),
            Err(_) => parse::prune_media_cache(&root),
        }
    }
}

async fn fetch_bodies_unpruned(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
    media_root: std::path::PathBuf,
    account: &str,
    on_message: &mut (dyn FnMut(u32, parse::Message) + Send),
) -> Result<()> {
    session.select(folder).await.context("SELECT")?;
    for chunk in uids.chunks(BODY_FETCH_BATCH) {
        if chunk.is_empty() {
            continue;
        }
        let set = chunk
            .iter()
            .map(|uid| uid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut stream = session
            .uid_fetch(set, "(UID INTERNALDATE BODY.PEEK[])")
            .await
            .context("UID FETCH")?;
        while let Some(item) = stream.next().await {
            let fetch = item.context("UID FETCH item")?;
            let Some(uid) = fetch.uid else { continue };
            let Some(body) = fetch.body() else { continue };
            let media = parse::MediaCtx {
                root: media_root.clone(),
                account: account.to_string(),
                folder: folder.to_string(),
                uid,
            };
            let mut parsed = parse::parse_message_unpruned(body, Some(&media));
            parsed.date = message_date(parsed.date, &fetch);
            on_message(uid, parsed);
        }
        drop(stream);
    }
    Ok(())
}

/// IMAP SEARCH key for the body prefetcher: every message received since `since` (`dd-Mon-yyyy`),
/// read or not. `UNSEEN SINCE` left already-read recent mail cold, so opening it always waited on a
/// full download. Older unread mail stays with the on-demand reader: a mailbox can hold years of
/// it, attachments included.
pub fn prefetch_search_criteria(since: &str) -> String {
    format!("SINCE {since}")
}

/// UIDs worth prefetching full bodies for in `folder`. Uses server-side SEARCH
/// so the set isn't limited to the recent-envelope window and doesn't depend on
/// locally-parsed date strings. Selects the folder.
pub async fn search_prefetch_uids(
    session: &mut Session,
    folder: &str,
    days: u32,
) -> Result<Vec<u32>> {
    session.select(folder).await.context("SELECT")?;
    let since = imap_date_days_ago(days);
    let criteria = prefetch_search_criteria(&since);
    let set: HashSet<u32> = session
        .uid_search(&criteria)
        .await
        .with_context(|| format!("UID SEARCH {criteria}"))?;
    let mut uids: Vec<u32> = set.into_iter().collect();
    uids.sort_unstable();
    Ok(uids)
}

/// Format the IMAP `dd-Mon-yyyy` date for `days` days before now (UTC), for use
/// in `SEARCH SINCE` (which compares by internal date, ignoring time of day).
fn imap_date_days_ago(days: u32) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let secs = now - (days as i64) * 86_400;
    let (y, month, day) = civil_from_days(secs.div_euclid(86_400));
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!("{:02}-{}-{:04}", day, MONTHS[(month - 1) as usize], y)
}

pub(super) fn imap_quote(value: &str) -> String {
    let escaped = value
        .trim()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], " ");
    format!("\"{escaped}\"")
}

/// Convert a count of days since the Unix epoch to a civil (year, month, day),
/// month in 1..=12 and day in 1..=31. Howard Hinnant's `civil_from_days`.
pub(super) fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (y + i64::from(month <= 2), month, day)
}
