//! Updates read and starred state for RSS entries in the shared message store.
//! Thread commands resolve a subscription from its public thread ID.
//! Item and account commands apply the change at the requested scope.

use super::parse_thread_id;
use crate::store;
use anyhow::Result;
use anyhow::anyhow;
use rusqlite::Connection;
use rusqlite::params;

pub fn mark_thread_read(conn: &Connection, thread_id: &str, seen: bool) -> Result<()> {
    let (account, sub_id) =
        parse_thread_id(thread_id).ok_or_else(|| anyhow!("invalid RSS thread id: {thread_id}"))?;
    if !seen {
        // Marking a feed unread flags its newest item only — see
        // store::update_rss_newest_item_seen.
        return store::update_rss_newest_item_seen(conn, &account, &sub_id, seen);
    }
    store::update_rss_thread_seen(conn, &account, &sub_id, seen)
}

pub fn mark_items_read(
    conn: &Connection,
    thread_id: &str,
    item_keys: &[String],
    seen: bool,
) -> Result<()> {
    let (account, sub_id) =
        parse_thread_id(thread_id).ok_or_else(|| anyhow!("invalid RSS thread id: {thread_id}"))?;
    for item_key in item_keys {
        store::update_rss_item_seen(conn, &account, &sub_id, item_key, seen)?;
    }
    Ok(())
}

pub fn mark_account_read(conn: &Connection, account: &str) -> Result<usize> {
    if !account.starts_with("rss-") {
        return Err(anyhow!("invalid RSS account id: {account}"));
    }
    Ok(conn.execute(
        "UPDATE messages SET seen = 1 WHERE account = ?1 AND seen = 0",
        params![account],
    )?)
}

pub fn mark_thread_starred(conn: &Connection, thread_id: &str, starred: bool) -> Result<()> {
    let (account, sub_id) =
        parse_thread_id(thread_id).ok_or_else(|| anyhow!("invalid RSS thread id: {thread_id}"))?;
    store::update_rss_thread_starred(conn, &account, &sub_id, starred)
}

pub fn mark_items_starred(
    conn: &Connection,
    thread_id: &str,
    item_keys: &[String],
    starred: bool,
) -> Result<()> {
    let (account, sub_id) =
        parse_thread_id(thread_id).ok_or_else(|| anyhow!("invalid RSS thread id: {thread_id}"))?;
    for item_key in item_keys {
        store::update_rss_item_starred(conn, &account, &sub_id, item_key, starred)?;
    }
    Ok(())
}
