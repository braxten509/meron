//! Checks how the engine continues a search across cached and live results.
//! SQLite fixtures exercise page boundaries, cursor resumption and ordering.
//! The final cases cover per-folder failures and attachment filters.

use crate::engine::{
    SearchContinuation, batch_placement, cached_search_mail_page, continue_search_page,
    record_search_folder_result,
};
use crate::imap;
use rusqlite::{Connection, params};

#[test]
fn cached_search_page_advances_the_engine_cursor() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    for (uid, date) in [(1u32, 100i64), (2, 200)] {
        conn.execute(
            "INSERT INTO messages(
               account, folder, msg_id, uid, subject, from_name, from_addr, date
             ) VALUES('acct', 'INBOX', ?1, ?2, 'deploy notes', 'Ops',
                      'ops@example.com', ?3)",
            params![uid.to_string(), uid, date],
        )
        .unwrap();
    }
    let folders = ["INBOX".to_string()];
    let first = cached_search_mail_page(&conn, "acct", &folders, "deploy", 1, None).unwrap();
    assert_eq!(first.messages[0].uid, 2);
    let cursor = crate::thread_list::parse_search_cursor(
        first.next_cursor.as_deref().expect("full page has cursor"),
    )
    .unwrap();

    let second =
        cached_search_mail_page(&conn, "acct", &folders, "deploy", 1, Some(&cursor)).unwrap();
    assert_eq!(second.messages[0].uid, 1);
}

fn insert_deploy_hits(conn: &Connection, uids: std::ops::RangeInclusive<u32>) {
    for uid in uids {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date)
             VALUES('acct', 'INBOX', ?1, ?2, 'deploy notes', 'Ops', 'ops@example.com', ?3)",
            params![uid.to_string(), uid, uid as i64],
        )
        .unwrap();
    }
}

fn snapshot_cursor(token: &str, offset: u32) -> crate::thread_list::SearchCursor {
    crate::thread_list::SearchCursor {
        date: 0,
        uid: 0,
        folder: String::new(),
        scanned: 0,
        snapshot: Some(token.to_string()),
        offset,
    }
}

fn expect_page(continuation: SearchContinuation) -> crate::engine::SearchMailPage {
    match continuation {
        SearchContinuation::Page(page) => page,
        SearchContinuation::NeedsServer(_) => panic!("expected a local page"),
    }
}

#[test]
fn capped_search_snapshot_resumes_the_cache_where_its_scan_stopped() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    // Cached hits 2..=5, plus uid 1: an old server hit, also in the cache
    // since fetched hits are cached. The batch took cached hits 5 and 4, hit
    // its cap, and appended the server hit after them.
    insert_deploy_hits(&conn, 1..=5);
    let folders = ["INBOX".to_string()];
    let snapshot_rows =
        crate::store::search_messages_in_folders(&conn, "acct", &folders, "deploy", 10, None)
            .unwrap()
            .into_iter()
            .filter(|hit| [5, 4, 1].contains(&hit.uid))
            .collect::<Vec<_>>();
    let resume = crate::thread_list::format_search_cursor(&crate::thread_list::SearchCursor {
        date: 4,
        uid: 4,
        folder: "INBOX".to_string(),
        scanned: 0,
        snapshot: None,
        offset: 0,
    });
    let token =
        crate::store::create_search_snapshot(&conn, "acct", "deploy", &folders, &[]).unwrap();
    crate::store::finish_search_batch(&conn, &token, "acct", &[], &snapshot_rows, Some(&resume))
        .unwrap();

    let first = expect_page(
        continue_search_page(
            &conn,
            "acct",
            &folders,
            "deploy",
            10,
            &snapshot_cursor(&token, 0),
        )
        .unwrap(),
    );
    assert_eq!(
        first
            .messages
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>(),
        vec![5, 4, 1]
    );
    let tail =
        crate::thread_list::parse_search_cursor(first.next_cursor.as_deref().unwrap()).unwrap();

    // Paging past the snapshot resumes after cached hit 4, not after the
    // server hit, so 3 and 2 are not skipped; uid 1 is not listed twice.
    let rest =
        expect_page(continue_search_page(&conn, "acct", &folders, "deploy", 10, &tail).unwrap());
    assert_eq!(
        rest.messages
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>(),
        vec![3, 2]
    );
    assert!(rest.next_cursor.is_none());
}

