//! Address suggestions and directory queries over cached messages.
//! The store ranks correspondents and resolves contact history across accounts.
//! Directory results exclude automated senders, junk and the account owner.

use super::*;

#[test]
fn suggest_contacts_dedupes_by_address_and_ranks_by_frequency() {
    let conn = test_conn();
    // aki appears twice → ranks above bea; the duplicate collapses to one row.
    insert_message(&conn, 1, "Hi", "Aki", "aki@example.com", None);
    insert_message(&conn, 2, "Re: Hi", "Aki", "aki@example.com", None);
    insert_message(&conn, 3, "Plan", "Bea", "bea@example.com", None);

    let top = suggest_contacts(&conn, "acct", "", 10).unwrap();
    assert_eq!(
        top.iter().map(|c| c.addr.as_str()).collect::<Vec<_>>(),
        vec!["aki@example.com", "bea@example.com"]
    );

    // Query matches on both address and display name, case-insensitively.
    let by_name = suggest_contacts(&conn, "acct", "BEA", 10).unwrap();
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].addr, "bea@example.com");
    assert_eq!(by_name[0].name, "Bea");

    // A different account sees none of acct's correspondents.
    assert!(suggest_contacts(&conn, "other", "", 10).unwrap().is_empty());
}

#[test]
fn contact_history_matches_exact_participants_across_accounts_and_folders() {
    let conn = test_conn();
    let message = |uid, from: &str, to: &str, cc: &str, date| crate::imap::MessageHeader {
        uid,
        subject: "A topic".into(),
        from_addr: from.into(),
        to: vec![crate::imap::Recipient {
            name: "Person".into(),
            addr: to.into(),
        }],
        cc: vec![crate::imap::Recipient {
            name: String::new(),
            addr: cc.into(),
        }],
        date,
        thread_key: format!("mid:{uid}"),
        message_id: format!("{uid}@example.com"),
        ..Default::default()
    };
    upsert_messages(
        &conn,
        "acct",
        "Archive",
        &[message(1, "Alice@example.com", "me@example.com", "", 10)],
    )
    .unwrap();
    upsert_messages(
        &conn,
        "other",
        "Sent",
        &[message(2, "me@example.com", "alice@example.com", "", 20)],
    )
    .unwrap();
    upsert_messages(
        &conn,
        "acct",
        "Projects",
        &[message(
            3,
            "bob@example.com",
            "me@example.com",
            "ALICE@example.com",
            30,
        )],
    )
    .unwrap();
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[message(4, "notalice@example.com", "me@example.com", "", 40)],
    )
    .unwrap();
    let hits = contact_messages(&conn, " alice@example.com ").unwrap();
    assert_eq!(
        hits.iter().map(|(_, h)| h.uid).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    assert_eq!(hits[0].1.folder, "Projects");
    assert_eq!(hits[1].0, "other");
    assert_eq!(hits[1].1.folder, "Sent");
    assert_eq!(hits[0].1.message_id, "3@example.com");
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[message(8, "Élodie@x.com", "me@example.com", "", 1)],
    )
    .unwrap();
    assert_eq!(contact_messages(&conn, "élodie@x.com").unwrap().len(), 1);
    upsert_messages(
        &conn,
        "acct",
        "Sent",
        &[message(9, "me@example.com", "Élodie@x.com", "", 2)],
    )
    .unwrap();
    assert_eq!(contact_messages(&conn, "élodie@x.com").unwrap().len(), 2);
    let first = contact_thread_page(&conn, "alice@example.com", None, 1).unwrap();
    assert_eq!(first["threads"].as_array().unwrap().len(), 1);
    assert!(
        first["threads"][0]["thread_id"]
            .as_str()
            .unwrap()
            .contains("#")
    );
    let cursor = first["next_cursor"].as_str().unwrap();
    upsert_messages(
        &conn,
        "acct",
        "INBOX",
        &[message(5, "alice@example.com", "me@example.com", "", 50)],
    )
    .unwrap();
    let second = contact_thread_page(&conn, "alice@example.com", Some(cursor), 1).unwrap();
    assert_ne!(
        first["threads"][0]["thread_id"],
        second["threads"][0]["thread_id"]
    );
    assert_eq!(second["threads"][0]["date"], 20);
    assert!(
        contact_messages(&conn, "nobody@example.com")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn contacts_exclude_junk_and_trash_using_live_folder_roles() {
    let conn = test_conn();
    let folder = |name: &str, role: &str| Folder {
        name: name.into(),
        special_use: Some(role.into()),
        ..Default::default()
    };
    upsert_folders(
        &conn,
        "acct",
        &[
            folder("迷惑メール", "junk"),
            folder("Deleted items custom", "trash"),
            folder("Spam", LOCAL_SPECIAL_USE),
        ],
    )
    .unwrap();
    let header = |uid, addr: &str, date| crate::imap::MessageHeader {
        uid,
        from_addr: addr.into(),
        date,
        subject: addr.into(),
        thread_key: format!("mid:{uid}"),
        message_id: format!("{uid}@example.com"),
        ..Default::default()
    };
    for (name, message) in [
        ("INBOX", header(1, "alice@example.com", 10)),
        ("迷惑メール", header(2, "alice@example.com", 100)),
        (
            "Deleted items custom",
            header(3, "deleted@example.com", 200),
        ),
        ("Junk", header(4, "junkonly@example.com", 300)),
        ("Spam", header(5, "local@example.com", 20)),
        ("Projects", header(6, "bob@example.com", 30)),
    ] {
        upsert_messages(&conn, "acct", name, &[message]).unwrap();
    }
    let addresses = |sort| {
        list_contacts(&conn, "", "", 20, sort, true)
            .unwrap()
            .into_iter()
            .map(|c| c.addr)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        addresses(ContactSort::Recent),
        vec!["bob@example.com", "local@example.com", "alice@example.com"]
    );
    assert_eq!(
        contact_messages(&conn, "alice@example.com").unwrap().len(),
        1
    );
    assert!(
        contact_messages(&conn, "deleted@example.com")
            .unwrap()
            .is_empty()
    );
    assert!(
        contact_messages(&conn, "junkonly@example.com")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        contact_messages(&conn, "local@example.com").unwrap().len(),
        1
    );
    assert!(
        list_contacts(&conn, "", "deleted@", 20, ContactSort::Name, true)
            .unwrap()
            .is_empty()
    );
    // Metadata arrives after mail caching or is corrected by the provider.
    upsert_folders(&conn, "acct", &[folder("Projects", "trash")]).unwrap();
    assert!(!addresses(ContactSort::Recent).contains(&"bob@example.com".to_string()));
    assert!(
        contact_thread_page(&conn, "bob@example.com", None, 50).unwrap()["threads"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    upsert_folders(&conn, "acct", &[folder("Projects", LOCAL_SPECIAL_USE)]).unwrap();
    assert!(addresses(ContactSort::Recent).contains(&"bob@example.com".to_string()));
}

#[test]
fn list_contacts_sorts_by_frequency_name_or_recency() {
    let conn = test_conn();
    let at = |uid: u32, name: &str, addr: &str, date: i64| {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, seen)
             VALUES('acct', 'INBOX', ?1, ?2, 's', ?3, ?4, ?5, 0)",
            params![uid.to_string(), uid, name, addr, date],
        )
        .unwrap();
    };
    // Zed writes most, Amy most recently, and bob@ has no name.
    at(1, "Zed", "zed@example.com", 100);
    at(2, "Zed", "zed@example.com", 200);
    at(3, "Zed", "zed@example.com", 300);
    at(4, "Amy", "amy@example.com", 900);
    at(5, "", "bob@example.com", 500);
    let order = |sort, query: &str| {
        list_contacts(&conn, "acct", query, 10, sort, false)
            .unwrap()
            .into_iter()
            .map(|c| c.addr)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        order(ContactSort::Frequent, ""),
        ["zed@example.com", "amy@example.com", "bob@example.com"]
    );
    assert_eq!(
        order(ContactSort::Name, ""),
        ["amy@example.com", "bob@example.com", "zed@example.com"]
    );
    assert_eq!(
        order(ContactSort::Recent, ""),
        ["amy@example.com", "bob@example.com", "zed@example.com"]
    );
    // A search keeps the chosen order rather than ranking by match.
    assert_eq!(
        order(ContactSort::Recent, "e"),
        ["amy@example.com", "bob@example.com", "zed@example.com"]
    );
    // A message re-dated later moves its sender up.
    conn.execute("UPDATE messages SET date = 1000 WHERE uid = 5", [])
        .unwrap();
    assert_eq!(order(ContactSort::Recent, "")[0], "bob@example.com");
    assert_eq!(ContactSort::parse("bogus"), ContactSort::Frequent);
}

#[test]
fn contact_directory_hides_own_and_automated_addresses() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO accounts(id, email, prefs)
         VALUES('acct', 'Me@example.com', '{\"aliases\":[{\"email\":\"alias@example.com\"}]}')",
        [],
    )
    .unwrap();
    insert_message(&conn, 1, "a", "Me", "me@example.com", None);
    insert_message(&conn, 2, "b", "Alias", "alias@example.com", None);
    insert_message(
        &conn,
        3,
        "c",
        "Microsoft",
        "account-security-noreply@example.com",
        None,
    );
    insert_message(&conn, 4, "d", "Proton", "no-reply@notify.example.com", None);
    insert_message(&conn, 5, "e", "Bounces", "MAILER-DAEMON@example.com", None);
    insert_message(&conn, 6, "f", "Aki", "aki@example.com", None);
    let addrs = |query: &str, directory| {
        let mut addrs = list_contacts(&conn, "acct", query, 10, ContactSort::Name, directory)
            .unwrap()
            .into_iter()
            .map(|c| c.addr.to_lowercase())
            .collect::<Vec<_>>();
        addrs.sort();
        addrs
    };
    assert_eq!(addrs("", true), ["aki@example.com"]);
    // A search still finds automated senders, never the user's own addresses.
    assert_eq!(
        addrs("example", true),
        [
            "account-security-noreply@example.com",
            "aki@example.com",
            "mailer-daemon@example.com",
            "no-reply@notify.example.com"
        ]
    );
    // Autocomplete keeps everything, so mailing yourself still works.
    assert_eq!(addrs("", false).len(), 6);
}

