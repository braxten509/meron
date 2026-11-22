//! Shared store and HTTP fixtures for the RSS behavior tests.
//! The local server serves controlled snapshots for sync and notification cases.
//! Database helpers populate entries for mailbox, flag and subscription tests.

use super::content::{
    clean_feed_text, extract_image_urls, extract_video_urls, image_mime, video_mime,
};
use super::fetch::{discover_feed_url, parse_item};
use super::media_cache::sniff_image_ext;
use super::*;
use crate::store::RssMedia;

fn read_thread(conn: &Connection, thread_id: &str) -> Result<Vec<Value>> {
    let (items, _) = read_thread_page(conn, thread_id, None, None)?;
    Ok(items)
}

fn entry(id: &str, link: &str, title: &str, published: i64) -> feed_rs::model::Entry {
    let mut item = feed_rs::model::Entry::default();
    item.id = id.to_string();
    if !link.is_empty() {
        item.links = vec![feed_rs::model::Link {
            href: link.to_string(),
            rel: None,
            media_type: None,
            href_lang: None,
            title: None,
            length: None,
        }];
    }
    if !title.is_empty() {
        item.title = Some(feed_rs::model::Text {
            content_type: "text/plain".parse().unwrap(),
            src: None,
            content: title.to_string(),
        });
    }
    if published != 0 {
        item.published = chrono::DateTime::from_timestamp(published, 0);
    }
    item
}

// ---- test helpers for the additions below -------------------------------

fn rss_conn_with_feed(account: &str, sub: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, display_name, config)
             VALUES(?1, 'rss', 'rss', 'RSS', '{}')",
        params![account],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, feed_title, enabled)
             VALUES(?1, ?2, 'https://example.com/feed', 'Example Feed', 'Example Feed', 1)",
        params![sub, account],
    )
    .unwrap();
    conn
}

fn insert_item(conn: &Connection, account: &str, sub: &str, key: &str, published_at: i64) {
    store::upsert_rss_item(
        conn,
        account,
        sub,
        key,
        key,
        true,
        None,
        &RssItemExtra {
            author: String::new(),
            link: "https://example.com/post".to_string(),
            summary: "Caption".to_string(),
            content: String::new(),
            images: Vec::new(),
            videos: Vec::new(),
            published_at,
            updated_at: 0,
            fetched_at: published_at + 100,
        },
    )
    .unwrap();
}

// ---- thread list filters -------------------------------------------------

// ---- read_thread_page pagination ----------------------------------------

// ---- mark read / starred lifecycle --------------------------------------

fn seen_starred(conn: &Connection, sub: &str, key: &str) -> (i64, i64) {
    conn.query_row(
        "SELECT seen, starred FROM messages WHERE folder = ?1 AND msg_id = ?2",
        params![sub, key],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )
    .unwrap()
}

// ---- remove_feed --------------------------------------------------------

// ---- OPML export round-trips with import --------------------------------

// ---- small pure helpers -------------------------------------------------

// ---- end-to-end feed fetch over a real HTTP server ----------------------

/// Spawn a throwaway HTTP/1.1 server that replies to every request with the
/// same body. Returns the feed URL. The thread is detached (dies with the
/// test process). Exercises the real `ureq` fetch path without any network.
fn serve_feed(body: &'static str) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let _ = stream.read(&mut buffer); // drain the request line/headers
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}/feed.xml")
}

const SAMPLE_RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Sample Feed</title>
    <link>https://example.com</link>
    <item>
      <title>First post</title>
      <link>https://example.com/1</link>
      <guid>guid-1</guid>
      <description>Hello from the first post.</description>
    </item>
    <item>
      <title>Second post</title>
      <link>https://example.com/2</link>
      <guid>guid-2</guid>
      <description>And the second.</description>
    </item>
  </channel>
</rss>"#;

/// Spawn a throwaway HTTP/1.1 server that serves `bodies` in order, one per
/// request, repeating the last body once the list runs out. Lets a test fetch a
/// feed, then re-fetch it after a new entry appeared.
fn serve_feed_sequence(bodies: Vec<&'static str>) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut served = 0usize;
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 2048];
            let _ = stream.read(&mut buffer); // drain the request line/headers
            let body = bodies[served.min(bodies.len() - 1)];
            served += 1;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/rss+xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}/feed.xml")
}

/// A feed whose only entry is dated *later* than the arrival the other feed
/// publishes below, so it wins any "newest stored item" ordering.
const LATE_DATED_RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Late Dated Feed</title>
    <link>https://late.example.com</link>
    <item>
      <title>Published yesterday</title>
      <link>https://late.example.com/1</link>
      <guid>late-1</guid>
      <pubDate>Tue, 25 Aug 2026 12:00:00 GMT</pubDate>
      <description>Already seen, already read.</description>
    </item>
  </channel>
</rss>"#;

const SLOW_FEED_BEFORE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Slow Feed</title>
    <link>https://slow.example.com</link>
    <item>
      <title>An older post</title>
      <link>https://slow.example.com/1</link>
      <guid>slow-1</guid>
      <pubDate>Thu, 20 Aug 2026 12:00:00 GMT</pubDate>
      <description>Nothing new here.</description>
    </item>
  </channel>
</rss>"#;

/// The same feed after publishing an entry dated *before* the other feed's
/// entry — the arrival a notification must name.
const SLOW_FEED_AFTER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Slow Feed</title>
    <link>https://slow.example.com</link>
    <item>
      <title>The actual arrival</title>
      <link>https://slow.example.com/2</link>
      <guid>slow-2</guid>
      <pubDate>Mon, 24 Aug 2026 12:00:00 GMT</pubDate>
      <description>This is what just landed.</description>
    </item>
    <item>
      <title>An older post</title>
      <link>https://slow.example.com/1</link>
      <guid>slow-1</guid>
      <pubDate>Thu, 20 Aug 2026 12:00:00 GMT</pubDate>
      <description>Nothing new here.</description>
    </item>
  </channel>
</rss>"#;

fn new_item(feed: &str, title: &str, date: i64) -> NewItem {
    NewItem {
        subscription_id: format!("sub-{feed}"),
        item_key: format!("key-{feed}-{title}"),
        feed_title: feed.to_string(),
        title: title.to_string(),
        preview: format!("{title} preview"),
        date,
    }
}

mod arrivals;
mod mailbox;
mod parsing;
mod subscriptions;
