//! RSS entries presented as conversations in the shared mailbox.
//! Queries exercise search filters, paging and attachment fields on feed messages.
//! Flag updates must respect the requested subscription and account.

use super::*;

#[test]
fn read_thread_exposes_inline_images_as_url_attachments() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, enabled)
             VALUES('feed-1', 'rss-acct', 'https://example.com/feed', 'Example Feed', 1)",
        [],
    )
    .unwrap();
    store::upsert_rss_item(
        &conn,
        "rss-acct",
        "feed-1",
        "item-1",
        "Post",
        true,
        None,
        &RssItemExtra {
            author: String::new(),
            link: "https://example.com/post".to_string(),
            summary: "Caption".to_string(),
            content: String::new(),
            images: vec![RssMedia {
                url: "https://cdn.example.com/p/1.jpg".to_string(),
                key: None,
            }],
            videos: Vec::new(),
            published_at: 1_700_000_000,
            updated_at: 0,
            fetched_at: 1_700_000_100,
        },
    )
    .unwrap();

    let items = read_thread(&conn, "rss-acct#rss#feed-1").unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["has_attachments"], true);
    let attachments = items[0]["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0]["url"], "https://cdn.example.com/p/1.jpg");
    assert_eq!(attachments[0]["filename"], "1.jpg");
    assert!(attachments[0]["key"].is_null());
}

#[test]
fn read_thread_exposes_inline_videos_as_remote_attachments() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, enabled)
             VALUES('feed-1', 'rss-acct', 'https://example.com/feed', 'Example Feed', 1)",
        [],
    )
    .unwrap();
    store::upsert_rss_item(
        &conn,
        "rss-acct",
        "feed-1",
        "item-1",
        "Post",
        true,
        None,
        &RssItemExtra {
            author: String::new(),
            link: "https://example.com/post".to_string(),
            summary: "Caption".to_string(),
            content: String::new(),
            images: Vec::new(),
            videos: vec![RssMedia {
                url: "https://media.example.test/link/abc/media/1".to_string(),
                key: None,
            }],
            published_at: 1_700_000_000,
            updated_at: 0,
            fetched_at: 1_700_000_100,
        },
    )
    .unwrap();

    let items = read_thread(&conn, "rss-acct#rss#feed-1").unwrap();
    assert_eq!(items.len(), 1);
    let attachments = items[0]["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(
        attachments[0]["url"],
        "https://media.example.test/link/abc/media/1"
    );
    assert_eq!(attachments[0]["mime"], "video/mp4");
    assert!(attachments[0]["key"].is_null());
}

#[test]
fn starred_items_returns_only_starred_rows_across_subscriptions() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    for (sub, account, url, title) in [
        ("feed-1", "rss-acct", "https://example.com/feed", "Feed One"),
        (
            "feed-2",
            "rss-other",
            "https://other.example/feed",
            "Feed Two",
        ),
    ] {
        conn.execute(
            "INSERT INTO subscriptions(id, account, url, title, enabled)
                 VALUES(?1, ?2, ?3, ?4, 1)",
            params![sub, account, url, title],
        )
        .unwrap();
    }
    let extra = |published_at: i64| RssItemExtra {
        author: String::new(),
        link: "https://example.com/post".to_string(),
        summary: "Caption".to_string(),
        content: String::new(),
        images: Vec::new(),
        videos: Vec::new(),
        published_at,
        updated_at: 0,
        fetched_at: published_at + 100,
    };
    store::upsert_rss_item(
        &conn,
        "rss-acct",
        "feed-1",
        "item-1",
        "Old starred",
        true,
        None,
        &extra(1_700_000_000),
    )
    .unwrap();
    store::upsert_rss_item(
        &conn,
        "rss-acct",
        "feed-1",
        "item-2",
        "Unstarred",
        true,
        None,
        &extra(1_700_000_500),
    )
    .unwrap();
    store::upsert_rss_item(
        &conn,
        "rss-other",
        "feed-2",
        "item-3",
        "New starred",
        true,
        None,
        &extra(1_700_001_000),
    )
    .unwrap();
    store::update_rss_item_starred(&conn, "rss-acct", "feed-1", "item-1", true).unwrap();
    store::update_rss_item_starred(&conn, "rss-other", "feed-2", "item-3", true).unwrap();

    let items = starred_items(&conn, 10).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|message| message["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["rss-other#rss#feed-2#item-3", "rss-acct#rss#feed-1#item-1",]
    );
    assert!(items.iter().all(|message| message["starred"] == true));
    assert_eq!(items[0]["from_name"], "Feed Two");
}