#[test]
fn contact_directory_hides_no_reply_first_domain_label() {
    let conn = test_conn();
    let hidden = [
        "comment@noreply.github.com",
        "author@NO-REPLY.github.com",
        "subscribed@do_not_reply.example.com",
        "meron@donotreply.github.com",
    ];
    let visible = [
        "person@notifications.example.com",
        "person@bounce.example.com",
        "person@mail.noreply.example.com",
        "person@github.com",
    ];
    for (index, addr) in hidden.iter().chain(visible.iter()).enumerate() {
        insert_message(&conn, index as u32 + 1, addr, "Sender", addr, None);
    }
    let directory = list_contacts(&conn, "acct", "", 20, ContactSort::Name, true).unwrap();
    let mut addresses: Vec<_> = directory.iter().map(|c| c.addr.as_str()).collect();
    addresses.sort();
    let mut expected = visible.to_vec();
    expected.sort();
    assert_eq!(addresses, expected);

    // Hidden domain-based senders remain searchable and available to compose.
    for addr in hidden {
        let found = list_contacts(&conn, "acct", addr, 20, ContactSort::Name, true).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].addr, addr);
    }
    assert_eq!(suggest_contacts(&conn, "acct", "", 20).unwrap().len(), 8);
}

#[test]
fn suggest_contacts_includes_to_and_cc_recipients() {
    use crate::imap::MessageHeader;
    let conn = test_conn();
    // A sent message we authored: the useful contacts are its To/Cc, not us.
    let message = MessageHeader {
        uid: 10,
        subject: "Lunch".into(),
        from_name: "Me".into(),
        from_addr: "me@example.com".into(),
        date: 1_779_580_800, // 2026-05-24
        to: vec![crate::imap::Recipient {
            name: "Cleo".into(),
            addr: "cleo@example.com".into(),
        }],
        cc: vec![crate::imap::Recipient {
            name: String::new(),
            addr: "dan@example.com".into(),
        }],
        ..Default::default()
    };
    upsert_messages(&conn, "acct", "Sent", &[message]).unwrap();

    let cleo = suggest_contacts(&conn, "acct", "cleo", 10).unwrap();
    assert_eq!(cleo.len(), 1);
    assert_eq!(cleo[0].addr, "cleo@example.com");
    assert_eq!(cleo[0].name, "Cleo");

    // A Cc-only address with no display name is still suggested by address.
    let dan = suggest_contacts(&conn, "acct", "dan@", 10).unwrap();
    assert_eq!(
        dan.iter().map(|c| c.addr.as_str()).collect::<Vec<_>>(),
        vec!["dan@example.com"]
    );
}
