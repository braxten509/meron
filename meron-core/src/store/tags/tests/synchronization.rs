//! Tag reconciliation across local mail, server keywords and folder copies.
//! Fixtures track pending writes, refusals and conflicting observations from a later sync.
//! Account and message identity determine which copies share a tag.

use super::*;

#[test]
fn the_key_index_matches_every_query_form() {
    let conn = db();
    for query in [
        format!("SELECT 1 FROM messages WHERE account = 'a' AND uid <> 0 AND {TAG_KEY_SQL} = 'x'"),
        format!(
            "SELECT 1 FROM messages m WHERE m.account = 'a' AND m.uid <> 0 AND {} = 'x'",
            qualified_tag_key("m")
        ),
    ] {
        let plan: String = conn
            .query_row(&format!("EXPLAIN QUERY PLAN {query}"), [], |row| row.get(3))
            .unwrap();
        assert!(plan.contains("messages_tag_key_idx"), "{plan}");
    }
}

#[test]
fn a_keyword_from_another_client_becomes_a_tag_and_leaves_with_it() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "<M@x>", Some(&["todo"]))]);
    assert_eq!(tags(&conn, "m@x"), ["todo"]);
    age_tags(&conn);
    upsert(&conn, "INBOX", &[message(1, "<M@x>", Some(&[]))]);
    assert!(tags(&conn, "m@x").is_empty());
}

#[test]
fn a_copy_that_never_had_the_keyword_keeps_the_tag_and_gets_it_written() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["todo"]))]);
    age_tags(&conn);
    upsert(&conn, "Archive", &[message(9, "m@x", Some(&[]))]);
    upsert(&conn, "Archive", &[message(9, "m@x", Some(&[]))]);
    assert_eq!(tags(&conn, "m@x"), ["todo"]);
    assert_eq!(
        pending_tag_pushes(&conn, "a").unwrap(),
        [TagPush {
            folder: "Archive".into(),
            tag: "todo".into(),
            add: true,
            uids: vec![9],
        }]
    );
}

#[test]
fn a_stale_sync_right_after_a_push_cannot_remove_the_tag() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    change_tags(&conn, "a", &["m@x".into()], &["todo".into()], &[], true).unwrap();
    for push in pending_tag_pushes(&conn, "a").unwrap() {
        record_tag_push(&conn, "a", &push).unwrap();
    }
    settle_pending_tags(&conn, "a", &HashSet::new()).unwrap();
    assert_eq!(state(&conn, "todo"), SYNCED);

    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    assert_eq!(tags(&conn, "m@x"), ["todo"], "echo from before the push");
    assert_eq!(
        pending_tag_pushes(&conn, "a").unwrap().len(),
        1,
        "written again, so server and store converge without another look"
    );
}

#[test]
fn a_stale_sync_right_after_a_removal_cannot_revive_the_tag() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["todo"]))]);
    change_tags(&conn, "a", &["m@x".into()], &[], &["todo".into()], true).unwrap();
    for push in pending_tag_pushes(&conn, "a").unwrap() {
        record_tag_push(&conn, "a", &push).unwrap();
    }
    settle_pending_tags(&conn, "a", &HashSet::new()).unwrap();
    assert_eq!(state(&conn, "todo"), REMOVED);

    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["todo"]))]);
    assert!(tags(&conn, "m@x").is_empty(), "echo from before the push");
    assert_eq!(state(&conn, "todo"), PENDING_REMOVE);
}

#[test]
fn a_tombstone_gives_way_to_a_later_add_and_expires() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["todo"]))]);
    change_tags(&conn, "a", &["m@x".into()], &[], &["todo".into()], true).unwrap();
    for push in pending_tag_pushes(&conn, "a").unwrap() {
        record_tag_push(&conn, "a", &push).unwrap();
    }
    settle_pending_tags(&conn, "a", &HashSet::new()).unwrap();
    change_tags(&conn, "a", &["m@x".into()], &["todo".into()], &[], true).unwrap();
    assert_eq!(tags(&conn, "m@x"), ["todo"], "added back by hand");

    change_tags(&conn, "a", &["m@x".into()], &[], &["todo".into()], true).unwrap();
    settle_pending_tags(&conn, "a", &HashSet::new()).unwrap();
    age_tags(&conn);
    settle_pending_tags(&conn, "a", &HashSet::new()).unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM message_tags", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn a_tag_learned_from_the_server_can_leave_right_away() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["todo"]))]);
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    assert!(tags(&conn, "m@x").is_empty());
}

