//! Feed refresh and the notifications produced by newly stored entries.
//! Local HTTP fixtures change the feed between successive refreshes.
//! The cases compare desktop notification details with the mobile worker response.

use super::*;

#[test]
fn add_fetches_parses_and_syncs_over_http() {
    // Keep prune_feed_media (called by sync_account) confined to a temp dir
    // instead of the real media cache.
    let media = std::env::temp_dir().join(format!("meron-rss-test-{}", std::process::id()));
    unsafe { std::env::set_var("MERON_MEDIA_DIR", &media) };

    let url = serve_feed(SAMPLE_RSS);
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    let db = Mutex::new(conn);

    // add() fetches the feed, creates the account, and stores both items.
    let account = add(&db, &url, "Sample Feed").unwrap();
    let account_id = account["id"].as_str().unwrap().to_string();
    assert!(account_id.starts_with("rss-"));

    let (subs, message_count): (i64, i64) = {
        let conn = db.lock().unwrap();
        let subs = conn
            .query_row(
                "SELECT COUNT(*) FROM subscriptions WHERE account = ?1",
                params![account_id],
                |r| r.get(0),
            )
            .unwrap();
        let message_count = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account = ?1",
                params![account_id],
                |r| r.get(0),
            )
            .unwrap();
        (subs, message_count)
    };
    assert_eq!(subs, 1, "one subscription created");
    assert_eq!(message_count, 2, "both feed items stored");

    // The parsed items surface as a thread with the feed's titles.
    let threads = {
        let conn = db.lock().unwrap();
        recent(&conn, &account_id, "", "", 10).unwrap()
    };
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0]["from_name"], "Sample Feed");

    // A re-sync of the unchanged feed finds no new items (guids dedupe).
    let new_items = sync_account(&db, &account_id).unwrap();
    assert!(
        new_items.is_empty(),
        "unchanged feed yields no new items on re-sync"
    );
}

