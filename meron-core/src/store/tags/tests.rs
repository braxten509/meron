//! Shared database fixtures for message tags and mailbox rules.
//! The child suites simulate local mutations, keyword updates and queued operations.
//! Fixtures provide the accounts, folders and identities needed to follow a message.

//! Regression tests for tags.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;
use crate::imap::MessageHeader;
use crate::store::{
    Rule, RuleCopy, apply_rules, count_existing_rule_matches, finish_rule_actions,
    pending_rule_actions, queue_existing_rule, replace_rule_tag, retry_rule_actions, rules,
    set_rules,
};

fn db() -> Connection {
    let path = std::env::temp_dir().join(format!(
        "meron-tags-{}-{:?}.db",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    crate::store::open_at(&path).unwrap()
}

fn message(uid: u32, id: &str, server_tags: Option<&[&str]>) -> MessageHeader {
    MessageHeader {
        uid,
        subject: "s".into(),
        thread_key: "t".into(),
        message_id: id.into(),
        server_tags: server_tags.map(|tags| tags.iter().map(|tag| tag.to_string()).collect()),
        ..Default::default()
    }
}

fn tags(conn: &Connection, id: &str) -> Vec<String> {
    tags_by_key(conn, "a", &[id.to_string()])
        .unwrap()
        .remove(id)
        .unwrap_or_default()
}

fn state(conn: &Connection, tag: &str) -> i64 {
    conn.query_row(
        "SELECT state FROM message_tags WHERE tag = ?1",
        [tag],
        |row| row.get(0),
    )
    .unwrap()
}

fn age_tags(conn: &Connection) {
    conn.execute("UPDATE message_tags SET updated_at = 0", [])
        .unwrap();
}

fn upsert(conn: &Connection, folder: &str, messages: &[MessageHeader]) {
    crate::store::upsert_messages(conn, "a", folder, messages).unwrap();
}

fn account(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, email, display_name, avatar_url, config, created_at, updated_at)
         VALUES(?1, 'imap', 'imap', ?1, '', '', '{}', 0, 0)",
        [id],
    )
    .unwrap();
}

mod definitions;
mod queued_actions;
mod rule_runs;
mod synchronization;
