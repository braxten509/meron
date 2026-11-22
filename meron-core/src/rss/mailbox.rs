//! Lists subscriptions as conversations and their entries as individual messages.
//! The desktop and mobile mail interfaces consume the same JSON shapes.
//! Pagination and starred queries resolve entries in the same store as regular mail.

use super::content::{first_line, image_filename, image_mime, video_mime};
use super::media_cache::key_is_image;
use super::{RSS_FOLDER_ID, feed_host_label, format_thread_id, parse_thread_id};
use crate::search_query::SearchQuery;
use crate::search_query::local_midnight;
use crate::store;
use anyhow::Result;
use anyhow::anyhow;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use rusqlite::params_from_iter;
use serde_json::Value;
use serde_json::json;

/// List subscriptions as conversations, newest first.
///
/// `filter` mirrors the mail thread list: "unread" keeps feeds with an unread
/// item, "starred" keeps feeds with a starred item, anything else keeps all.
///
/// `query` keeps feeds with an item that matches it, operators included, as
/// they read for a feed: `from:` is the feed or the item's author, `subject:`
/// the item's title. A feed item is addressed to no one, so `to:` matches none.
pub fn recent(
    conn: &Connection,
    account: &str,
    query: &str,
    filter: &str,
    limit: i64,
) -> Result<Vec<Value>> {
    let limit = if limit <= 0 { 50 } else { limit };
    let query = SearchQuery::parse(query);
    let mut sql = String::from(
        "SELECT s.id, s.account, s.url, s.title,
           COALESCE(MAX(NULLIF(json_extract(m.json,'$.published_at'),0)),
                    MAX(NULLIF(json_extract(m.json,'$.updated_at'),0)),
                    MAX(json_extract(m.json,'$.fetched_at')), 0) AS latest_at,
           COALESCE(SUM(CASE WHEN m.seen = 0 THEN 1 ELSE 0 END), 0) AS unread_count,
           EXISTS(SELECT 1 FROM messages sm
                    WHERE sm.account = s.account AND sm.folder = s.id AND sm.starred <> 0)
             AS has_starred_items,
           COALESCE((SELECT subject FROM messages lm WHERE lm.account = s.account AND lm.folder = s.id
              ORDER BY COALESCE(NULLIF(json_extract(lm.json,'$.published_at'),0),
                                NULLIF(json_extract(lm.json,'$.updated_at'),0),
                                json_extract(lm.json,'$.fetched_at')) DESC LIMIT 1), '') AS latest_title,
           COALESCE((SELECT json_extract(lm.json,'$.summary') FROM messages lm WHERE lm.account = s.account AND lm.folder = s.id
              ORDER BY COALESCE(NULLIF(json_extract(lm.json,'$.published_at'),0),
                                NULLIF(json_extract(lm.json,'$.updated_at'),0),
                                json_extract(lm.json,'$.fetched_at')) DESC LIMIT 1), '') AS latest_summary,
           s.json AS sub_json
         FROM subscriptions s
         LEFT JOIN messages m ON m.account = s.account AND m.folder = s.id
         WHERE s.account = ?1 AND s.enabled = 1",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(account.to_string())];
    // Each text condition: any of `columns` holds any of `values`.
    let mut like = |columns: &[&str], values: &[String]| {
        let mut clauses = Vec::new();
        for value in values {
            let pattern = format!("%{}%", value.to_lowercase());
            for column in columns {
                args.push(Box::new(pattern.clone()));
                clauses.push(format!("lower({column}) LIKE ?"));
            }
        }
        sql.push_str(&format!(" AND ({})", clauses.join(" OR ")));
    };
    for text in &query.text {
        like(
            &["s.title", "m.subject", "json_extract(m.json,'$.summary')"],
            std::slice::from_ref(text),
        );
    }
    for from in &query.from {
        like(&["s.title", "m.from_name"], from);
    }
    for subject in &query.subject {
        like(&["m.subject"], subject);
    }
    if !query.to.is_empty() {
        sql.push_str(" AND 0");
    }
    if query.read {
        sql.push_str(" AND m.seen <> 0");
    }
    if query.unread {
        sql.push_str(" AND m.seen = 0");
    }
    if query.starred {
        sql.push_str(" AND m.starred <> 0");
    }
    // Tags are for mail: no feed item carries one.
    if !query.tags.is_empty() {
        sql.push_str(" AND 0");
    }
    // What an item's `has_attachments` counts: its inline images and videos.
    if query.has_attachment {
        sql.push_str(
            " AND (json_array_length(m.json,'$.images') > 0
                   OR json_array_length(m.json,'$.videos') > 0)",
        );
    }
    if let Some(after) = query.after {
        sql.push_str(" AND m.date >= ?");
        args.push(Box::new(local_midnight(after)));
    }
    if let Some(before) = query.before {
        sql.push_str(" AND m.date < ?");
        args.push(Box::new(local_midnight(before)));
    }
    sql.push_str(" GROUP BY s.id");
    match filter {
        "unread" => sql.push_str(" HAVING unread_count > 0"),
        "starred" => sql.push_str(" HAVING has_starred_items"),
        _ => {}
    }
    sql.push_str(" ORDER BY latest_at DESC, s.title COLLATE NOCASE LIMIT ");
    sql.push_str(&limit.to_string());

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|b| b.as_ref())), |row| {
        Ok((
            row.get::<_, String>(0)?,                             // sub id
            row.get::<_, String>(1)?,                             // account
            row.get::<_, String>(2)?,                             // url
            row.get::<_, String>(3)?,                             // title
            row.get::<_, i64>(4)?,                                // latest_at
            row.get::<_, i64>(5)?,                                // unread_count
            row.get::<_, i64>(6)? != 0,                           // has starred items
            row.get::<_, String>(7)?,                             // latest_title
            row.get::<_, Option<String>>(8)?.unwrap_or_default(), // latest_summary
            row.get::<_, Option<String>>(9)?.unwrap_or_default(), // sub json extras
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (
            sub_id,
            acct,
            url,
            title,
            latest_at,
            unread,
            has_starred_items,
            latest_title,
            latest_summary,
            sub_json,
        ) = row?;
        let icon_key = serde_json::from_str::<Value>(&sub_json)
            .ok()
            .and_then(|v| v["icon_key"].as_str().map(str::to_string))
            .unwrap_or_default();
        out.push(thread_message(
            &acct,
            &sub_id,
            &title,
            &url,
            latest_at,
            unread,
            has_starred_items,
            &latest_title,
            &latest_summary,
            &icon_key,
        ));
    }
    Ok(out)
}

/// Paginated newest-first slice of an RSS thread, returned ascending by
/// (sort_ts, item_key). `before_cursor` is `Some((ts, item_key))` to fetch the
/// next older page; `None` returns the most recent page. When `limit` is
/// `None`, the full thread is returned (caller did not opt into pagination).
pub fn read_thread_page(
    conn: &Connection,
    thread_id: &str,
    before_cursor: Option<(i64, String)>,
    limit: Option<u32>,
) -> Result<(Vec<Value>, Option<String>)> {
    let (account, sub_id) =
        parse_thread_id(thread_id).ok_or_else(|| anyhow!("invalid RSS thread id: {thread_id}"))?;
    let sub_title: String = conn
        .query_row(
            "SELECT title FROM subscriptions WHERE id = ?1 AND account = ?2",
            params![sub_id, account],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default();

    let load_remote_images = store::load_remote_images(conn, &account).unwrap_or(false);

    // Newest-first when paginating so the cursor walks backwards in time;
    // unpaginated callers still want oldest-first to preserve old behavior.
    let (sql, ascending) = match limit {
        Some(_) => (
            "SELECT msg_id, subject, seen, json,
                    COALESCE(NULLIF(json_extract(json,'$.published_at'),0),
                             NULLIF(json_extract(json,'$.updated_at'),0),
                             json_extract(json,'$.fetched_at')) AS sort_ts,
                    starred
               FROM messages
              WHERE account = ?1 AND folder = ?2
                AND (?3 IS NULL
                     OR sort_ts < ?3
                     OR (sort_ts = ?3 AND msg_id < ?4))
              ORDER BY sort_ts DESC, msg_id DESC
              LIMIT ?5",
            false,
        ),
        None => (
            "SELECT msg_id, subject, seen, json,
                    COALESCE(NULLIF(json_extract(json,'$.published_at'),0),
                             NULLIF(json_extract(json,'$.updated_at'),0),
                             json_extract(json,'$.fetched_at')) AS sort_ts,
                    starred
               FROM messages
              WHERE account = ?1 AND folder = ?2
                AND (?3 IS NULL
                     OR sort_ts < ?3
                     OR (sort_ts = ?3 AND msg_id < ?4))
              ORDER BY sort_ts ASC, msg_id ASC",
            true,
        ),
    };

    let mut stmt = conn.prepare(sql)?;
    let cursor_ts = before_cursor.as_ref().map(|(t, _)| *t);
    let cursor_key = before_cursor
        .as_ref()
        .map(|(_, key)| key.as_str())
        .unwrap_or("");
    let probe_limit = limit
        .map(|n| n.saturating_add(1) as i64)
        .unwrap_or(i64::MAX);

    let mut map_row = |row: &rusqlite::Row<'_>| {
        Ok((
            row.get::<_, String>(0)?,                   // item key
            row.get::<_, String>(1)?,                   // subject
            row.get::<_, i64>(2)?,                      // seen
            row.get::<_, String>(3)?,                   // json
            row.get::<_, Option<i64>>(4)?.unwrap_or(0), // sort_ts
            row.get::<_, i64>(5)? != 0,                 // starred
        ))
    };
    let rows = match limit {
        Some(_) => stmt.query_map(
            params![account, sub_id, cursor_ts, cursor_key, probe_limit],
            &mut map_row,
        )?,
        None => stmt.query_map(
            params![account, sub_id, cursor_ts, cursor_key],
            &mut map_row,
        )?,
    };

    let mut rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = match limit {
        Some(n) if rows.len() > n as usize => {
            rows.truncate(n as usize);
            true
        }
        _ => false,
    };

    // After truncation, the oldest item in this newest-first page is the
    // cursor for the next (older) page.
    let next_cursor = if has_more {
        rows.last()
            .map(|(item_key, _, _, _, ts, _)| format!("ts:{}:{}", ts, item_key))
    } else {
        None
    };

    if !ascending {
        rows.reverse();
    }

    let mut out = Vec::with_capacity(rows.len());
    for (item_key, subject, seen, json, _ts, starred) in rows {
        let body_html = serde_json::from_str::<Value>(&json)
            .ok()
            .and_then(|value| value["body_html"].as_str().map(str::to_string));
        out.push(item_message(
            &account,
            &sub_id,
            &sub_title,
            &item_key,
            &subject,
            seen == 0,
            starred,
            &json,
            body_html.as_deref(),
            load_remote_images,
        ));
    }
    Ok((out, next_cursor))
}

/// Final bridge-shaped `Message` for every starred RSS item across all
/// accounts/subscriptions, newest first (same sort_ts ordering as
/// `read_thread_page`).
pub fn starred_items(conn: &Connection, limit: i64) -> Result<Vec<Value>> {
    let limit = if limit <= 0 { 200 } else { limit };
    let mut stmt = conn.prepare(
        "SELECT m.account, m.folder, s.title, m.msg_id, m.subject, m.seen, m.json
           FROM messages m
           JOIN subscriptions s ON s.account = m.account AND s.id = m.folder
          WHERE m.starred <> 0
          ORDER BY COALESCE(NULLIF(json_extract(m.json,'$.published_at'),0),
                            NULLIF(json_extract(m.json,'$.updated_at'),0),
                            json_extract(m.json,'$.fetched_at')) DESC, m.msg_id DESC
          LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok((
            row.get::<_, String>(0)?, // account
            row.get::<_, String>(1)?, // sub id
            row.get::<_, String>(2)?, // sub title
            row.get::<_, String>(3)?, // item key
            row.get::<_, String>(4)?, // subject
            row.get::<_, i64>(5)?,    // seen
            row.get::<_, String>(6)?, // json
        ))
    })?;

    let mut remote_images: std::collections::HashMap<String, bool> = Default::default();
    let mut out = Vec::new();
    for row in rows {
        let (account, sub_id, sub_title, item_key, subject, seen, json) = row?;
        let load_remote_images = *remote_images
            .entry(account.clone())
            .or_insert_with(|| store::load_remote_images(conn, &account).unwrap_or(false));
        let body_html = serde_json::from_str::<Value>(&json)
            .ok()
            .and_then(|value| value["body_html"].as_str().map(str::to_string));
        out.push(item_message(
            &account,
            &sub_id,
            &sub_title,
            &item_key,
            &subject,
            seen == 0,
            true,
            &json,
            body_html.as_deref(),
            load_remote_images,
        ));
    }
    Ok(out)
}

