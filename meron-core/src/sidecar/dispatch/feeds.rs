//! Handles RSS commands received through the desktop sidecar.
//! The selected operation reads or changes feed accounts and subscriptions.
//! The core returns the data needed to refresh the desktop feed views.

use serde_json::{Value, json};
use std::sync::Arc;

use meron_core::engine::Engine;
use meron_core::protocol::Request;
use meron_core::rss;

use crate::Writer;
use crate::sidecar::params::*;

/// Handle RSS accounts and feeds: `account.addRss`, `feed.*`, `rss.*`.
pub(crate) async fn dispatch(
    engine: &Arc<Engine>,
    request: &Request,
    _out: &Writer,
) -> anyhow::Result<Value> {
    let parameters = &request.params;
    match request.method.as_str() {
        // Add an RSS feed: fetch + parse + persist on the blocking pool (network
        // I/O), returning the bridge Account JSON.
        "account.addRss" => {
            let feed_url = required_string(parameters, "feed_url")?;
            let display_name = required_string(parameters, "display_name").unwrap_or_default();
            let engine = engine.clone();
            let account =
                tokio::task::spawn_blocking(move || rss::add(&engine.db, &feed_url, &display_name))
                    .await??;
            Ok(json!({ "account": account }))
        }

        // Add a feed to an existing RSS account (network on the blocking pool).
        "feed.add" => {
            let account = required_string(parameters, "account")?;
            let feed_url = required_string(parameters, "feed_url")?;
            let engine = engine.clone();
            let feed_result =
                tokio::task::spawn_blocking(move || rss::add_feed(&engine.db, &account, &feed_url))
                    .await??;
            Ok(feed_result)
        }

        // Remove a single feed (subscription) and its items from an RSS account.
        "feed.remove" => {
            let thread_id = required_string(parameters, "thread_id")?;
            let feed_result = rss::remove_feed(&engine.db.lock().unwrap(), &thread_id)?;
            Ok(feed_result)
        }

        // Move a feed subscription between RSS accounts without losing cached
        // items or per-item read/starred state.
        "feed.move" => {
            let thread_id = required_string(parameters, "thread_id")?;
            let target_account = required_string(parameters, "target_account")?;
            let feed_result =
                rss::move_feed(&engine.db.lock().unwrap(), &thread_id, &target_account)?;
            Ok(feed_result)
        }

        // Serialize one RSS account's feeds to an OPML 2.0 document.
        "rss.exportOpml" => {
            let account = required_string(parameters, "account")?;
            let opml = rss::export_opml(&engine.db.lock().unwrap(), &account)?;
            Ok(json!({ "opml": opml }))
        }

        // Import feeds from an OPML document into one RSS account. Returns the
        // number of feeds added; the caller reloads accounts and syncs.
        "rss.importOpml" => {
            let opml = required_string(parameters, "opml")?;
            let account = required_string(parameters, "account")?;
            let engine = engine.clone();
            let imported =
                tokio::task::spawn_blocking(move || rss::import_opml(&engine.db, &opml, &account))
                    .await??;
            Ok(json!({ "imported": imported }))
        }

        // RSS thread read: paginated newest-first slice (or full thread when
        // `limit` is omitted), as final Message JSON.
        "rss.thread" => {
            let thread_id = required_string(parameters, "thread_id")?;
            let limit = parameters
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            let before_cursor = parameters
                .get("before_cursor")
                .and_then(Value::as_str)
                .and_then(parse_rss_cursor);
            let (messages, next_cursor) = rss::read_thread_page(
                &engine.db.lock().unwrap(),
                &thread_id,
                before_cursor,
                limit,
            )?;
            let mut out = json!({ "messages": messages });
            if let Some(cursor) = next_cursor {
                out.as_object_mut()
                    .unwrap()
                    .insert("next_cursor".into(), Value::String(cursor));
            }
            Ok(out)
        }

        "rss.markRead" => {
            let thread_id = required_string(parameters, "thread_id")?;
            // Defaults to read; pass seen:false to mark unread.
            let seen = parameters
                .get("seen")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let item_keys = parameters
                .get("item_keys")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if item_keys.is_empty() {
                rss::mark_thread_read(&engine.db.lock().unwrap(), &thread_id, seen)?;
            } else {
                rss::mark_items_read(&engine.db.lock().unwrap(), &thread_id, &item_keys, seen)?;
            }
            Ok(json!({ "ok": true }))
        }

        "rss.markAllRead" => {
            let account = required_string(parameters, "account")?;
            let updated = rss::mark_account_read(&engine.db.lock().unwrap(), &account)?;
            Ok(json!({
                "ok": true,
                "updated": updated,
                "folder_unreads": { (account): { "inbox": 0 } },
            }))
        }

        "rss.markStarred" => {
            let thread_id = required_string(parameters, "thread_id")?;
            let starred = parameters
                .get("starred")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let item_keys = parameters
                .get("item_keys")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if item_keys.is_empty() {
                rss::mark_thread_starred(&engine.db.lock().unwrap(), &thread_id, starred)?;
            } else {
                rss::mark_items_starred(
                    &engine.db.lock().unwrap(),
                    &thread_id,
                    &item_keys,
                    starred,
                )?;
            }
            Ok(json!({ "ok": true }))
        }

        other => Err(anyhow::anyhow!("unknown method: {other}")),
    }
}
