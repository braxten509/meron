//! RSS and Atom subscriptions in the shared accounts and messages database.
//!
//! Feed and item identities retain the legacy RSSStore encoding. Hashes use the
//! first twelve bytes of SHA-256 and URL-safe base64 without padding. This keeps
//! subscriptions imported from rss.sqlite stable across synchronization.
//!
//! The mailbox module returns the message and folder JSON used by the desktop
//! and mobile clients. Sync returns newly stored entries for notifications.

mod content;
mod fetch;
mod flags;
mod mailbox;
mod media_cache;
mod opml;

use anyhow::{Result, anyhow};
use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::time::Duration;

use crate::store::{self, RssItemExtra};
use content::{first_line, text_content};
use fetch::{fetch_feed, parse_items};
use media_cache::{cache_feed_icon, cache_feed_images, prune_feed_media};

pub use flags::{
    mark_account_read, mark_items_read, mark_items_starred, mark_thread_read, mark_thread_starred,
};
pub use mailbox::{folders, read_thread_page, recent, starred_items, unread_count};
pub use opml::{export_opml, import_opml};

const RSS_FOLDER_ID: &str = "inbox";
const DEFAULT_RSS_ACCOUNT_TITLE: &str = "RSS";
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// A subscription as needed for sync and presentation.
struct Subscription {
    id: String,
    account: String,
    url: String,
    title: String,
    etag: String,
    last_modified: String,
}

struct ParsedItem {
    item_key: String,
    title: String,
    unread: bool,
    /// Raw HTML body of the entry (content, else summary), for the reader's HTML
    /// mode. `None` when the feed gave no HTML. Stored in message `json`.
    content_html: Option<String>,
    extra: RssItemExtra,
}

/// A feed entry that landed in the store for the first time during a sync.
/// Carries everything a "new items" notification needs, so callers never have to
/// guess from the store which item triggered the sync — an item's `date` is its
/// *published* date, so the newest arrival is often not the newest-dated row.
pub struct NewItem {
    /// Subscription id, which is both the item's `folder` and the feed thread's
    /// key (`<account>#rss#<sub>`) that a notification tap should open.
    pub subscription_id: String,
    /// The entry's stable key (its guid/link hash). A feed is one thread, so the
    /// subscription id does not tell two of its entries apart — this does, and
    /// it is what keeps two same-titled arrivals from sharing a notification.
    pub item_key: String,
    /// The feed's display title, used as the notification's "sender".
    pub feed_title: String,
    pub title: String,
    /// Body snippet for the notification, empty when the entry carried no text.
    pub preview: String,
    /// Item date as epoch seconds, matching the stored `date` column.
    pub date: i64,
}

// ---- Public API -------------------------------------------------------------

/// Create a new RSS account (fresh stable id) and, if a feed URL is supplied,
/// add it as the account's first feed. Returns the bridge `Account` JSON. Each
/// call creates a *distinct* account — no name-based grouping — so users can have
/// multiple RSS accounts (even same-named) and rename them later.
pub fn add(db: &Mutex<Connection>, feed_url: &str, display_name: &str) -> Result<Value> {
    let account_title = {
        let t = display_name.trim();
        if t.is_empty() {
            DEFAULT_RSS_ACCOUNT_TITLE.to_string()
        } else {
            t.to_string()
        }
    };
    let account_id = new_account_id();
    let feed_url = feed_url.trim();
    // Fetch the first feed (if any) *before* writing, so a bad URL doesn't leave
    // an empty account behind.
    let parsed = if feed_url.is_empty() {
        None
    } else {
        Some(fetch_parsed(feed_url)?)
    };
    // A fresh RSS account defaults to images-on; we defer caching images to the
    // background sync so the account addition is fast and doesn't block/timeout.

    let now = now_unix();
    let conn = db.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO accounts(id, engine, provider, display_name, config, created_at, updated_at, sort_order)
         VALUES(?1, 'rss', 'rss', ?2, '{}', ?3, ?3, COALESCE((SELECT MAX(sort_order) + 1 FROM accounts), 0))
         ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, updated_at = excluded.updated_at",
        params![account_id, account_title, now],
    )?;
    if let Some(parsed) = parsed {
        store_parsed_feed(&tx, &account_id, &parsed)?;
    }
    tx.commit()?;
    Ok(account_json(&account_id, &account_title))
}

