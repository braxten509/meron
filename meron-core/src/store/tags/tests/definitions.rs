//! Search and rule references across tag edits in the store.
//! The fixtures query messages with one or more tags and rename or delete a tag.
//! Changes must reach the affected rules and copies of the same message.

use super::*;

#[test]
fn tag_search_finds_any_copy_and_skips_removed_tags() {
    let conn = db();
    let mut archived = message(9, "m@x", Some(&["todo"]));
    archived.date = 5;
    upsert(&conn, "Archive", &[archived]);
    upsert(&conn, "INBOX", &[message(1, "other@x", Some(&[]))]);
    let folders = ["INBOX".to_string(), "Archive".to_string()];
    let found = |query: &str| {
        crate::store::search_messages_in_folders(&conn, "a", &folders, query, 10, None)
            .unwrap()
            .len()
    };
    assert_eq!(found("tag:todo"), 1);
    assert_eq!(found("tag:other"), 0);
    change_tags(&conn, "a", &["m@x".into()], &[], &["todo".into()], true).unwrap();
    assert_eq!(found("tag:todo"), 0);
}

#[test]
fn a_search_for_several_tags_wants_them_all() {
    let conn = db();
    upsert(
        &conn,
        "INBOX",
        &[
            message(1, "both@x", Some(&["todo", "work"])),
            message(2, "one@x", Some(&["todo"])),
        ],
    );
    let folders = ["INBOX".to_string()];
    let found = |query: &str| {
        crate::store::search_messages_in_folders(&conn, "a", &folders, query, 10, None)
            .unwrap()
            .len()
    };
    assert_eq!(found("tag:todo"), 2);
    assert_eq!(found("tag:todo tag:work"), 1);
    assert_eq!(found("tag:todo tag:absent"), 0);
}

#[test]
fn a_deleted_tag_leaves_every_account_and_stays_gone_through_a_stale_sync() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["old", "keep"]))]);
    crate::store::upsert_messages(&conn, "b", "INBOX", &[message(1, "m@x", Some(&["old"]))])
        .unwrap();
    assert_eq!(replace_tag(&conn, "old", None, true).unwrap(), ["a", "b"]);
    assert_eq!(tags(&conn, "m@x"), ["keep"]);
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["old", "keep"]))]);
    assert_eq!(tags(&conn, "m@x"), ["keep"], "echo from before the push");
}

#[test]
fn replacing_a_tag_with_itself_or_an_unknown_one_changes_nothing() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["todo"]))]);
    assert!(
        replace_tag(&conn, "todo", Some("todo"), true)
            .unwrap()
            .is_empty()
    );
    assert!(replace_tag(&conn, "absent", None, true).unwrap().is_empty());
    assert_eq!(tags(&conn, "m@x"), ["todo"]);
    assert!(pending_tag_pushes(&conn, "a").unwrap().is_empty());
}

#[test]
fn local_mail_renames_without_waiting_for_a_server() {
    let conn = db();
    upsert(&conn, "Notes", &[message(1, "m@x", None)]);
    change_tags(&conn, "a", &["m@x".into()], &["old".into()], &[], false).unwrap();
    replace_tag(&conn, "old", Some("new"), false).unwrap();
    assert_eq!(tags(&conn, "m@x"), ["new"]);
    assert!(!has_pending_tags(&conn, "a").unwrap());
}

#[test]
fn rules_follow_a_rename_and_lose_a_deleted_tag() {
    let conn = db();
    let rule = |query: &str, tags: &[&str]| Rule {
        query: query.into(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
        ..Default::default()
    };
    set_rules(
        &conn,
        vec![
            rule("from:a", &["old", "keep"]),
            rule("from:b tag:OLD tag:older", &["keep"]),
            rule("from:c", &["old"]),
        ],
    )
    .unwrap();

    let renamed = replace_rule_tag(&conn, "old", Some("new")).unwrap();
    assert_eq!(renamed[0].tags, ["keep", "new"]);
    assert_eq!(renamed[1].query, "from:b tag:new tag:older");
    assert_eq!(renamed[2].tags, ["new"]);

    let deleted = replace_rule_tag(&conn, "new", None).unwrap();
    assert_eq!(deleted.len(), 2, "a rule with nothing left to do is gone");
    assert_eq!(deleted[0].tags, ["keep"]);
    assert_eq!(rules(&conn).unwrap().len(), 2);
}

#[test]
fn a_deleted_tag_leaves_a_rule_its_other_actions() {
    let conn = db();
    let rule = Rule {
        query: "from:a".into(),
        tags: vec!["old".into()],
        star: true,
        ..Default::default()
    };
    set_rules(&conn, vec![rule]).unwrap();
    let kept = replace_rule_tag(&conn, "old", None).unwrap();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].tags.is_empty() && kept[0].star);
}

#[test]
fn rules_may_do_anything_but_nothing() {
    let conn = db();
    let rule = |move_to: &str, folder: &str| Rule {
        query: "from:a".into(),
        move_to: move_to.into(),
        folder: folder.into(),
        ..Default::default()
    };
    assert!(set_rules(&conn, vec![rule("", "")]).is_err());
    assert!(set_rules(&conn, vec![rule("folder", " ")]).is_err());
    assert!(set_rules(&conn, vec![rule("somewhere", "")]).is_err());
    let kept = set_rules(
        &conn,
        vec![rule("trash", "stray"), rule("folder", " Work ")],
    )
    .unwrap();
    assert_eq!(kept[0].folder, "");
    assert_eq!(kept[1].folder, "Work");
}

#[test]
fn a_rename_with_its_rules_changes_both_and_leaves_no_old_tag_for_new_mail() {
    let conn = db();
    upsert(&conn, "INBOX", &[message(1, "m@x", Some(&["old"]))]);
    let rule = Rule {
        query: "subject:s".into(),
        tags: vec!["old".into()],
        ..Default::default()
    };
    set_rules(&conn, vec![rule]).unwrap();

    let (accounts, rules) = replace_tag_and_rules(&conn, "old", Some("new")).unwrap();
    assert_eq!(accounts, ["a"]);
    assert_eq!(rules[0].tags, ["new"]);
    assert_eq!(tags(&conn, "m@x"), ["new"]);

    upsert(&conn, "INBOX", &[message(2, "n@x", Some(&[]))]);
    apply_rules(&conn, "a", "INBOX", 1, &[2]).unwrap();
    assert_eq!(tags(&conn, "n@x"), ["new"]);
    assert_eq!(tag_counts(&conn, None).unwrap().get("old"), None);
}
