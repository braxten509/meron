//! Shared protocol test fixtures and feature-specific suites.
//!
//! The fixtures seed accounts, folders and media in isolated SQLite stores.
//! Token exchanges and provider requests use local HTTP responders.

use super::*;
use crate::imap::{Folder, MessageHeader, Recipient};
use crate::parse::Message;
use crate::store::{RssItemExtra, RssMedia};
use base64::engine::general_purpose::STANDARD;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEST_UNIQUE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Reconnecting an account re-runs `account.addPassword` with the setup form's
/// fields. The form has no proxy or certificate entry, so a per-account proxy —
/// and any certificate the user accepted — has to survive the save.

fn one_shot_oauth_token_server() -> String {
    one_shot_oauth_token_server_with_body(
        r#"{"access_token":"access-from-code","refresh_token":"refresh-from-code","expires_in":3600}"#
            .to_string(),
    )
}

fn one_shot_oauth_token_server_with_id_token(email: &str, name: &str) -> String {
    use base64::Engine;

    let claims = serde_json::json!({
        "preferred_username": email,
        "name": name,
    });
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string());
    let id_token = format!("header.{payload}.signature");
    one_shot_oauth_token_server_with_body(format!(
        r#"{{"access_token":"access-from-code","refresh_token":"refresh-from-code","expires_in":3600,"id_token":"{id_token}"}}"#
    ))
}

fn one_shot_oauth_token_server_with_body(body: String) -> String {
    one_shot_json_server("/token", &body)
}

fn one_shot_json_server(path: &str, body: &str) -> String {
    one_shot_server(path, "application/json", body)
}

fn one_shot_server(path: &str, content_type: &str, body: &str) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let body = body.to_string();
    let content_type = content_type.to_string();
    let path = path.to_string();
    let server_path = path.clone();
    std::thread::spawn(move || {
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).unwrap_or(0);
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            if request.is_empty() {
                continue;
            }
            let request_line = String::from_utf8_lossy(&request);
            let expected = format!(" {server_path} ");
            if !request_line
                .lines()
                .next()
                .unwrap_or("")
                .contains(&expected)
            {
                continue;
            }
            write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    content_type,
                    body.len(),
                    body
                )
                .unwrap();
            stream.flush().unwrap();
            return;
        }
    });
    format!("http://{addr}{path}")
}

// Notifications are withdrawn for mail no longer unread in the cache: read,
// or gone from the folder (moved/deleted here, or pruned by a sync after
// another device did it).

// A reply drafted on another client lands in the local store via the regular
// Drafts folder sync; opening the thread must surface it from the store alone
// (thread read spans folders), with no per-thread network check.

fn unique_data_directory(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("meron-mobile-{label}-{}", unique_test_suffix()))
}

fn unique_test_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let count = TEST_UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{pid}-{nanos}-{count}")
}

/// A send date for test mail that a real Inbox still lists: the Inbox leaves
/// out mail past its age limit (see `store::inbox_listed_sql`), so a fixed
/// date ages out. Tests add small offsets to it to order their mail.
fn fresh_date() -> i64 {
    static DATE: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *DATE.get_or_init(|| store::now_unix() - 3600)
}

fn seed_mobile_account(data_directory: &std::path::Path, email: &str) {
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    let creds = Creds {
        host: "imap.example.com".to_string(),
        port: 993,
        user: email.to_string(),
        password: String::new(),
        tls: true,
        starttls: false,
        smtp_host: "smtp.example.com".to_string(),
        smtp_port: 465,
        smtp_tls: true,
        smtp_starttls: false,
        auth_type: "password".to_string(),
        access_token: None,
        refresh_token: None,
        token_expires_at: 0,
        oauth_client_id: String::new(),
        oauth_client_secret: String::new(),
        oauth_token_url: String::new(),
        oauth_scope: String::new(),
        proxy: crate::proxy::ProxyChoice::Global,
        cert_pin: None,
        smtp_cert_pin: None,
        rejected_grant: None,
    };
    let meta = AccountMeta {
        engine: "mail".to_string(),
        provider: "custom".to_string(),
        email: email.to_string(),
        display_name: String::new(),
        avatar_url: String::new(),
        sender_name: String::new(),
    };
    store::upsert_account(&conn, email, &meta, &creds).unwrap();
}

fn seed_rss_account(data_directory: &std::path::Path, account: &str, display_name: &str) {
    let conn = store::open_at(data_directory.join("meron.db")).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, email, display_name, config)
             VALUES(?1, 'rss', 'rss', ?2, ?3, '{}')",
        rusqlite::params![account, format!("{account}.local"), display_name],
    )
    .unwrap();
}

fn seed_rss_feed(conn: &rusqlite::Connection, account: &str, sub_id: &str, title: &str) {
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, feed_title, enabled)
             VALUES(?1, ?2, ?3, ?4, ?4, 1)",
        rusqlite::params![
            sub_id,
            account,
            format!("https://example.com/{sub_id}"),
            title
        ],
    )
    .unwrap();
}

fn seed_rss_item(
    conn: &rusqlite::Connection,
    account: &str,
    sub_id: &str,
    item_key: &str,
    title: &str,
    ts: i64,
    unread: bool,
) {
    store::upsert_rss_item(
        conn,
        account,
        sub_id,
        item_key,
        title,
        unread,
        Some(&format!("<p>Body for {title}</p>")),
        &RssItemExtra {
            author: "Author".to_string(),
            link: format!("https://example.com/{item_key}"),
            summary: format!("Summary for {title}"),
            content: String::new(),
            images: Vec::<RssMedia>::new(),
            videos: Vec::<RssMedia>::new(),
            published_at: ts,
            updated_at: 0,
            fetched_at: ts,
        },
    )
    .unwrap();
}

fn add_password_account(directory_text: &str, email: &str) {
    let add = format!(
        r#"{{"id":1,"method":"account.addPassword","params":{{"email":"{email}","display_name":"E","sender_name":"E","imap_host":"imap.example.com","imap_port":993,"smtp_host":"smtp.example.com","smtp_port":587,"username":"{email}","password":"secret","tls":true}}}}"#
    );
    let added = invoke_mobile_protocol_json(&add, Some(directory_text));
    assert_eq!(added["result"]["account"]["id"], email);
}

// ---- Backup / restore -------------------------------------------------------

/// Mobile settings are rows in the core `settings` table, the same as desktop;
/// the platform store is only a cache in front of them.

/// Settings restored from a backup land in the same table the host hydrates
/// from, so a restore reaches mobile preferences with no extra channel.

/// Before a data dir is bound the stub table answers, so a host that polls the
/// core during startup gets an empty surface rather than "unknown method".
mod accounts;
mod backup;
mod contacts;
mod credentials;
mod feeds;
mod flags;
mod folders;
mod media;
mod mutations;
mod notifications;
mod oauth_accounts;
mod oauth_exchange;
mod oauth_refresh;
mod requests;
mod settings;
mod starred;
mod tasks;
mod thread_identity;
mod thread_listing;
mod thread_read;