#[test]
fn cache_tail_pages_keep_going_when_the_filter_thins_a_page() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    insert_deploy_hits(&conn, 1..=5);
    let folders = ["INBOX".to_string()];
    let listed =
        crate::store::search_messages_in_folders(&conn, "acct", &folders, "deploy", 10, None)
            .unwrap()
            .into_iter()
            .filter(|hit| [5, 3, 2].contains(&hit.uid))
            .collect::<Vec<_>>();
    let resume = crate::thread_list::format_search_cursor(&crate::thread_list::SearchCursor {
        date: 5,
        uid: 5,
        folder: "INBOX".to_string(),
        scanned: 0,
        snapshot: None,
        offset: 0,
    });
    let token =
        crate::store::create_search_snapshot(&conn, "acct", "deploy", &folders, &[]).unwrap();
    crate::store::finish_search_batch(&conn, &token, "acct", &[], &listed, Some(&resume)).unwrap();
    let first = expect_page(
        continue_search_page(
            &conn,
            "acct",
            &folders,
            "deploy",
            3,
            &snapshot_cursor(&token, 0),
        )
        .unwrap(),
    );
    let tail =
        crate::thread_list::parse_search_cursor(first.next_cursor.as_deref().unwrap()).unwrap();

    // The cache page after 5 is [4, 3, 2]; 3 and 2 are already listed, but the
    // page was full, so paging continues to 1.
    let thinned =
        expect_page(continue_search_page(&conn, "acct", &folders, "deploy", 3, &tail).unwrap());
    assert_eq!(
        thinned
            .messages
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>(),
        vec![4]
    );
    let next =
        crate::thread_list::parse_search_cursor(thinned.next_cursor.as_deref().unwrap()).unwrap();
    let last =
        expect_page(continue_search_page(&conn, "acct", &folders, "deploy", 3, &next).unwrap());
    assert_eq!(
        last.messages
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert!(last.next_cursor.is_none());
}

#[test]
fn search_page_reaching_unfetched_server_hits_needs_the_server() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    // The server matched UIDs 1..=10, all already cached; only the newest two
    // have been fetched into the snapshot.
    insert_deploy_hits(&conn, 1..=10);
    let folders = ["INBOX".to_string()];
    let pending = (1..=10)
        .map(|uid| crate::store::PendingSearchHit {
            folder: "INBOX".to_string(),
            uid,
            date: uid as i64,
        })
        .collect::<Vec<_>>();
    let token =
        crate::store::create_search_snapshot(&conn, "acct", "deploy", &folders, &pending).unwrap();
    let fetched =
        crate::store::search_messages_in_folders(&conn, "acct", &folders, "deploy", 2, None)
            .unwrap();
    crate::store::finish_search_batch(
        &conn,
        &token,
        "acct",
        &[("INBOX".to_string(), 10), ("INBOX".to_string(), 9)],
        &fetched,
        None,
    )
    .unwrap();

    // A full page inside the fetched part is served locally, with a cursor on.
    let first = expect_page(
        continue_search_page(
            &conn,
            "acct",
            &folders,
            "deploy",
            1,
            &snapshot_cursor(&token, 0),
        )
        .unwrap(),
    );
    assert_eq!(first.messages[0].uid, 10);
    assert!(first.next_cursor.is_some());

    // A page that runs past it asks for the next server batch, carrying the
    // local part as the offline answer, flagged incomplete. That answer goes
    // on through the cached matches below the snapshot instead of ending it.
    let partial = match continue_search_page(
        &conn,
        "acct",
        &folders,
        "deploy",
        5,
        &snapshot_cursor(&token, 0),
    )
    .unwrap()
    {
        SearchContinuation::NeedsServer(partial) => partial,
        SearchContinuation::Page(_) => panic!("unfetched server hits must not be skipped"),
    };
    assert_eq!(
        partial
            .messages
            .iter()
            .map(|message| message.uid)
            .collect::<Vec<_>>(),
        vec![10, 9]
    );
    assert!(partial.incomplete);
    let mut cursor =
        crate::thread_list::parse_search_cursor(partial.next_cursor.as_deref().unwrap()).unwrap();
    let mut rest = Vec::new();
    loop {
        let page = expect_page(
            continue_search_page(&conn, "acct", &folders, "deploy", 5, &cursor).unwrap(),
        );
        assert!(page.incomplete, "the tail passes over pending server hits");
        rest.extend(page.messages.iter().map(|message| message.uid));
        let Some(next) = page.next_cursor else {
            break;
        };
        cursor = crate::thread_list::parse_search_cursor(&next).unwrap();
    }
    assert_eq!(rest, (1..=8).rev().collect::<Vec<_>>());

    let (batch, next) = crate::store::take_search_pending(&conn, &token, 3).unwrap();
    assert_eq!(
        batch.iter().map(|hit| hit.uid).collect::<Vec<_>>(),
        vec![8, 7, 6]
    );
    assert_eq!(next.map(|hit| hit.uid), Some(5));
}

