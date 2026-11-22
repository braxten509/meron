//! The contact directory and address suggestions.
//! Fixtures include senders, recipients, automated addresses and aliases.
//! Search and ranking cases exercise contacts across folders and accounts.

use super::*;

#[test]
fn apply_card_identity_rewrites_outbound_to_counterparty() {
    use crate::imap::{MessageHeader, Recipient};
    let conn = test_conn();
    conn.execute(
            "INSERT INTO accounts(id, engine, email, prefs)
             VALUES('acct', 'mail', 'me@example.com', '{\"aliases\":[{\"email\":\"alias@other.jp\"}]}')",
            [],
        )
        .unwrap();

    let recipient = |name: &str, addr: &str| Recipient {
        name: name.into(),
        addr: addr.into(),
    };
    let mut headers = vec![
        // Outbound from the primary address → shows first recipient, +1 overflow.
        MessageHeader {
            uid: 1,
            from_name: "Me".into(),
            from_addr: "me@example.com".into(),
            to: vec![
                recipient("Cleo", "cleo@example.com"),
                recipient("Dan", "dan@example.com"),
            ],
            ..Default::default()
        },
        // Outbound from an alias counts as self too.
        MessageHeader {
            uid: 2,
            from_name: "Me".into(),
            from_addr: "Alias@Other.JP".into(),
            to: vec![recipient("Cleo", "cleo@example.com")],
            ..Default::default()
        },
        // Inbound stays untouched.
        MessageHeader {
            uid: 3,
            from_name: "Cleo".into(),
            from_addr: "cleo@example.com".into(),
            ..Default::default()
        },
        // Outbound with no cached recipients (old row / Bcc-only) stays untouched.
        MessageHeader {
            uid: 4,
            from_name: "Me".into(),
            from_addr: "me@example.com".into(),
            ..Default::default()
        },
        // Sent-folder row from an *unconfigured* alias (e.g. webmail send-as):
        // folder provenance alone marks it outbound.
        MessageHeader {
            uid: 5,
            folder: "[Gmail]/Sent Mail".into(),
            from_name: "Me".into(),
            from_addr: "unknown-alias@other.jp".into(),
            to: vec![recipient("Cleo", "cleo@example.com")],
            ..Default::default()
        },
    ];
    apply_card_identity(&conn, "acct", "INBOX", &mut headers);

    assert_eq!(headers[0].from_name, "Cleo");
    assert_eq!(headers[0].from_addr, "cleo@example.com");
    assert_eq!(headers[0].recipient_overflow, 1);
    assert_eq!(headers[1].from_name, "Cleo");
    assert_eq!(headers[1].recipient_overflow, 0);
    assert_eq!(headers[2].from_name, "Cleo");
    assert_eq!(headers[2].from_addr, "cleo@example.com");
    assert_eq!(headers[3].from_name, "Me");
    assert_eq!(headers[3].from_addr, "me@example.com");
    assert_eq!(headers[4].from_name, "Cleo");
    assert_eq!(headers[4].from_addr, "cleo@example.com");
}

#[test]
fn is_outgoing_matches_own_addresses_and_sent_folder_provenance() {
    let mine: std::collections::HashSet<String> =
        ["me@example.com".to_string(), "alias@other.jp".to_string()].into();
    // Own address (any casing), regardless of folder.
    assert!(is_outgoing(&mine, "INBOX", "Me@Example.Com", false));
    assert!(is_outgoing(&mine, "", "alias@other.jp", false));
    // Sent-folder provenance wins even for an unconfigured alias.
    assert!(is_outgoing(
        &mine,
        "[Gmail]/Sent Mail",
        "unknown@other.jp",
        false
    ));
    assert!(is_outgoing(&mine, "Sent", "unknown@other.jp", false));
    // Inbound in a regular folder is not ours.
    assert!(!is_outgoing(&mine, "INBOX", "cleo@example.com", false));
    assert!(!is_outgoing(&mine, "INBOX", "", false));
    // A colleague sending from a shared address configured here as an alias:
    // delivery headers say we received it, wherever it has since been filed.
    assert!(!is_outgoing(&mine, "INBOX", "alias@other.jp", true));
    assert!(!is_outgoing(&mine, "Archive", "alias@other.jp", true));
    assert!(!is_outgoing(&mine, "Trash", "alias@other.jp", true));
    // Our own copy, moved out of Sent, keeps no delivery headers.
    assert!(is_outgoing(&mine, "Archive", "alias@other.jp", false));
    // A self-addressed copy in Sent stays outgoing despite being delivered too.
    assert!(is_outgoing(&mine, "Sent", "me@example.com", true));
    // A local folder named Sent is just a name: received mail kept there stays incoming.
    assert!(!account_message_is_outgoing(
        LOCAL_ACCOUNT_ID,
        &mine,
        "Sent",
        "cleo@example.com",
        false
    ));
    assert!(account_message_is_outgoing(
        LOCAL_ACCOUNT_ID,
        &mine,
        "Sent",
        "me@example.com",
        false
    ));
}