/// Add a feed to an existing RSS account. Fetches + parses (with HTML discovery
/// fallback) before writing the subscription and its items. Returns the resolved
/// feed title.
pub fn add_feed(db: &Mutex<Connection>, account: &str, feed_url: &str) -> Result<Value> {
    {
        let conn = db.lock().unwrap();
        match store::account_engine(&conn, account)?.as_deref() {
            Some("rss") => {}
            Some(other) => return Err(anyhow!("account {account} is not RSS (engine={other})")),
            None => return Err(anyhow!("unknown account: {account}")),
        }
    }
    let parsed = fetch_parsed(feed_url)?;
    let feed_title = parsed.feed_title.clone();
    // Defer caching images to the background sync so the feed addition is fast
    // and doesn't block/timeout.
    let conn = db.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    store_parsed_feed(&tx, account, &parsed)?;
    tx.commit()?;
    Ok(json!({ "ok": true, "feed_title": feed_title }))
}

/// Remove a single feed (subscription) and its cached items from an RSS account.
/// Takes the subscription's thread id ("<account>#rss#<sub>"), the same id the
/// UI carries for the feed's thread row. The account itself is left in place.
pub fn remove_feed(conn: &Connection, thread_id: &str) -> Result<Value> {
    let (account, sub_id) =
        parse_thread_id(thread_id).ok_or_else(|| anyhow!("invalid RSS thread id: {thread_id}"))?;
    let tx = conn.unchecked_transaction()?;
    // RSS items are stored with the subscription id as their `folder`.
    tx.execute(
        "DELETE FROM messages WHERE account = ?1 AND folder = ?2",
        params![account, sub_id],
    )?;
    tx.execute(
        "DELETE FROM subscriptions WHERE id = ?1 AND account = ?2",
        params![sub_id, account],
    )?;
    tx.commit()?;
    Ok(json!({ "ok": true }))
}

/// Move a feed subscription and its cached items to another RSS account. This is
/// a metadata reassignment, not a delete/re-add, so read/starred item state and
/// sync metadata stay intact.
pub fn move_feed(conn: &Connection, thread_id: &str, target_account: &str) -> Result<Value> {
    let (source_account, sub_id) =
        parse_thread_id(thread_id).ok_or_else(|| anyhow!("invalid RSS thread id: {thread_id}"))?;
    let target_account = target_account.trim();
    if target_account.is_empty() {
        return Err(anyhow!("target account required"));
    }

    match store::account_engine(conn, target_account)?.as_deref() {
        Some("rss") => {}
        Some(other) => {
            return Err(anyhow!(
                "account {target_account} is not RSS (engine={other})"
            ));
        }
        None => return Err(anyhow!("unknown account: {target_account}")),
    }

    let exists = conn
        .query_row(
            "SELECT 1 FROM subscriptions WHERE id = ?1 AND account = ?2",
            params![sub_id, source_account],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Err(anyhow!("unknown feed: {thread_id}"));
    }

    if source_account == target_account {
        return Ok(json!({
            "ok": true,
            "moved": 0,
            "thread_id": thread_id,
            "source_account_id": source_account,
            "target_account_id": target_account
        }));
    }

    let tx = conn.unchecked_transaction()?;
    let now = now_unix();
    tx.execute(
        "UPDATE subscriptions SET account = ?3, updated_at = ?4 WHERE id = ?1 AND account = ?2",
        params![sub_id, source_account, target_account, now],
    )?;
    let moved = tx.execute(
        "UPDATE messages SET account = ?3 WHERE account = ?1 AND folder = ?2",
        params![source_account, sub_id, target_account],
    )?;
    tx.commit()?;

    Ok(json!({
        "ok": true,
        "moved": moved,
        "thread_id": format_thread_id(target_account, &sub_id),
        "source_account_id": source_account,
        "target_account_id": target_account
    }))
}