#[test]
fn search_page_reaching_the_snapshot_end_offline_starts_on_the_cache() {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    insert_deploy_hits(&conn, 1..=3);
    let folders = ["INBOX".to_string()];
    let pending = (1..=3)
        .map(|uid| crate::store::PendingSearchHit {
            folder: "INBOX".to_string(),
            uid,
            date: uid as i64,
        })
        .collect::<Vec<_>>();
    let token =
        crate::store::create_search_snapshot(&conn, "acct", "deploy", &folders, &pending).unwrap();

    // Nothing placed yet: the offline answer is the cache itself.
    match continue_search_page(
        &conn,
        "acct",
        &folders,
        "deploy",
        10,
        &snapshot_cursor(&token, 0),
    )
    .unwrap()
    {
        SearchContinuation::NeedsServer(partial) => {
            assert_eq!(
                partial
                    .messages
                    .iter()
                    .map(|message| message.uid)
                    .collect::<Vec<_>>(),
                vec![3, 2, 1]
            );
            assert!(partial.incomplete);
            assert!(partial.next_cursor.is_none());
        }
        SearchContinuation::Page(_) => panic!("unfetched server hits must not be skipped"),
    }
}

#[test]
fn search_batches_place_hits_by_date_not_uid() {
    let hit = |folder: &str, uid: u32, date: i64| crate::store::PendingSearchHit {
        folder: folder.to_string(),
        uid,
        date,
    };
    // UIDs 501..=2 are dated 501..=2, but UID 1 was imported late and is the
    // newest match. Batches are taken by date, so it leads the first one.
    let pending = std::iter::once(hit("INBOX", 1, 1000))
        .chain((2..=501).rev().map(|uid| hit("INBOX", uid, uid as i64)))
        .collect::<Vec<_>>();
    let (batch, next) = pending.split_at(3);
    let fetched = std::collections::HashSet::from(["INBOX"]);
    let (frontier, done) = batch_placement(batch, next.first(), &fetched);
    assert_eq!(frontier, Some((499, 499, "INBOX")));
    assert_eq!(
        done.iter().map(|(_, uid)| *uid).collect::<Vec<_>>(),
        vec![1, 501, 500]
    );

    // A folder whose fetch failed keeps its hits pending, and nothing older
    // than its newest is placed: Sent's 800 is, its 700 waits behind 750.
    let batch = [
        hit("Sent", 9, 800),
        hit("Archive", 4, 750),
        hit("Sent", 8, 700),
    ];
    let next = hit("Sent", 7, 600);
    let fetched = std::collections::HashSet::from(["Sent"]);
    let (frontier, done) = batch_placement(&batch, Some(&next), &fetched);
    assert_eq!(frontier, Some((750, 4, "Archive")));
    assert_eq!(done, vec![("Sent".to_string(), 9)]);

    // When the failed folder holds the newest hit, nothing fetched can be
    // placed, and the batch must report no progress rather than loop.
    let archive_only = std::collections::HashSet::from(["Archive"]);
    let (frontier, done) = batch_placement(&batch, Some(&next), &archive_only);
    assert_eq!(frontier, Some((800, 9, "Sent")));
    assert!(done.is_empty());

    // Nothing left pending: every fetched hit is placed.
    let (frontier, done) = batch_placement(&batch[..1], None, &fetched);
    assert_eq!(frontier, None);
    assert_eq!(done.len(), 1);
}

#[test]
fn folder_search_failure_keeps_other_folder_successes() {
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    record_search_folder_result(
        "INBOX",
        Ok(vec![crate::imap::MessageHeader {
            uid: 7,
            ..Default::default()
        }]),
        &mut successes,
        &mut failures,
    );
    record_search_folder_result(
        "Sent",
        Err(anyhow::anyhow!("SELECT failed")),
        &mut successes,
        &mut failures,
    );

    assert_eq!(successes.len(), 1);
    assert_eq!(successes[0].0, "INBOX");
    assert_eq!(successes[0].1[0].uid, 7);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, "Sent");
}
#[test]
fn live_hits_need_files_only_for_has_attachment() {
    use crate::search_query::SearchQuery;
    let header = |files| imap::MessageHeader {
        files,
        ..Default::default()
    };
    let file = crate::store::CardFile {
        filename: "a.pdf".into(),
        mime: "application/pdf".into(),
    };
    let wanted = SearchQuery::parse("has:attachment");
    assert!(crate::engine::search::live_hit_matches(
        &wanted,
        &header(Some(vec![file]))
    ));
    assert!(!crate::engine::search::live_hit_matches(
        &wanted,
        &header(Some(vec![]))
    ));
    // Gmail answered the query itself.
    assert!(crate::engine::search::live_hit_matches(
        &wanted,
        &imap::MessageHeader {
            gmail_msg_id: Some(1),
            files: Some(vec![]),
            ..Default::default()
        }
    ));
    // Not fetched: nothing says the hit is wrong.
    assert!(crate::engine::search::live_hit_matches(
        &wanted,
        &header(None)
    ));
    assert!(crate::engine::search::live_hit_matches(
        &SearchQuery::parse("plan"),
        &header(Some(vec![]))
    ));
}
