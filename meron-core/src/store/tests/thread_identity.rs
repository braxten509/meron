//! Conversation roots and sending identities in the store.
//! References can arrive out of order, differ in case or form a cycle.
//! Alias cases exercise selection of an address owned by the account.

use super::*;

#[test]
fn thread_key_inheritance_matches_parent_message_id_case_insensitively() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    // Parent cached with its original mixed-case Message-ID.
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[MessageHeader {
            uid: 1,
            subject: "test".into(),
            date: 100,
            thread_key: "Root@Mail.example".into(),
            message_id: "Root@Mail.example".into(),
            ..Default::default()
        }],
    )
    .unwrap();
    // A reply whose computed key fell back to In-Reply-To, with different
    // casing than the cached parent (e.g. a row synced before ids preserved
    // case): the parent lookup must still match and hand down the root key.
    upsert_messages(
        &conn,
        "acct",
        "Sent",
        &[MessageHeader {
            uid: 2,
            subject: "Re: test".into(),
            date: 200,
            thread_key: "root@mail.example".into(),
            message_id: "reply@mailo.com".into(),
            in_reply_to: "root@mail.example".into(),
            ..Default::default()
        }],
    )
    .unwrap();

    let child_key: String = conn
        .query_row("SELECT thread_key FROM messages WHERE uid = 2", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(child_key, "Root@Mail.example");
}

#[test]
fn thread_key_inheritance_settles_a_reply_cached_before_its_parent() {
    use crate::imap::MessageHeader;
    let conn = test_conn();

    // Newest-first sync: the reply lands while the message its key names is
    // still unknown, so nothing can be inherited yet.
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[MessageHeader {
            uid: 2,
            subject: "Re: test".into(),
            date: 200,
            thread_key: "first-reply@local".into(),
            message_id: "second-reply@local".into(),
            ..Default::default()
        }],
    )
    .unwrap();

    // The parent arrives with the canonical root, and hands it down.
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[MessageHeader {
            uid: 1,
            subject: "Re: test".into(),
            date: 100,
            thread_key: "root@mail.example".into(),
            message_id: "First-Reply@Local".into(),
            ..Default::default()
        }],
    )
    .unwrap();

    let child_key: String = conn
        .query_row("SELECT thread_key FROM messages WHERE uid = 2", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(child_key, "root@mail.example");
}

#[test]
fn thread_key_inheritance_terminates_on_a_reference_cycle() {
    use crate::imap::MessageHeader;
    let conn = test_conn();

    // Two messages naming each other: whatever key they settle on, the walk
    // must stop rather than trade keys back and forth.
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[
            MessageHeader {
                uid: 1,
                subject: "test".into(),
                date: 100,
                thread_key: "b@local".into(),
                message_id: "a@local".into(),
                ..Default::default()
            },
            MessageHeader {
                uid: 2,
                subject: "Re: test".into(),
                date: 200,
                thread_key: "a@local".into(),
                message_id: "b@local".into(),
                ..Default::default()
            },
        ],
    )
    .unwrap();

    let keys: Vec<String> = conn
        .prepare("SELECT thread_key FROM messages ORDER BY uid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(keys.len(), 2);
}

#[test]
fn thread_key_lookups_use_their_expression_indexes() {
    let conn = test_conn();
    let plan_of = |sql: &str| -> String {
        conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("\n")
    };

    let by_message_id = plan_of(
        "SELECT thread_key FROM messages
         WHERE account = 'acct'
           AND lower(COALESCE(json_extract(json, '$.message_id'), '')) = 'x'
           AND COALESCE(thread_key, '') <> ''
         ORDER BY date ASC, uid ASC
         LIMIT 1",
    );
    assert!(
        by_message_id.contains("messages_message_id_idx"),
        "message-id lookup fell back to a scan: {by_message_id}"
    );

    let by_thread_key = plan_of(
        "SELECT id FROM messages
         WHERE account = 'acct'
           AND uid <> 0
           AND lower(COALESCE(thread_key, '')) = 'x'
           AND thread_key <> 'y'
           AND thread_key NOT LIKE 'uid:%'
           AND thread_key NOT LIKE 'gmthrid:%'",
    );
    assert!(
        by_thread_key.contains("messages_thread_key_idx"),
        "thread-key lookup fell back to a scan: {by_thread_key}"
    );
}