/// A fetched + parsed feed, ready to persist (no DB lock held during the fetch).
struct FetchedParsed {
    resolved_url: String,
    feed_title: String,
    site_url: String,
    /// Feed-supplied icon/logo URL (favicon preferred over banner logo), empty
    /// when the feed declared neither. Stored in the subscription `json`.
    icon_url: String,
    items: Vec<ParsedItem>,
}

/// The best icon URL a feed declares for itself: prefer `<icon>` (a square
/// favicon, what an avatar wants) over `<logo>` (a wide banner), empty when
/// neither is present.
fn feed_icon_url(feed: &feed_rs::model::Feed) -> String {
    feed.icon
        .as_ref()
        .or(feed.logo.as_ref())
        .map(|image| image.uri.trim().to_string())
        .unwrap_or_default()
}

/// Serialize a subscription's `json` extras column: the feed-declared icon URL
/// and, once downloaded, its cached media key. New per-feed metadata can be
/// added as fields here without a schema migration.
fn subscription_json(icon_url: &str, icon_key: &str) -> String {
    json!({ "icon_url": icon_url, "icon_key": icon_key }).to_string()
}

fn fetch_parsed(feed_url: &str) -> Result<FetchedParsed> {
    let normalized = normalize_feed_url(feed_url)?;
    let fetched = fetch_feed(&normalized, "", "")?;
    let feed = fetched
        .feed
        .ok_or_else(|| anyhow!("feed returned no content"))?;
    let resolved = fetched.resolved_url;
    let feed_title = {
        let t = text_content(&feed.title);
        if t.trim().is_empty() {
            feed_host_label(&resolved)
        } else {
            t.trim().to_string()
        }
    };
    let site_url = feed
        .links
        .first()
        .map(|link| link.href.clone())
        .unwrap_or_default();
    let icon_url = feed_icon_url(&feed);
    let items = parse_items(&feed, now_unix());
    Ok(FetchedParsed {
        resolved_url: resolved,
        feed_title,
        site_url,
        icon_url,
        items,
    })
}

fn store_parsed_feed(tx: &Connection, account: &str, parsed: &FetchedParsed) -> Result<()> {
    let sub_id = rss_subscription_id(&parsed.resolved_url);
    let now = now_unix();
    tx.execute(
        "INSERT INTO subscriptions(id, account, url, title, site_url, feed_title, json, enabled, created_at, updated_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)
         ON CONFLICT(id) DO UPDATE SET
           account = excluded.account,
           url = excluded.url,
           title = excluded.title,
           site_url = excluded.site_url,
           feed_title = excluded.feed_title,
           json = excluded.json,
           enabled = 1,
           updated_at = excluded.updated_at",
        params![
            sub_id,
            account,
            parsed.resolved_url,
            parsed.feed_title,
            parsed.site_url,
            parsed.feed_title,
            subscription_json(&parsed.icon_url, ""),
            now
        ],
    )?;
    for item in &parsed.items {
        // Initial feed load: store items, but don't treat them as "new" arrivals.
        store::upsert_rss_item(
            tx,
            account,
            &sub_id,
            &item.item_key,
            &item.title,
            item.unread,
            item.content_html.as_deref(),
            &item.extra,
        )?;
    }
    Ok(())
}

/// Sync every enabled subscription and return its newly stored entries.
///
/// Each feed uses its saved ETag or Last-Modified value for the conditional request.
/// New arrivals are returned newest first for the notification clients.
pub fn sync_account(db: &Mutex<Connection>, account: &str) -> Result<Vec<NewItem>> {
    let subs = {
        let conn = crate::log::timed_db_lock(db, "rss.sync.load_subscriptions");
        load_subscriptions(&conn, account)?
    };
    let mut new_items: Vec<NewItem> = Vec::new();
    for sub in subs {
        match sync_subscription(db, &sub) {
            Ok(items) => new_items.extend(items),
            Err(e) => {
                let conn = crate::log::timed_db_lock(db, "rss.sync.save_error");
                let _ = conn.execute(
                    "UPDATE subscriptions SET last_error = ?2, updated_at = ?3 WHERE id = ?1",
                    params![sub.id, format!("{e:#}"), now_unix()],
                );
            }
        }
    }
    prune_feed_media(&crate::parse::media_root());
    new_items.sort_by_key(|item| std::cmp::Reverse(item.date));
    Ok(new_items)
}