#[test]
fn removal_elsewhere_clears_the_remaining_copies() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["todo"]))]);
    upsert(&conn, "Archive", &[message(9, "m@x", Some(&["todo"]))]);
    age_tags(&conn);
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    assert!(tags(&conn, "m@x").is_empty());
    assert_eq!(
        pending_tag_pushes(&conn, "a").unwrap(),
        [TagPush {
            folder: "Archive".into(),
            tag: "todo".into(),
            add: false,
            uids: vec![9],
        }]
    );
}

#[test]
fn pushes_cover_every_copy_and_settle_only_when_all_agree() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    upsert(&conn, "Archive", &[message(9, "m@x", Some(&[]))]);
    change_tags(&conn, "a", &["m@x".into()], &["todo".into()], &[], true).unwrap();
    let pushes = pending_tag_pushes(&conn, "a").unwrap();
    assert_eq!(pushes.len(), 2);

    record_tag_push(&conn, "a", &pushes[0]).unwrap();
    settle_pending_tags(&conn, "a", &HashSet::new()).unwrap();
    assert_eq!(state(&conn, "todo"), PENDING_ADD, "one copy still lacks it");

    record_tag_push(&conn, "a", &pushes[1]).unwrap();
    settle_pending_tags(&conn, "a", &HashSet::new()).unwrap();
    assert_eq!(state(&conn, "todo"), SYNCED);
    assert!(pending_tag_pushes(&conn, "a").unwrap().is_empty());
}

#[test]
fn a_refused_keyword_keeps_the_tag_and_blocks_only_that_tag() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    change_tags(
        &conn,
        "a",
        &["m@x".into()],
        &["new".into(), "old".into()],
        &[],
        true,
    )
    .unwrap();
    let refused = HashSet::from([("INBOX".to_string(), "new".to_string())]);
    for push in pending_tag_pushes(&conn, "a").unwrap() {
        if push.tag == "old" {
            record_tag_push(&conn, "a", &push).unwrap();
        }
    }
    settle_pending_tags(&conn, "a", &refused).unwrap();
    assert_eq!(tags(&conn, "m@x"), ["new", "old"]);
    assert_eq!(state(&conn, "old"), SYNCED);
    assert_eq!(state(&conn, "new"), PENDING_ADD);

    change_tags(&conn, "a", &["m@x".into()], &[], &["new".into()], true).unwrap();
    settle_pending_tags(&conn, "a", &refused).unwrap();
    assert_eq!(tags(&conn, "m@x"), ["old"]);
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM message_tags", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 2, "removal done, its tombstone kept for a while");
}

#[test]
fn local_only_changes_never_wait_for_a_server() {
    let conn = db();
    upsert(&conn, "Notes", &[message(1, "m@x", None)]);
    change_tags(&conn, "a", &["m@x".into()], &["todo".into()], &[], false).unwrap();
    assert_eq!(state(&conn, "todo"), SYNCED);
    assert!(pending_tag_pushes(&conn, "a").unwrap().is_empty());
    change_tags(&conn, "a", &["m@x".into()], &[], &["todo".into()], false).unwrap();
    assert!(tags(&conn, "m@x").is_empty());
}

#[test]
fn a_message_without_a_message_id_is_tagged_by_location_until_uids_reset() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(4, "", Some(&["todo"]))]);
    assert_eq!(tags(&conn, "uid:INBOX:4"), ["todo"]);
    crate::store::clear_folder_messages(&conn, "a", "INBOX").unwrap();
    assert!(tags(&conn, "uid:INBOX:4").is_empty());
}

#[test]
fn a_keyword_seen_on_one_copy_is_written_to_the_others() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&[]))]);
    upsert(&conn, "Archive", &[message(9, "m@x", Some(&[]))]);
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["todo"]))]);
    assert_eq!(
        pending_tag_pushes(&conn, "a").unwrap(),
        [TagPush {
            folder: "Archive".into(),
            tag: "todo".into(),
            add: true,
            uids: vec![9],
        }]
    );
}

#[test]
fn mail_cached_before_tags_gets_one_full_read() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", None)]);
    assert!(needs_tag_baseline(&conn, "a", "INBOX").unwrap());
    observe_server_tags(&conn, "a", "INBOX", 1, &["todo".to_string()]).unwrap();
    assert_eq!(tags(&conn, "m@x"), ["todo"]);
    upsert(&conn, "INBOX", &[message(2, "n@x", None)]);
    mark_tag_baseline(&conn, "a", "INBOX").unwrap();
    assert!(!needs_tag_baseline(&conn, "a", "INBOX").unwrap());
}