#[test]
fn recent_and_read_thread_build_bridge_messages() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, enabled)
             VALUES('feed-1', 'rss-acct', 'https://example.com/feed', 'Example Feed', 1)",
        [],
    )
    .unwrap();
    store::upsert_rss_item(
        &conn,
        "rss-acct",
        "feed-1",
        "item-1",
        "Hello RSS",
        true,
        None,
        &RssItemExtra {
            author: String::new(),
            link: "https://example.com/post".to_string(),
            summary: "Hello world.".to_string(),
            content: String::new(),
            images: Vec::new(),
            videos: Vec::new(),
            published_at: 1_700_000_000,
            updated_at: 0,
            fetched_at: 1_700_000_100,
        },
    )
    .unwrap();

    let threads = recent(&conn, "rss-acct", "", "", 10).unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0]["subject"], "Example Feed");
    assert_eq!(threads[0]["from_name"], "Example Feed");
    assert_eq!(threads[0]["preview"], "Hello RSS - Hello world.");
    assert_eq!(threads[0]["unread"], true);

    let thread_id = threads[0]["thread_id"].as_str().unwrap().to_string();
    assert!(parse_thread_id(&thread_id).is_some());

    let items = read_thread(&conn, &thread_id).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["subject"], "Hello RSS");
    assert_eq!(items[0]["body"], "Hello world.");
    assert_eq!(items[0]["link"], "https://example.com/post");
    assert_eq!(items[0]["unread"], true);
}