/// Build the same notification payload as [`crate::mail_model::new_messages_detail`].
///
/// The top-level fields describe the newest arrival. The messages array identifies
/// individual arrivals for per-message notifications. Pass entries in the order
/// returned by [`sync_account`].
pub fn new_items_detail(
    account: &str,
    account_name: &str,
    muted: bool,
    items: &[NewItem],
) -> Option<Value> {
    let latest = items.first()?;
    let messages: Vec<Value> = items
        .iter()
        .take(crate::mail_model::NEW_MESSAGES_DETAIL_MAX)
        .map(|item| {
            json!({
                // RSS rows carry no IMAP uid; `itemKey` is what identifies one,
                // since a feed's entries all share its thread key.
                "uid": 0,
                "itemKey": item.item_key,
                "from": item.feed_title,
                "subject": item.title,
                "preview": item.preview,
                "threadKey": item.subscription_id,
                "date": item.date,
            })
        })
        .collect();
    Some(json!({
        "account": account,
        "accountName": account_name,
        "folder": RSS_FOLDER_ID,
        "count": items.len(),
        "muted": muted,
        "from": latest.feed_title,
        "subject": latest.title,
        "preview": latest.preview,
        // The subscription id: a feed is one thread, so this opens the feed the
        // newest arrival landed in.
        "threadKey": latest.subscription_id,
        "messages": messages,
    }))
}

/// Notification snippet for an item: its summary, else its content body.
fn item_preview(extra: &RssItemExtra) -> String {
    if !extra.summary.trim().is_empty() {
        first_line(&extra.summary)
    } else {
        first_line(&extra.content)
    }
}

/// Read an account's resolved "load remote images" preference under a short lock.
fn db_load_remote_images(db: &Mutex<Connection>, account: &str) -> bool {
    let conn = db.lock().unwrap();
    store::load_remote_images(&conn, account).unwrap_or(false)
}

fn sync_subscription(db: &Mutex<Connection>, sub: &Subscription) -> Result<Vec<NewItem>> {
    let now = now_unix();
    let fetched = fetch_feed(&sub.url, &sub.etag, &sub.last_modified)?;
    if fetched.not_modified {
        let conn = crate::log::timed_db_lock(db, "rss.sync.not_modified");
        conn.execute(
            "UPDATE subscriptions SET last_sync_at = ?2, last_error = '', updated_at = ?2 WHERE id = ?1",
            params![sub.id, now],
        )?;
        return Ok(Vec::new());
    }
    let feed = fetched
        .feed
        .ok_or_else(|| anyhow!("feed returned no content"))?;
    let mut items = parse_items(&feed, now);
    // Download inline images (off the DB lock) for image-enabled accounts so they
    // render from disk and never re-fetch from the origin on each open.
    if db_load_remote_images(db, &sub.account) {
        cache_feed_images(&sub.account, &sub.id, &mut items);
    }
    let feed_title = text_content(&feed.title);
    let site_url = feed
        .links
        .first()
        .map(|link| link.href.clone())
        .unwrap_or_default();
    let icon_url = feed_icon_url(&feed);
    // Cache the feed's own icon to disk (off the DB lock) so the avatar renders
    // from /media with no remote fetch. Unconditional: it's first-party branding,
    // not the third-party content the remote-images gate is meant to hold back.
    let icon_key = cache_feed_icon(&sub.account, &sub.id, &icon_url).unwrap_or_default();
    let title = if sub.title.is_empty() {
        feed_title.trim().to_string()
    } else {
        sub.title.clone()
    };

    let conn = crate::log::timed_db_lock(db, "rss.sync.persist_items");
    let tx = conn.unchecked_transaction()?;
    let mut new_items = Vec::new();
    for item in &items {
        if store::upsert_rss_item(
            &tx,
            &sub.account,
            &sub.id,
            &item.item_key,
            &item.title,
            item.unread,
            item.content_html.as_deref(),
            &item.extra,
        )? {
            new_items.push(NewItem {
                subscription_id: sub.id.clone(),
                item_key: item.item_key.clone(),
                // The user's own title for the feed when they renamed it, so the
                // notification matches the thread row they will land on.
                feed_title: title.clone(),
                title: item.title.clone(),
                preview: item_preview(&item.extra),
                date: store::item_date_epoch(
                    item.extra.published_at,
                    item.extra.updated_at,
                    item.extra.fetched_at,
                ),
            });
        }
    }
    tx.execute(
        "UPDATE subscriptions
         SET title = ?2, url = ?3, site_url = ?4, feed_title = ?5, json = ?6,
             etag = ?7, last_modified = ?8, last_sync_at = ?9, last_error = '', updated_at = ?9
         WHERE id = ?1",
        params![
            sub.id,
            title,
            fetched.resolved_url,
            site_url,
            feed_title.trim(),
            subscription_json(&icon_url, &icon_key),
            fetched.etag,
            fetched.last_modified,
            now
        ],
    )?;
    tx.commit()?;
    Ok(new_items)
}

