//! Shared backup fixtures and their portable account data.
//! Tests use a temporary database and a credential store to round-trip exports.
//! The child suites cover validation, restoration and organizer records.

use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    store::run_migrations(&conn).unwrap();
    conn
}

fn no_secrets(_: &str) -> Secrets {
    Secrets::default()
}

/// `BackupData` is deliberately not `Debug` (it can hold a password), so
/// `unwrap_err` is unavailable — pull the message out by hand.
fn parse_err(text: &str, passphrase: Option<&str>) -> String {
    match parse(text, passphrase) {
        Ok(_) => panic!("expected the backup to be rejected"),
        Err(err) => err.to_string(),
    }
}

fn secrets_with(password: &str) -> Secrets {
    Secrets {
        password: password.to_string(),
        access_token: None,
        refresh_token: None,
    }
}

/// A `store_secrets` sink that records what it was handed, standing in for the
/// keychain (desktop) or the keyed DB (mobile).
#[derive(Default)]
struct SecretSink(Mutex<HashMap<String, String>>);

impl SecretSink {
    fn saver(&self) -> impl Fn(&Connection, &str, &Secrets) -> Result<()> + '_ {
        move |_conn, account, secrets| {
            self.0
                .lock()
                .unwrap()
                .insert(account.to_string(), secrets.password.clone());
            Ok(())
        }
    }
}

fn insert_mail_account(conn: &Connection, id: &str, config: Value, prefs: Value) {
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, email, display_name, avatar_url, sender_name,
                              config, prefs, sort_order, created_at, updated_at)
         VALUES(?1, 'mail', 'custom', ?2, 'Work', '', 'Ada', ?3, ?4, 3, 100, 100)",
        params![id, id, config.to_string(), prefs.to_string()],
    )
    .unwrap();
}

fn insert_rss_account(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, display_name, config, prefs, sort_order, created_at, updated_at)
         VALUES(?1, 'rss', 'rss', 'Feeds', '{}', '{}', 7, 100, 100)",
        params![id],
    )
    .unwrap();
}

fn insert_subscription(conn: &Connection, account: &str, id: &str, url: &str, title: &str) {
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, site_url, feed_title, enabled, created_at, updated_at)
         VALUES(?1, ?2, ?3, ?4, 'https://site.example', ?4, 1, 100, 100)",
        params![id, account, url, title],
    )
    .unwrap();
}

fn sample_config() -> Value {
    json!({
        "host": "imap.example.com",
        "port": 993,
        "user": "ada@example.com",
        "tls": true,
        "smtp_host": "smtp.example.com",
        "smtp_port": 465,
        "auth_type": "password",
        "oauth_client_secret": "app-client-secret",
        "proxy": { "mode": "socks5", "host": "127.0.0.1", "port": 1080, "username": "u", "password": "proxy-pass" },
    })
}

// ---- Round trip -------------------------------------------------------------

// ---- Secrets and redaction --------------------------------------------------

// ---- Passphrase failures ----------------------------------------------------

// ---- Malformed input --------------------------------------------------------

// ---- Apply semantics --------------------------------------------------------

// ---- Pre-release `platform` map ---------------------------------------------

mod envelope;
mod export;
mod organizer;
mod restore;