/// A sync reports the entries that actually arrived, not the account's
/// newest-dated stored row. The two differ whenever a feed publishes with a
/// timestamp older than something another feed already stored, which used to
/// make the notification name the wrong feed and open the wrong thread.
#[test]
fn sync_reports_arrivals_not_the_newest_stored_item() {
    let media = std::env::temp_dir().join(format!("meron-rss-arrivals-{}", std::process::id()));
    unsafe { std::env::set_var("MERON_MEDIA_DIR", &media) };

    let late_url = serve_feed(LATE_DATED_RSS);
    let slow_url = serve_feed_sequence(vec![SLOW_FEED_BEFORE, SLOW_FEED_AFTER]);
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    let db = Mutex::new(conn);

    let account = add(&db, &late_url, "Feeds").unwrap();
    let account_id = account["id"].as_str().unwrap().to_string();
    add_feed(&db, &account_id, &slow_url).unwrap();

    // Both feeds are re-fetched; only the slow feed gained an entry, and its
    // date sorts below the late feed's already-stored (and read) entry.
    let new_items = sync_account(&db, &account_id).unwrap();
    assert_eq!(new_items.len(), 1, "one genuine arrival");
    assert_eq!(new_items[0].title, "The actual arrival");
    assert_eq!(new_items[0].feed_title, "Slow Feed");
    assert_eq!(new_items[0].preview, "This is what just landed.");
    assert_eq!(
        new_items[0].subscription_id,
        rss_subscription_id(&normalize_feed_url(&slow_url).unwrap()),
        "tapping the notification opens the feed the entry landed in"
    );

    // The trap: the account's newest-dated row belongs to the *other* feed, so
    // anything that picks the notification's subject by date lands on it.
    let newest_stored: String = {
        let conn = db.lock().unwrap();
        conn.query_row(
            "SELECT subject FROM messages WHERE account = ?1 ORDER BY date DESC, id DESC LIMIT 1",
            params![account_id],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(newest_stored, "Published yesterday");

    let stored_key: String = {
        let conn = db.lock().unwrap();
        conn.query_row(
            "SELECT msg_id FROM messages WHERE account = ?1 AND subject = ?2",
            params![account_id, "The actual arrival"],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        new_items[0].item_key, stored_key,
        "the arrival carries the key its stored row is identified by"
    );

    let detail = new_items_detail(&account_id, "Feeds", false, &new_items).unwrap();
    assert_eq!(detail["from"], "Slow Feed");
    assert_eq!(detail["subject"], "The actual arrival");
    assert_eq!(detail["count"], 1);
    assert_eq!(detail["threadKey"], new_items[0].subscription_id);
}

/// The detail describes the newest arrival at the top level (what the desktop
/// bridge reads) and every arrival in `messages` (what per-message clients read).
#[test]
fn new_items_detail_describes_every_arrival() {
    let items = vec![
        new_item("Feed A", "Newest", 300),
        new_item("Feed B", "Middle", 200),
        new_item("Feed A", "Oldest", 100),
    ];
    let detail = new_items_detail("rss-1", "My Feeds", false, &items).unwrap();

    assert_eq!(detail["account"], "rss-1");
    assert_eq!(detail["accountName"], "My Feeds");
    assert_eq!(detail["folder"], "inbox");
    assert_eq!(detail["count"], 3);
    assert_eq!(detail["muted"], false);
    assert_eq!(detail["from"], "Feed A");
    assert_eq!(detail["subject"], "Newest");
    assert_eq!(detail["preview"], "Newest preview");
    assert_eq!(detail["threadKey"], "sub-Feed A");

    let messages = detail["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1]["from"], "Feed B");
    assert_eq!(messages[1]["subject"], "Middle");
    assert_eq!(messages[1]["preview"], "Middle preview");
    assert_eq!(messages[1]["threadKey"], "sub-Feed B");
    assert_eq!(messages[1]["date"], 200);
    assert_eq!(messages[1]["itemKey"], "key-Feed B-Middle");
}

/// Two entries of one feed that happen to share a title are still told apart.
/// A feed is a single thread, so the thread key cannot separate them, and a
/// client keying a notification on thread + subject would have the second
/// arrival silently replace the first.
#[test]
fn new_items_detail_identifies_same_titled_arrivals_separately() {
    let mut first = new_item("Feed A", "Daily digest", 200);
    first.item_key = "key-monday".to_string();
    let mut second = new_item("Feed A", "Daily digest", 100);
    second.item_key = "key-tuesday".to_string();
    let detail = new_items_detail("rss-1", "My Feeds", false, &[first, second]).unwrap();

    let messages = detail["messages"].as_array().unwrap();
    assert_eq!(messages[0]["subject"], messages[1]["subject"]);
    assert_eq!(messages[0]["threadKey"], messages[1]["threadKey"]);
    assert_eq!(messages[0]["itemKey"], "key-monday");
    assert_eq!(messages[1]["itemKey"], "key-tuesday");
}

/// A batch bigger than the per-message cap still reports its true size, and the
/// muted flag rides along so the bridge can suppress the OS notification.
#[test]
fn new_items_detail_caps_listed_messages_and_carries_muted() {
    let items: Vec<NewItem> = (0..crate::mail_model::NEW_MESSAGES_DETAIL_MAX + 3)
        .map(|i| new_item("Feed", &format!("Item {i}"), 1000 - i as i64))
        .collect();
    let detail = new_items_detail("rss-1", "My Feeds", true, &items).unwrap();

    assert_eq!(detail["count"], items.len());
    assert_eq!(
        detail["messages"].as_array().unwrap().len(),
        crate::mail_model::NEW_MESSAGES_DETAIL_MAX
    );
    assert_eq!(detail["muted"], true);
}

/// No arrivals means no notification at all — the caller falls back to a silent
/// `mail.synced` refresh.
#[test]
fn new_items_detail_is_none_without_arrivals() {
    assert!(new_items_detail("rss-1", "My Feeds", false, &[]).is_none());
}

/// Android uses the response from rss.sync to notify about new feed items.
///
/// The background worker has no live event listener. Feed arrivals must appear in
/// the same new_messages field that the worker reads for mail.
#[test]
fn mobile_rss_sync_response_carries_new_arrivals() {
    let media = std::env::temp_dir().join(format!("meron-rss-mobile-{}", std::process::id()));
    unsafe { std::env::set_var("MERON_MEDIA_DIR", &media) };

    let url = serve_feed_sequence(vec![SLOW_FEED_BEFORE, SLOW_FEED_AFTER]);
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    let db = Mutex::new(conn);
    let account = add(&db, &url, "My Feeds").unwrap();
    let account_id = account["id"].as_str().unwrap().to_string();

    let conn = db.into_inner().unwrap();
    let response = crate::protocol::sync_mobile_rss_with_conn(conn, account_id.clone()).unwrap();

    assert_eq!(response["synced"], 1);
    let detail = &response["new_messages"];
    assert_eq!(detail["account"], account_id.as_str());
    // An RSS account has no email address, so the label is its display name.
    assert_eq!(detail["accountName"], "My Feeds");
    assert_eq!(detail["from"], "Slow Feed");
    assert_eq!(detail["subject"], "The actual arrival");
    assert_eq!(detail["count"], 1);
    assert_eq!(detail["muted"], false);
    assert_eq!(detail["messages"].as_array().unwrap().len(), 1);
}

/// A muted RSS account still syncs — the feed list refreshes — it just carries
/// the flag the clients read to skip the OS notification.
#[test]
fn mobile_rss_sync_marks_a_muted_account() {
    let media = std::env::temp_dir().join(format!("meron-rss-muted-{}", std::process::id()));
    unsafe { std::env::set_var("MERON_MEDIA_DIR", &media) };

    let url = serve_feed_sequence(vec![SLOW_FEED_BEFORE, SLOW_FEED_AFTER]);
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    let db = Mutex::new(conn);
    let account = add(&db, &url, "My Feeds").unwrap();
    let account_id = account["id"].as_str().unwrap().to_string();

    let conn = db.into_inner().unwrap();
    crate::store::set_account_pref(&conn, &account_id, "muted", true).unwrap();
    let response = crate::protocol::sync_mobile_rss_with_conn(conn, account_id).unwrap();

    assert_eq!(response["synced"], 1);
    assert_eq!(response["new_messages"]["muted"], true);
}

/// Nothing new means no `new_messages` at all, so a background refresh over
/// unchanged feeds stays silent.
#[test]
fn mobile_rss_sync_omits_new_messages_without_arrivals() {
    let media = std::env::temp_dir().join(format!("meron-rss-quiet-{}", std::process::id()));
    unsafe { std::env::set_var("MERON_MEDIA_DIR", &media) };

    let url = serve_feed(SLOW_FEED_BEFORE);
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    let db = Mutex::new(conn);
    let account = add(&db, &url, "My Feeds").unwrap();
    let account_id = account["id"].as_str().unwrap().to_string();

    let conn = db.into_inner().unwrap();
    let response = crate::protocol::sync_mobile_rss_with_conn(conn, account_id).unwrap();

    assert_eq!(response["synced"], 0);
    assert!(
        response.get("new_messages").is_none(),
        "an unchanged feed raises no notification"
    );
}