/// The single synthetic Inbox folder for an RSS account, with its unread count.
pub fn folders(conn: &Connection, account: &str) -> Result<Vec<Value>> {
    let unread = unread_count(conn, account)?;
    Ok(vec![json!({
        "id": RSS_FOLDER_ID,
        "account_id": account,
        "name": "Inbox",
        "role": "inbox",
        "unread": unread,
    })])
}

pub fn unread_count(conn: &Connection, account: &str) -> Result<i64> {
    Ok(conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE account = ?1 AND seen = 0",
            params![account],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

// ---- Presentation (final bridge JSON) ---------------------------------------

#[allow(clippy::too_many_arguments)]
fn thread_message(
    account: &str,
    sub_id: &str,
    title: &str,
    url: &str,
    latest_at: i64,
    unread: i64,
    has_starred_items: bool,
    latest_title: &str,
    latest_summary: &str,
    icon_key: &str,
) -> Value {
    let mut preview = latest_title.trim().to_string();
    if !latest_summary.is_empty() {
        if !preview.is_empty() {
            preview.push_str(" - ");
        }
        preview.push_str(&first_line(latest_summary));
    }
    let thread_id = format_thread_id(account, sub_id);
    json!({
        "id": thread_id,
        "account_id": account,
        "folder_id": RSS_FOLDER_ID,
        "thread_id": thread_id,
        "from_name": title,
        "from_addr": feed_host_label(url),
        "feed_url": url,
        "feed_icon": icon_key,
        "subject": title,
        "preview": preview,
        "date": latest_at,
        "unread": unread > 0,
        "unread_count": unread,
        // A feed is not itself starred; this only describes its contained items.
        "has_starred_items": has_starred_items,
    })
}

fn item_message(
    account: &str,
    sub_id: &str,
    sub_title: &str,
    item_key: &str,
    subject: &str,
    unread: bool,
    starred: bool,
    extra_json: &str,
    body_html: Option<&str>,
    load_remote_images: bool,
) -> Value {
    let json: Value = serde_json::from_str(extra_json).unwrap_or_else(|_| json!({}));
    let summary = json["summary"].as_str().unwrap_or_default();
    let content = json["content"].as_str().unwrap_or_default();
    let link = json["link"].as_str().unwrap_or_default();
    // Inline images as attachments. A cached image carries a media `key` (served
    // from disk, no third-party fetch); an uncached one carries only its `url`,
    // which the UI gates on the account's "load remote images" preference.
    let mut attachments: Vec<Value> = json["images"]
        .as_array()
        .map(|images| {
            let root = crate::parse::media_root();
            images
                .iter()
                .filter_map(|image| {
                    let url = image["url"].as_str()?;
                    // Trust the cached key only if it's a real image still on disk;
                    // otherwise fall back to the remote URL. Keys persisted before
                    // image validation can point at poison (".plain" error bodies
                    // from expired signed URLs), which would render as broken images.
                    let key = image["key"].as_str().filter(|key| key_is_image(&root, key));
                    Some(json!({
                        "filename": image_filename(url),
                        "mime": image_mime(url),
                        "size": 0,
                        "key": key,
                        "url": url,
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    // Inline videos as remote-only attachments (no cached key). The UI keys off
    // the `video/*` mime to render a player instead of an image.
    if let Some(images) = json["videos"].as_array() {
        for vid in images {
            let Some(url) = vid["url"].as_str() else {
                continue;
            };
            attachments.push(json!({
                "filename": image_filename(url),
                "mime": video_mime(url),
                "size": 0,
                "key": Value::Null,
                "url": url,
            }));
        }
    }
    let published = json["published_at"].as_i64().unwrap_or(0);
    let updated = json["updated_at"].as_i64().unwrap_or(0);
    let fetched = json["fetched_at"].as_i64().unwrap_or(0);

    let mut body = String::new();
    if let Some(html) = body_html {
        body = crate::parse::html_to_text(html);
    }
    if body.trim().is_empty() {
        body = summary.trim().to_string();
        if !content.trim().is_empty() {
            body = content.to_string();
        }
    }
    let ts = if published != 0 {
        published
    } else if updated != 0 {
        updated
    } else {
        fetched
    };
    // Iframe-ready original HTML for the reader's HTML mode (remote images gated on
    // the account setting, like mail). Omitted when the feed gave no HTML.
    let body_html = body_html
        .filter(|html| !html.trim().is_empty())
        .map(|html| crate::parse::prepare_html(html, load_remote_images));

    let thread_id = format_thread_id(account, sub_id);
    json!({
        "id": format!("{thread_id}#{item_key}"),
        "account_id": account,
        "folder_id": RSS_FOLDER_ID,
        "thread_id": thread_id,
        "from_name": sub_title,
        "from_addr": feed_host_label(link),
        // The item's original page; the reader offers to open or copy it.
        "link": link,
        "subject": subject,
        "preview": first_line(summary),
        "body": body,
        "body_html": body_html,
        "date": ts,
        "unread": unread,
        "starred": starred,
        "has_attachments": !attachments.is_empty(),
        "attachments": attachments,
    })
}