#[test]
fn apply_card_identity_resolves_junk_display_names() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    conn.execute(
        "INSERT INTO accounts(id, engine, email) VALUES('acct', 'mail', 'me@example.com')",
        [],
    )
    .unwrap();
    // A cached inbound message carries the bot's proper display name.
    insert_message(&conn, 1, "Hi", "Austin Mentions", "bot@example.com", None);

    let mut headers = vec![
        // Outbound reply whose To name is the junk "addr addr" pattern.
        MessageHeader {
            uid: 2,
            from_name: "Me".into(),
            from_addr: "me@example.com".into(),
            to: vec![crate::imap::Recipient {
                name: "bot@example.com bot@example.com".into(),
                addr: "bot@example.com".into(),
            }],
            ..Default::default()
        },
        // Inbound with the same junk in From resolves too.
        MessageHeader {
            uid: 3,
            from_name: "bot@example.com bot@example.com".into(),
            from_addr: "bot@example.com".into(),
            ..Default::default()
        },
        // Junk name with no better name cached anywhere is cleared.
        MessageHeader {
            uid: 4,
            from_name: "stranger@example.com".into(),
            from_addr: "stranger@example.com".into(),
            ..Default::default()
        },
    ];
    apply_card_identity(&conn, "acct", "INBOX", &mut headers);

    assert_eq!(headers[0].from_name, "Austin Mentions");
    assert_eq!(headers[0].from_addr, "bot@example.com");
    assert_eq!(headers[1].from_name, "Austin Mentions");
    assert_eq!(headers[2].from_name, "");
}

