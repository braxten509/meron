//! Shared store test fixtures and feature-specific suites.
//!
//! Child modules use these helpers to exercise the same underlying store.

use super::*;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::imap::Folder;
use crate::parse::Message;

const UNREAD: RecentFilter = RecentFilter {
    unread_only: true,
    attachments_only: false,
};
const ATTACHMENTS: RecentFilter = RecentFilter {
    unread_only: false,
    attachments_only: true,
};

fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    db::run_migrations(&conn).unwrap();
    conn
}

/// A send date for test mail that a real Inbox still lists. The Inbox leaves
/// out mail past its age limit (see [`inbox_listed_sql`]), so a fixed date
/// ages out of every Inbox test. One value per test run: rows stamped with it
/// tie on date.
fn fresh_date() -> i64 {
    static DATE: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *DATE.get_or_init(|| now_unix() - 3600)
}

fn insert_message(
    conn: &Connection,
    uid: u32,
    subject: &str,
    from_name: &str,
    from_addr: &str,
    body: Option<&str>,
) {
    insert_message_in(conn, "INBOX", uid, subject, from_name, from_addr, body);
}

/// [`insert_message`] into `folder`.
fn insert_message_in(
    conn: &Connection,
    folder: &str,
    uid: u32,
    subject: &str,
    from_name: &str,
    from_addr: &str,
    body: Option<&str>,
) {
    conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, body)
             VALUES('acct', ?7, ?1, ?2, ?3, ?4, ?5, ?8, 0, ?6)",
            params![uid.to_string(), uid, subject, from_name, from_addr, body, folder, fresh_date()],
        )
        .unwrap();
}

/// The inheritance lookups match rows on `lower(json_extract(...))` and
/// `lower(thread_key)`, which only hit an index when the query spells the
/// expression exactly as `MESSAGES_THREAD_KEY_INDEXES_DDL` does. Without them
/// every upserted reply scans the account's whole message cache, so guard the
/// access path, not just the result.

/// The account address and the mailbox login are separate fields, so both are
/// identities the user owns — a custom account whose server authenticates by
/// user name has them differ, and picking its own address used to fall through
/// to the login.

/// An address the account doesn't own must fail the send rather than quietly
/// going out as someone else — a silent substitution is invisible to the sender.

/// Accounts stored without an explicit address fall back to the login, matching
/// what `account.list` shows as the account's address.

/// A minimal mail account so proxy persistence can be exercised without the
/// rest of the connection metadata mattering.
fn proxy_test_account(conn: &Connection, id: &str, proxy: crate::proxy::ProxyChoice) {
    let creds = proxy_test_creds(proxy);
    let meta = AccountMeta {
        engine: "mail".to_string(),
        provider: "custom".to_string(),
        email: "user@example.com".to_string(),
        display_name: String::new(),
        avatar_url: String::new(),
        sender_name: String::new(),
    };
    upsert_account(conn, id, &meta, &creds).unwrap();
}

fn proxy_test_creds(proxy: crate::proxy::ProxyChoice) -> crate::imap::Creds {
    crate::imap::Creds {
        host: "imap.example.com".to_string(),
        port: 993,
        user: "user@example.com".to_string(),
        password: String::new(),
        tls: true,
        starttls: false,
        smtp_host: "smtp.example.com".to_string(),
        smtp_port: 587,
        smtp_tls: false,
        smtp_starttls: true,
        auth_type: "password".to_string(),
        access_token: None,
        refresh_token: None,
        token_expires_at: 0,
        oauth_client_id: String::new(),
        oauth_client_secret: String::new(),
        oauth_token_url: String::new(),
        oauth_scope: String::new(),
        proxy,
        cert_pin: None,
        smtp_cert_pin: None,
        rejected_grant: None,
    }
}

fn stored_creds(conn: &Connection, id: &str) -> crate::imap::Creds {
    load_accounts(conn)
        .unwrap()
        .into_iter()
        .find(|(account, _)| account == id)
        .map(|(_, creds)| creds)
        .expect("account not found")
}

fn stored_proxy(conn: &Connection, id: &str) -> crate::proxy::ProxyChoice {
    stored_creds(conn, id).proxy
}

