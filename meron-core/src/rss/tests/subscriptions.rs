//! Subscription changes and OPML exchange.
//! The cases import outlines, move feeds across accounts and remove subscriptions.
//! Database fixtures check that item identity and read state survive a move.

use super::*;

#[test]
fn import_opml_accepts_inoreader_export() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, display_name, config)
             VALUES('rss-acct', 'rss', 'rss', 'RSS', '{}')",
        [],
    )
    .unwrap();
    let db = Mutex::new(conn);
    let opml = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="1.0">
  <head>
    <title>Feeds exported from Reader</title>
  </head>
  <body>
    <outline text="Tech" title="Tech">
      <outline text="Feed One" title="Feed One" type="rss" xmlUrl="https://feeds.example.test/one.xml" htmlUrl="https://example.test/one"/>
      <outline text="Feed Two" title="Feed Two" type="rss" xmlUrl="https://feeds.example.test/two.xml" htmlUrl="https://example.test/two"/>
    </outline>
    <outline text="News" title="News">
      <outline text="Feed Three" title="Feed Three" type="rss" xmlUrl="https://feeds.example.test/three.xml" htmlUrl="https://example.test/three"/>
    </outline>
  </body>
</opml>"#;

    assert_eq!(import_opml(&db, opml, "rss-acct").unwrap(), 3);
    let count: i64 = db
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM subscriptions WHERE account = 'rss-acct'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 3);
}

#[test]
fn move_feed_reassigns_subscription_and_preserves_items() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, display_name, config)
             VALUES('rss-one', 'rss', 'rss', 'One', '{}'),
                   ('rss-two', 'rss', 'rss', 'Two', '{}')",
        [],
    )
    .unwrap();
    conn.execute(
            "INSERT INTO subscriptions(id, account, url, title, feed_title, etag, last_modified, enabled)
             VALUES('feed-1', 'rss-one', 'https://example.com/feed', 'Example', 'Example Feed', 'etag-1', 'mod-1', 1)",
            [],
        )
        .unwrap();
    store::upsert_rss_item(
        &conn,
        "rss-one",
        "feed-1",
        "item-1",
        "Post",
        false,
        None,
        &RssItemExtra {
            author: "Author".to_string(),
            link: "https://example.com/post".to_string(),
            summary: "Summary".to_string(),
            content: String::new(),
            images: vec![],
            videos: vec![],
            published_at: 10,
            updated_at: 0,
            fetched_at: 11,
        },
    )
    .unwrap();
    conn.execute(
            "UPDATE messages SET starred = 1 WHERE account = 'rss-one' AND folder = 'feed-1' AND msg_id = 'item-1'",
            [],
        )
        .unwrap();

    let result = move_feed(&conn, "rss-one#rss#feed-1", "rss-two").unwrap();
    assert_eq!(result["thread_id"], "rss-two#rss#feed-1");

    let sub_account: String = conn
        .query_row(
            "SELECT account FROM subscriptions WHERE id = 'feed-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sub_account, "rss-two");

    let (item_account, seen, starred): (String, i64, i64) = conn
            .query_row(
                "SELECT account, seen, starred FROM messages WHERE folder = 'feed-1' AND msg_id = 'item-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
    assert_eq!(item_account, "rss-two");
    assert_eq!(seen, 1);
    assert_eq!(starred, 1);
}

#[test]
fn move_feed_rejects_non_rss_target() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, display_name, config)
             VALUES('rss-one', 'rss', 'rss', 'One', '{}'),
                   ('mail-one', 'mail', 'custom', 'Mail', '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, enabled)
             VALUES('feed-1', 'rss-one', 'https://example.com/feed', 'Example', 1)",
        [],
    )
    .unwrap();

    let error = move_feed(&conn, "rss-one#rss#feed-1", "mail-one").unwrap_err();
    assert!(error.to_string().contains("is not RSS"));
}

#[test]
fn new_account_ids_are_unique_and_prefixed() {
    let a = new_account_id();
    let b = new_account_id();
    assert!(a.starts_with("rss-"));
    assert!(b.starts_with("rss-"));
    assert_ne!(a, b, "each account gets a distinct id (no name grouping)");
}

#[test]
fn remove_feed_deletes_subscription_and_items() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    insert_item(&conn, "rss-acct", "feed-1", "item-1", 100);
    insert_item(&conn, "rss-acct", "feed-1", "item-2", 200);

    remove_feed(&conn, "rss-acct#rss#feed-1").unwrap();

    let subs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM subscriptions WHERE id = 'feed-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let message_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE folder = 'feed-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(subs, 0, "subscription row must be deleted");
    assert_eq!(message_count, 0, "the feed's cached items must be deleted");
}

#[test]
fn remove_feed_rejects_invalid_thread_id() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    assert!(remove_feed(&conn, "garbage").is_err());
}

#[test]
fn export_opml_round_trips_through_import() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts(id, engine, provider, display_name, config)
             VALUES('rss-src', 'rss', 'rss', 'My Feeds', '{}')",
        [],
    )
    .unwrap();
    // Two feeds with an XML-special character in a title to exercise escaping.
    conn.execute(
            "INSERT INTO subscriptions(id, account, url, title, feed_title, site_url, enabled)
             VALUES('feed-1', 'rss-src', 'https://a.example/feed.xml', 'News & Co', 'News', 'https://a.example', 1),
                   ('feed-2', 'rss-src', 'https://b.example/feed.xml', 'Tech', 'Tech', '', 1)",
            [],
        )
        .unwrap();

    let opml = export_opml(&conn, "rss-src").unwrap();
    assert!(
        opml.contains("News &amp; Co"),
        "title must be XML-escaped: {opml}"
    );
    assert!(opml.contains("xmlUrl=\"https://a.example/feed.xml\""));

    // Import into a *separate* database — the `url` column is globally UNIQUE,
    // so re-importing the same feeds into the same DB is (correctly) a no-op.
    // The real round-trip is export-here, import-on-another-machine.
    let destination = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&destination).unwrap();
    destination
        .execute(
            "INSERT INTO accounts(id, engine, provider, display_name, config)
             VALUES('rss-dst', 'rss', 'rss', 'Dest', '{}')",
            [],
        )
        .unwrap();
    let db = Mutex::new(destination);
    let imported = import_opml(&db, &opml, "rss-dst").unwrap();
    assert_eq!(
        imported, 2,
        "both feeds must round-trip into the new account"
    );

    let urls: Vec<String> = {
        let conn = db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT url FROM subscriptions WHERE account = 'rss-dst' ORDER BY url")
            .unwrap();

        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert_eq!(
        urls,
        vec![
            "https://a.example/feed.xml".to_string(),
            "https://b.example/feed.xml".to_string(),
        ]
    );
}

#[test]
fn export_opml_rejects_unknown_account() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    assert!(export_opml(&conn, "nope").is_err());
}