#[test]
fn recent_filters_feeds_by_unread_and_starred() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, feed_title, enabled)
             VALUES('feed-2', 'rss-acct', 'https://example.com/two', 'Second Feed', 'Second Feed', 1)",
        [],
    )
    .unwrap();
    insert_item(&conn, "rss-acct", "feed-1", "item-1", 100);
    insert_item(&conn, "rss-acct", "feed-2", "item-2", 200);
    // feed-2 is fully read but holds the only starred item.
    store::update_rss_item_seen(&conn, "rss-acct", "feed-2", "item-2", true).unwrap();
    store::update_rss_item_starred(&conn, "rss-acct", "feed-2", "item-2", true).unwrap();

    let titles = |filter: &str| {
        recent(&conn, "rss-acct", "", filter, 10)
            .unwrap()
            .iter()
            .map(|thread| thread["subject"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(titles("").len(), 2, "no filter keeps every feed");
    assert_eq!(titles("unread"), vec!["Example Feed".to_string()]);
    assert_eq!(titles("starred"), vec!["Second Feed".to_string()]);
    let starred = recent(&conn, "rss-acct", "", "starred", 10).unwrap();
    assert_eq!(starred[0]["starred"], Value::Null);
    assert_eq!(starred[0]["has_starred_items"], true);
}

#[test]
fn recent_reads_search_operators_against_feed_items() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, feed_title, enabled)
             VALUES('feed-2', 'rss-acct', 'https://example.com/two', 'Second Feed', 'Second Feed', 1)",
        [],
    )
    .unwrap();
    insert_item(&conn, "rss-acct", "feed-1", "launch-notes", 100);
    insert_item(&conn, "rss-acct", "feed-2", "weekly-digest", 1_800_000_000);
    store::update_rss_item_seen(&conn, "rss-acct", "feed-2", "weekly-digest", true).unwrap();
    store::update_rss_item_starred(&conn, "rss-acct", "feed-2", "weekly-digest", true).unwrap();

    let titles = |query: &str| {
        recent(&conn, "rss-acct", query, "", 10)
            .unwrap()
            .iter()
            .map(|thread| thread["subject"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(titles("launch"), ["Example Feed"]);
    assert_eq!(titles("from:second"), ["Second Feed"]);
    assert_eq!(titles("subject:digest"), ["Second Feed"]);
    assert_eq!(titles("is:unread"), ["Example Feed"]);
    assert_eq!(titles("is:starred"), ["Second Feed"]);
    assert_eq!(titles("after:2020/01/01"), ["Second Feed"]);
    assert_eq!(titles("before:2020/01/01 caption"), ["Example Feed"]);
    assert!(titles("to:me@example.com").is_empty());
    conn.execute(
        "UPDATE messages SET json = json_set(json, '$.images', json('[{\"url\":\"https://x.y/a.png\"}]'))
         WHERE folder = 'feed-2'",
        [],
    )
    .unwrap();
    assert_eq!(titles("has:attachment"), ["Second Feed"]);
}

#[test]
fn read_thread_page_walks_backwards_with_cursor() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    insert_item(&conn, "rss-acct", "feed-1", "item-1", 100);
    insert_item(&conn, "rss-acct", "feed-1", "item-2", 200);
    insert_item(&conn, "rss-acct", "feed-1", "item-3", 300);

    // First page (limit 2): the two newest items, ascending within the page,
    // plus a cursor pointing at the oldest item on the page.
    let (page1, cursor1) = read_thread_page(&conn, "rss-acct#rss#feed-1", None, Some(2)).unwrap();
    assert_eq!(
        page1
            .iter()
            .map(|message| message["subject"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["item-2", "item-3"],
    );
    let cursor1 = cursor1.expect("more items remain, cursor must be present");
    assert_eq!(cursor1, "ts:200:item-2");

    // Second page from that cursor: the remaining older item, no further cursor.
    let (page2, cursor2) = read_thread_page(
        &conn,
        "rss-acct#rss#feed-1",
        Some((200, "item-2".into())),
        Some(2),
    )
    .unwrap();
    assert_eq!(
        page2
            .iter()
            .map(|message| message["subject"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["item-1"],
    );
    assert!(cursor2.is_none(), "last page must not yield a cursor");
}

#[test]
fn read_thread_page_unpaginated_returns_all_ascending() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    insert_item(&conn, "rss-acct", "feed-1", "item-1", 100);
    insert_item(&conn, "rss-acct", "feed-1", "item-2", 300);
    insert_item(&conn, "rss-acct", "feed-1", "item-3", 200);

    let (items, cursor) = read_thread_page(&conn, "rss-acct#rss#feed-1", None, None).unwrap();
    assert!(cursor.is_none());
    assert_eq!(
        items
            .iter()
            .map(|message| message["subject"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["item-1", "item-3", "item-2"], // ts 100, 200, 300 ascending
    );
}

#[test]
fn mark_items_then_thread_read() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    insert_item(&conn, "rss-acct", "feed-1", "item-1", 100);
    insert_item(&conn, "rss-acct", "feed-1", "item-2", 200);

    // Marking one item read leaves the other untouched.
    mark_items_read(&conn, "rss-acct#rss#feed-1", &["item-1".into()], true).unwrap();
    assert_eq!(seen_starred(&conn, "feed-1", "item-1").0, 1);
    assert_eq!(seen_starred(&conn, "feed-1", "item-2").0, 0);

    // Marking the thread read flips the whole feed.
    mark_thread_read(&conn, "rss-acct#rss#feed-1", true).unwrap();
    assert_eq!(seen_starred(&conn, "feed-1", "item-1").0, 1);
    assert_eq!(seen_starred(&conn, "feed-1", "item-2").0, 1);
}

#[test]
fn marking_a_feed_unread_flags_only_its_newest_item() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    insert_item(&conn, "rss-acct", "feed-1", "item-1", 100);
    insert_item(&conn, "rss-acct", "feed-1", "item-2", 200);
    mark_thread_read(&conn, "rss-acct#rss#feed-1", true).unwrap();

    // "Mark unread" means "bring this feed back", not "I read none of these":
    // the card shows one unread item and opening it lands on that item.
    mark_thread_read(&conn, "rss-acct#rss#feed-1", false).unwrap();

    assert_eq!(seen_starred(&conn, "feed-1", "item-1").0, 1);
    assert_eq!(seen_starred(&conn, "feed-1", "item-2").0, 0);
}

#[test]
fn mark_account_read_clears_every_feed_without_touching_other_accounts() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    insert_item(&conn, "rss-acct", "feed-1", "item-1", 100);
    insert_item(&conn, "rss-acct", "feed-1", "item-2", 200);

    conn.execute(
        "INSERT INTO accounts(id, engine, provider, display_name, config)
             VALUES('rss-other', 'rss', 'rss', 'Other', '{}')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO subscriptions(id, account, url, title, feed_title, etag, last_modified, enabled)
             VALUES('feed-other', 'rss-other', 'https://example.com/other', 'Other', 'Other', '', '', 1)",
        [],
    )
    .unwrap();
    insert_item(&conn, "rss-other", "feed-other", "item-other", 300);

    assert_eq!(mark_account_read(&conn, "rss-acct").unwrap(), 2);
    assert_eq!(unread_count(&conn, "rss-acct").unwrap(), 0);
    assert_eq!(unread_count(&conn, "rss-other").unwrap(), 1);
}

#[test]
fn mark_account_read_rejects_non_rss_account_ids() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    insert_item(&conn, "rss-acct", "feed-1", "item-1", 100);

    assert!(mark_account_read(&conn, "mail-acct").is_err());
    assert_eq!(unread_count(&conn, "rss-acct").unwrap(), 1);
}

#[test]
fn mark_items_then_thread_starred() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    insert_item(&conn, "rss-acct", "feed-1", "item-1", 100);
    insert_item(&conn, "rss-acct", "feed-1", "item-2", 200);

    mark_items_starred(&conn, "rss-acct#rss#feed-1", &["item-2".into()], true).unwrap();
    assert_eq!(seen_starred(&conn, "feed-1", "item-1").1, 0);
    assert_eq!(seen_starred(&conn, "feed-1", "item-2").1, 1);

    mark_thread_starred(&conn, "rss-acct#rss#feed-1", true).unwrap();
    assert_eq!(seen_starred(&conn, "feed-1", "item-1").1, 1);
    assert_eq!(seen_starred(&conn, "feed-1", "item-2").1, 1);
}

#[test]
fn mark_rejects_invalid_thread_id() {
    let conn = rss_conn_with_feed("rss-acct", "feed-1");
    assert!(mark_thread_read(&conn, "not-an-rss-id", true).is_err());
    assert!(mark_items_read(&conn, "bad", &["x".into()], true).is_err());
}