/// Bridge `Account` JSON for an RSS account.
pub fn account_json(id: &str, title: &str) -> Value {
    json!({
        "id": id,
        "email": format!("{id}.local"),
        "display_name": title,
        "provider": "rss",
        "auth_type": "rss",
        "imap_host": "",
        "imap_port": 0,
        "smtp_host": "",
        "smtp_port": 0,
        "tls": false,
    })
}

// ---- Persistence helpers ----------------------------------------------------

fn load_subscriptions(conn: &Connection, account: &str) -> Result<Vec<Subscription>> {
    let mut stmt = conn.prepare(
        "SELECT id, account, url, title, etag, last_modified FROM subscriptions
         WHERE account = ?1 AND enabled = 1 ORDER BY title COLLATE NOCASE, url",
    )?;
    let rows = stmt.query_map(params![account], |row| {
        Ok(Subscription {
            id: row.get(0)?,
            account: row.get(1)?,
            url: row.get(2)?,
            title: row.get(3)?,
            etag: row.get(4)?,
            last_modified: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub(crate) fn normalize_feed_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(anyhow!("feed URL required"));
    }
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let mut parsed = url::Url::parse(&with_scheme).map_err(|_| anyhow!("invalid feed URL"))?;
    if parsed.host_str().unwrap_or("").is_empty() {
        return Err(anyhow!("invalid feed URL"));
    }
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(anyhow!("feed URL must use http or https"));
    }
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

/// A fresh, stable, name-independent RSS account id. Carries the "rss-" prefix
/// the bridge routes on, and is random so accounts can be renamed and even share
/// a display name.
fn new_account_id() -> String {
    format!("rss-{}", uuid::Uuid::new_v4().simple())
}

pub(crate) fn rss_subscription_id(feed_url: &str) -> String {
    format!("feed-{}", stable_hash(feed_url))
}

fn format_thread_id(account: &str, sub_id: &str) -> String {
    format!("{account}#rss#{sub_id}")
}

fn parse_thread_id(thread_id: &str) -> Option<(String, String)> {
    let (account, rest) = thread_id.split_once("#rss#")?;
    if account.is_empty() || rest.is_empty() {
        return None;
    }
    let sub_id = rest.split('#').next().unwrap_or("");
    if sub_id.is_empty() {
        return None;
    }
    Some((account.to_string(), sub_id.to_string()))
}

fn stable_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let sum = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&sum[..12])
}

fn feed_host_label(raw: &str) -> String {
    url::Url::parse(raw)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|h| h.trim_start_matches("www.").to_string())
        })
        .unwrap_or_default()
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