/// A sign-in the provider refused has to stay refused across a restart, and
/// saving the account again — which is what reconnecting does — has to lift it.

/// The pin an account carries for a server whose certificate webpki refuses
/// (Proton Bridge and friends) has to survive a restart, or every launch would
/// re-prompt.

/// Reconnecting resends the setup form, which has no field for the account's
/// proxy or the certificates it accepted. Saving credentials again must not
/// reset settings the form never carried.

/// A setting the caller *did* send wins: that is how the user changes it, and
/// how accepting a new certificate replaces the pin.

/// Editing an account's servers from Settings resends the form without a
/// password — the UI never holds one. An omitted password must keep the stored
/// credential rather than blank it, or saving a port change would lock the
/// account out.

/// A password the caller *did* send replaces the stored one — that is how the
/// user changes it after a server-side reset.

/// Editing an existing account (server settings, reconnect) re-runs the upsert
/// with no avatar; the custom one the user picked must survive it.

/// Accepting a certificate for an account that already exists must not disturb
/// the rest of its connection settings — the user is not re-entering them.

/// A cached body carrying `attachments` (and optionally HTML that shows some of
/// them inline), saved the way the body fetch saves it.
fn save_body_with_attachments(conn: &Connection, uid: u32, keys: &[&str], html: Option<&str>) {
    let message = Message {
        subject: "Hi".into(),
        body: "body".into(),
        body_html: html.map(str::to_string),
        attachments: keys
            .iter()
            .map(|key| crate::parse::Attachment {
                filename: key.rsplit('/').next().unwrap().into(),
                mime: "application/octet-stream".into(),
                size: 1,
                key: Some(key.to_string()),
                oversized: false,
            })
            .collect(),
        ..Default::default()
    };
    save_cached_message(conn, "acct", "INBOX", uid, &message).unwrap();
}

fn parse_cursor(cursor: Option<&str>) -> Option<(i64, u32)> {
    let (date, uid) = cursor?.strip_prefix("date:")?.split_once(':')?;
    Some((date.parse().ok()?, uid.parse().ok()?))
}

/// `open_notifying` leans on this constant to spot a store it must upgrade.

/// Encrypting a legacy store exports all of it, so it has to be announced even
/// when the schema is already current, and only by the open that does it.

/// A mailbox for the sender-grouping tests: Inbox, Sent, Drafts and Archive,
/// with the account's own address set so outgoing mail is recognised.
fn sender_test_conn() -> Connection {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO accounts(id, engine, email) VALUES('acct', 'mail', 'me@example.com')",
        [],
    )
    .unwrap();
    let folder = |name: &str, special_use: Option<&str>| Folder {
        name: name.to_string(),
        special_use: special_use.map(str::to_string),
        ..Default::default()
    };
    upsert_folders(
        &conn,
        "acct",
        &[
            folder("INBOX", None),
            folder("Sent", Some("sent")),
            folder("Drafts", Some("drafts")),
            folder("Archive", Some("archive")),
        ],
    )
    .unwrap();
    conn
}

#[allow(clippy::too_many_arguments)]
fn insert_sender_row(
    conn: &Connection,
    folder: &str,
    uid: u32,
    from: &str,
    to: &str,
    date: i64,
    seen: bool,
    message_id: &str,
) {
    let json = json!({
        "to": [{ "name": "", "addr": to }],
        "message_id": message_id,
    })
    .to_string();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen, thread_key, json)
         VALUES('acct', ?1, ?2, ?3, 'Hello', '', ?4, ?5, ?6, ?7, ?8)",
        params![folder, uid.to_string(), uid, from, date, seen as i64, format!("t{uid}"), json],
    )
    .unwrap();
}

mod accounts;
mod attachments;
mod contacts;
mod drafts;
mod folders;
mod invitations;
mod message_cache;
mod message_counts;
mod migrations;
mod notifications;
mod preferences;
mod search;
mod search_pages;
mod senders;
mod starred;
mod starred_updates;
mod sync_snapshots;
mod thread_groups;
mod thread_identity;
mod thread_messages;