#[test]
fn thread_key_inheritance_follows_referenced_message_to_canonical_root() {
    use crate::imap::MessageHeader;
    let conn = test_conn();

    // Proton Bridge can return a later reply whose References root is the
    // first sent reply, while In-Reply-To names the newest received message.
    // The first reply has already inherited the conversation's older root.
    upsert_messages(
        &conn,
        "acct",
        "Sent",
        &[MessageHeader {
            uid: 1,
            subject: "Re: test".into(),
            date: 100,
            thread_key: "proton-root@protonmail.internalid".into(),
            message_id: "first-reply@local".into(),
            in_reply_to: "proton-root@protonmail.internalid".into(),
            ..Default::default()
        }],
    )
    .unwrap();

    upsert_messages(
        &conn,
        "acct",
        "Sent",
        &[MessageHeader {
            uid: 2,
            subject: "Re: test".into(),
            date: 200,
            thread_key: "first-reply@local".into(),
            message_id: "second-reply@local".into(),
            in_reply_to: "received-between-replies@example.com".into(),
            ..Default::default()
        }],
    )
    .unwrap();

    let child_key: String = conn
        .query_row("SELECT thread_key FROM messages WHERE uid = 2", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(child_key, "proton-root@protonmail.internalid");
}

#[test]
fn resolve_send_from_honors_account_address_login_and_aliases() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO accounts(id, engine, email, sender_name, prefs)
         VALUES('acct', 'mail', 'me@example.com', 'Me',
                '{\"aliases\":[{\"email\":\"Sales@Example.JP\",\"name\":\"Sales\"},
                               {\"email\":\"quiet@example.com\"}]}')",
        [],
    )
    .unwrap();
    let resolve = |requested| resolve_send_from(&conn, "acct", "login-name", requested);

    // No request: the account's own address, not the login it authenticates as.
    assert_eq!(
        resolve("").unwrap(),
        ("me@example.com".to_string(), "Me".to_string())
    );
    assert_eq!(
        resolve("ME@example.com").unwrap(),
        ("me@example.com".to_string(), "Me".to_string())
    );
    // The login is still a legitimate send-as identity.
    assert_eq!(
        resolve("login-name").unwrap(),
        ("login-name".to_string(), "Me".to_string())
    );
    // Aliases match case-insensitively and keep their own display name.
    assert_eq!(
        resolve(" sales@example.jp ").unwrap(),
        ("Sales@Example.JP".to_string(), "Sales".to_string())
    );
    // A blank alias name inherits the account's sender name.
    assert_eq!(
        resolve("quiet@example.com").unwrap(),
        ("quiet@example.com".to_string(), "Me".to_string())
    );
}

#[test]
fn resolve_send_from_rejects_an_unowned_address() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO accounts(id, engine, email, sender_name, prefs)
         VALUES('acct', 'mail', 'me@example.com', 'Me', '{}')",
        [],
    )
    .unwrap();

    let error = resolve_send_from(&conn, "acct", "me@example.com", "someone@else.com").unwrap_err();
    assert!(
        error.to_string().contains("someone@else.com"),
        "error should name the rejected address: {error}"
    );
}

#[test]
fn resolve_send_from_falls_back_to_the_login_without_an_account_address() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO accounts(id, engine, email, sender_name) VALUES('acct', 'mail', '', 'Me')",
        [],
    )
    .unwrap();

    assert_eq!(
        resolve_send_from(&conn, "acct", "me@example.com", "").unwrap(),
        ("me@example.com".to_string(), "Me".to_string())
    );
}
