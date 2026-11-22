//! Updates the stored identities, read flags and stars of feed items.
//! The feed sync and reader paths use the same SQLite representation.
//! Thread-level updates can target the newest item or the selected conversation.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

pub fn update_rss_thread_seen(
    conn: &Connection,
    account: &str,
    subscription_id: &str,
    seen: bool,
) -> Result<()> {
    conn.execute(
        "UPDATE messages SET seen = ?3 WHERE account = ?1 AND folder = ?2",
        params![account, subscription_id, seen as i64],
    )?;
    Ok(())
}

/// The feed equivalent of [`newest_thread_uids`]: flag only the newest item, so
/// "mark unread" on a feed brings back one item rather than claiming every item
/// in it is unread.
pub fn update_rss_newest_item_seen(
    conn: &Connection,
    account: &str,
    subscription_id: &str,
    seen: bool,
) -> Result<()> {
    conn.execute(
        "UPDATE messages SET seen = ?3 WHERE rowid = (
             SELECT rowid FROM messages WHERE account = ?1 AND folder = ?2
             ORDER BY date DESC, uid DESC LIMIT 1
         )",
        params![account, subscription_id, seen as i64],
    )?;
    Ok(())
}

pub fn update_rss_thread_starred(
    conn: &Connection,
    account: &str,
    subscription_id: &str,
    starred: bool,
) -> Result<()> {
    conn.execute(
        "UPDATE messages SET starred = ?3 WHERE account = ?1 AND folder = ?2",
        params![account, subscription_id, starred as i64],
    )?;
    Ok(())
}

pub fn update_rss_item_seen(
    conn: &Connection,
    account: &str,
    subscription_id: &str,
    item_key: &str,
    seen: bool,
) -> Result<()> {
    conn.execute(
        "UPDATE messages SET seen = ?4 WHERE account = ?1 AND folder = ?2 AND msg_id = ?3",
        params![account, subscription_id, item_key, seen as i64],
    )?;
    Ok(())
}

pub fn update_rss_item_starred(
    conn: &Connection,
    account: &str,
    subscription_id: &str,
    item_key: &str,
    starred: bool,
) -> Result<()> {
    conn.execute(
        "UPDATE messages SET starred = ?4 WHERE account = ?1 AND folder = ?2 AND msg_id = ?3",
        params![account, subscription_id, item_key, starred as i64],
    )?;
    Ok(())
}

/// Per-item RSS fields that don't map to a typed mail column; stored as the
/// message row's `json` JSON.
pub struct RssItemExtra {
    pub author: String,
    pub link: String,
    pub summary: String,
    pub content: String,
    /// Inline images lifted from the item's HTML. Each carries its source `url`
    /// and, once downloaded to the media dir, a local `key` served at `/media`.
    pub images: Vec<RssMedia>,
    /// Inline videos lifted from the item's HTML. Remote-only (rendered straight
    /// from their source `url`); not cached to disk, so `key` stays `None`.
    pub videos: Vec<RssMedia>,
    pub published_at: i64,
    pub updated_at: i64,
    pub fetched_at: i64,
}

/// One inline feed media item (image or video): its remote source and, when
/// cached locally, its media key served at `/media`.
pub struct RssMedia {
    pub url: String,
    pub key: Option<String>,
}

/// Upsert one RSS item as a message row. `folder` is the subscription id and
/// `msg_id` the stable item key; the feed-specific payload lives in `json`.
/// Updates preserve the existing `seen` flag (don't un-read on refetch).
pub fn upsert_rss_item(
    conn: &Connection,
    account: &str,
    subscription_id: &str,
    item_key: &str,
    title: &str,
    unread: bool,
    body_html: Option<&str>,
    extra: &RssItemExtra,
) -> Result<bool> {
    // Tell new arrivals apart from re-syncs of items we already stored, so callers
    // can surface a "new items" notification only for genuinely new entries.
    let is_new = conn
        .query_row(
            "SELECT 1 FROM messages WHERE account = ?1 AND folder = ?2 AND msg_id = ?3",
            params![account, subscription_id, item_key],
            |_| Ok(()),
        )
        .optional()?
        .is_none();
    let extra_json = json!({
        "author": extra.author,
        "link": extra.link,
        "summary": extra.summary,
        "content": extra.content,
        "body_html": body_html,
        "images": extra.images.iter()
            .map(|image| json!({ "url": image.url, "key": image.key }))
            .collect::<Vec<_>>(),
        "videos": extra.videos.iter()
            .map(|vid| json!({ "url": vid.url, "key": vid.key }))
            .collect::<Vec<_>>(),
        "published_at": extra.published_at,
        "updated_at": extra.updated_at,
        "fetched_at": extra.fetched_at,
    })
    .to_string();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, thread_key, json)
         VALUES(?1, ?2, ?3, 0, ?4, ?5, '', ?6, ?7, ?2, ?8)
         ON CONFLICT(account, folder, msg_id) DO UPDATE SET
           subject = excluded.subject,
           date    = excluded.date,
           json   = excluded.json",
        params![
            account,
            subscription_id,
            item_key,
            title,
            extra.author,
            item_date_epoch(extra.published_at, extra.updated_at, extra.fetched_at),
            (!unread) as i64,
            extra_json,
        ],
    )?;
    Ok(is_new)
}

/// Best available timestamp for an RSS item as epoch seconds (0 when none),
/// preferring published > updated > fetched. Stored in the `date` column so RSS
/// rows sort alongside mail.
pub(crate) fn item_date_epoch(published: i64, updated: i64, fetched: i64) -> i64 {
    if published != 0 {
        published
    } else if updated != 0 {
        updated
    } else {
        fetched
    }
}