#[test]
fn contacts_follow_every_message_write() {
    use crate::imap::{MessageHeader, Recipient};
    let conn = test_conn();
    let to = |addr: &str| Recipient {
        name: String::new(),
        addr: addr.to_string(),
    };
    let message = |uid: u32, recipients: Vec<Recipient>| MessageHeader {
        uid,
        subject: "Hi".into(),
        from_name: "Aki".into(),
        from_addr: "Aki@example.com".into(),
        to: recipients,
        ..Default::default()
    };
    let contacts = |conn: &Connection| {
        let mut stmt = conn
            .prepare("SELECT addr_key, name, count FROM contacts ORDER BY addr_key")
            .unwrap();
        stmt.query_map([], |row| {
            Ok(format!(
                "{} {} {}",
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
    };

    // An address listed twice on one message counts it once.
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[
            message(1, vec![to("bea@example.com"), to("BEA@example.com")]),
            message(2, vec![to("cleo@example.com")]),
        ],
    )
    .unwrap();
    assert_eq!(
        contacts(&conn),
        [
            "aki@example.com Aki 2",
            "bea@example.com  1",
            "cleo@example.com  1"
        ]
    );

    // A resync of the same envelope leaves the counts alone; a changed
    // recipient list moves them.
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[message(2, vec![to("cleo@example.com")])],
    )
    .unwrap();
    conn.execute(
        "UPDATE messages SET seen = 1, json = json_set(json, '$.x', 1)",
        [],
    )
    .unwrap();
    assert_eq!(contacts(&conn).len(), 3);
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[message(2, vec![to("dan@example.com")])],
    )
    .unwrap();
    assert_eq!(
        contacts(&conn),
        [
            "aki@example.com Aki 2",
            "bea@example.com  1",
            "dan@example.com  1"
        ]
    );

    // Deleting the last message an address is on drops it.
    conn.execute("DELETE FROM messages WHERE uid = 1", [])
        .unwrap();
    assert_eq!(
        contacts(&conn),
        ["aki@example.com Aki 1", "dan@example.com  1"]
    );

    // Each name is kept apart, so one leaves with its last message.
    let named = |uid: u32, name: &str| MessageHeader {
        uid,
        from_name: name.into(),
        from_addr: "person@example.com".into(),
        ..Default::default()
    };
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[named(3, "Alice"), named(4, "Ally"), named(5, "Ally")],
    )
    .unwrap();
    let person = |query: &str| {
        suggest_contacts(&conn, "acct", query, 10)
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect::<Vec<_>>()
    };
    // Matched by name, it shows the name that matched.
    assert_eq!(person("alice"), ["Alice"]);
    assert_eq!(person("person"), ["Ally"]);
    conn.execute("DELETE FROM messages WHERE uid IN (4, 5)", [])
        .unwrap();
    assert_eq!(person("ally"), Vec::<String>::new());
    assert_eq!(person("alice"), ["Alice"]);
    conn.execute("DELETE FROM messages WHERE uid = 3", [])
        .unwrap();

    // A shared sender matched only by one of its names ranks, and shows, by
    // that name; a word-start match outranks one inside a word.
    let notification = |uid: u32, name: &str| MessageHeader {
        uid,
        from_name: name.into(),
        from_addr: "notifications@github.com".into(),
        ..Default::default()
    };
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[
            notification(10, "Flathub Bot"),
            notification(11, "Flathub Bot"),
            notification(12, "Flathub Bot"),
            notification(13, "ISAN98"),
            message(14, vec![to("isan98@proton.me")]),
            message(15, vec![to("isan98@proton.me")]),
            message(16, vec![to("louisa@example.com")]),
            message(17, vec![to("louisa@example.com")]),
            message(18, vec![to("louisa@example.com")]),
        ],
    )
    .unwrap();
    let isa = suggest_contacts(&conn, "acct", "isa", 10).unwrap();
    assert_eq!(
        isa.iter()
            .map(|c| format!("{} {}", c.name, c.addr))
            .collect::<Vec<_>>(),
        [
            " isan98@proton.me",
            "ISAN98 notifications@github.com",
            " louisa@example.com"
        ]
    );
    conn.execute("DELETE FROM messages WHERE uid >= 10", [])
        .unwrap();

    // Non-ASCII capitals still make one address.
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[
            message(20, vec![to("Éva@example.com")]),
            message(21, vec![to("éva@example.com")]),
        ],
    )
    .unwrap();
    assert_eq!(suggest_contacts(&conn, "acct", "éva", 10).unwrap().len(), 1);
    conn.execute("DELETE FROM messages WHERE uid >= 20", [])
        .unwrap();

    // Feed items aren't correspondents.
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, from_addr)
         VALUES('acct', 'feed', 'item', 0, 'feed@example.com')",
        [],
    )
    .unwrap();
    assert_eq!(
        contacts(&conn),
        ["aki@example.com Aki 1", "dan@example.com  1"]
    );
}

#[test]
fn migration_backfills_contacts_from_cached_mail() {
    let conn = Connection::open_in_memory().unwrap();
    db::migrate_to_v8(&conn).unwrap();
    conn.execute(
        "INSERT INTO messages(account, folder, msg_id, uid, from_name, from_addr, date, json)
         VALUES('acct', 'INBOX', '1', 1, 'Aki', 'aki@example.com', 100,
                '{\"to\":[{\"name\":\"Bea\",\"addr\":\"bea@example.com\"}],
                  \"cc\":[{\"name\":\"\",\"addr\":\"Aki@example.com\"}]}'),
               ('acct', 'Sent', '2', 2, 'Me', 'me@example.com', 200,
                '{\"to\":[{\"name\":\"\",\"addr\":\"bea@example.com\"}]}'),
               ('acct', 'feed', 'item', 0, 'Feed', 'feed@example.com', 300, '{}')",
        [],
    )
    .unwrap();
    db::run_migrations(&conn).unwrap();

    let top = suggest_contacts(&conn, "acct", "", 10).unwrap();
    assert_eq!(
        top.iter()
            .map(|c| format!("{} {}", c.addr, c.name))
            .collect::<Vec<_>>(),
        // Aki sent message 1 and is Cc'd on it nameless: two names, counted
        // once each.
        [
            "aki@example.com Aki",
            "bea@example.com Bea",
            "me@example.com Me"
        ]
    );
    // v23 backfills when each was last seen: bea@ is on both messages.
    let recent = list_contacts(&conn, "acct", "", 10, ContactSort::Recent, false).unwrap();
    assert_eq!(
        recent.iter().map(|c| c.addr.as_str()).collect::<Vec<_>>(),
        ["bea@example.com", "me@example.com", "aki@example.com"]
    );
}

mod directory;
