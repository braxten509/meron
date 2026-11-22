//! Regression tests for ai lessons.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;

fn test_conn(sorting_on: bool) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    super::super::run_migrations(&conn).unwrap();
    super::super::setting_set(
        &conn,
        super::super::AI_SORT_KEY,
        &serde_json::json!({ "enabled": sorting_on, "model": "m" }),
    )
    .unwrap();
    conn
}

fn insert(conn: &Connection, folder: &str, uid: u32, addr: &str, subject: &str, id: &str) {
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, json)
         VALUES('acct', ?1, ?2, ?2, ?3, 'Shop', ?4, 100, ?5)",
        params![folder, uid, subject, addr, serde_json::json!({ "message_id": id }).to_string()],
    )
    .unwrap();
}

fn places(conn: &Connection) -> Vec<(String, String)> {
    ai_lessons(conn)
        .unwrap()
        .into_iter()
        .map(|lesson| (lesson.from_addr, lesson.place))
        .collect()
}

fn tagged(conn: &Connection, place: &LessonPlace) -> Vec<String> {
    let mut keys: Vec<String> = super::super::keys_with_tag(conn, "acct", &place.tag())
        .unwrap()
        .into_iter()
        .collect();
    keys.sort();
    keys
}

#[test]
fn a_move_to_spam_teaches_and_its_undo_forgets() {
    // While AI sorting is off, moves teach nothing.
    let off = test_conn(false);
    insert(
        &off,
        "INBOX",
        1,
        "promo@shop.example",
        "Big sale",
        "<m1@shop>",
    );
    assert!(!ai_lesson_before_move(&off, "acct", "INBOX", "Junk", &[1], false).learn(&off));
    assert!(places(&off).is_empty());

    let conn = test_conn(true);
    insert(
        &conn,
        "INBOX",
        1,
        "promo@shop.example",
        "Big sale",
        "<m1@shop>",
    );
    // A move to a plain folder is no lesson; one to Spam is.
    assert!(!ai_lesson_before_move(&conn, "acct", "INBOX", "Receipts", &[1], false).learn(&conn));
    let lesson = ai_lesson_before_move(&conn, "acct", "INBOX", "Junk", &[1], false);
    conn.execute(
        "UPDATE messages SET folder = 'Junk', uid = 9, msg_id = 9",
        [],
    )
    .unwrap();
    assert!(lesson.learn(&conn));
    assert_eq!(
        places(&conn),
        [("promo@shop.example".to_string(), "spam".to_string())]
    );
    // The email carries the lesson for the reader's other devices.
    assert_eq!(tagged(&conn, &LessonPlace::Spam), ["m1@shop"]);

    // Undo moves it back out of Spam: the lesson and its tag go.
    let undo = ai_lesson_before_move(&conn, "acct", "Junk", "INBOX", &[9], true);
    assert!(undo.learn(&conn));
    assert!(places(&conn).is_empty());
    assert!(tagged(&conn, &LessonPlace::Spam).is_empty());

    // Moved back to the Inbox by hand (not an Undo): keep such mail there.
    let back = ai_lesson_before_move(&conn, "acct", "Junk", "INBOX", &[9], false);
    assert!(back.learn(&conn));
    assert_eq!(
        places(&conn),
        [("promo@shop.example".to_string(), "inbox".to_string())]
    );
    assert_eq!(tagged(&conn, &LessonPlace::Inbox), ["m1@shop"]);

    // A conversation with a colleague's answer in it, sent to Spam whole:
    // nothing tells whose mail the Spam is, so nobody is taught as Spam.
    insert(
        &conn,
        "INBOX",
        2,
        "phish@bad.example",
        "Your account",
        "<m3@bad>",
    );
    insert(
        &conn,
        "INBOX",
        3,
        "colleague@work.example",
        "Re: Your account",
        "<m4@work>",
    );
    insert(
        &conn,
        "INBOX",
        4,
        "bulk@ads.example",
        "Deal one",
        "<m5@ads>",
    );
    insert(
        &conn,
        "INBOX",
        5,
        "bulk@ads.example",
        "Deal two",
        "<m6@ads>",
    );
    conn.execute(
        "UPDATE messages SET thread_key = 't1' WHERE uid IN (2, 3)",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE messages SET thread_key = 't2', date = uid WHERE uid IN (4, 5)",
        [],
    )
    .unwrap();
    let spam: Vec<String> = ai_lesson_seeds(&conn, "acct", "INBOX", &[2, 3, 4, 5], true)
        .unwrap()
        .into_iter()
        .map(|seed| seed.subject)
        .collect();
    assert_eq!(spam, ["Deal two"]);
}

#[test]
fn another_devices_tag_becomes_a_lesson_and_its_removal_forgets_it() {
    let conn = test_conn(true);
    insert(
        &conn,
        "Junk",
        5,
        "spam@bulk.example",
        "You won",
        "<m2@bulk>",
    );
    // A sync reads the lesson tag another device put on the email.
    let seen = [LessonPlace::Spam.tag()];
    super::super::observe_server_tags(&conn, "acct", "Junk", 5, &seen).unwrap();
    assert_eq!(
        places(&conn),
        [("spam@bulk.example".to_string(), "spam".to_string())]
    );
    // When it was taught isn't known: a Spam lesson counts as old as its
    // email, so it never beats a newer "keep in the Inbox" on a guess.
    assert_eq!(ai_lessons(&conn).unwrap()[0].at, 100);

    // The other device forgot the lesson: the tag is gone at the next sync.
    super::super::observe_server_tags(&conn, "acct", "Junk", 5, &[]).unwrap();
    assert!(places(&conn).is_empty());
}
