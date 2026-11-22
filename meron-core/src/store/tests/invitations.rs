//! Exercises store behavior for invitations.
//!
//! Uses the shared fixtures in the parent test module.

use super::*;

#[test]
fn invite_responses_are_kept_per_event_and_dropped_with_the_account() {
    let conn = test_conn();
    assert_eq!(invite_response(&conn, "acct", "uid-1", 0).unwrap(), None);
    record_invite_response(&conn, "acct", "uid-1", 0, "tentative", 1).unwrap();
    record_invite_response(&conn, "acct", "uid-1", 0, "accepted", 2).unwrap();
    // One occurrence of the series is a separate answer.
    record_invite_response(&conn, "acct", "uid-1", 1_792_688_400, "declined", 2).unwrap();
    let series = invite_response(&conn, "acct", "uid-1", 0).unwrap().unwrap();
    assert_eq!((series.response.as_str(), series.sequence), ("accepted", 2));
    let occurrence = invite_response(&conn, "acct", "uid-1", 1_792_688_400)
        .unwrap()
        .unwrap();
    assert_eq!(occurrence.response, "declined");
    assert_eq!(invite_response(&conn, "other", "uid-1", 0).unwrap(), None);

    delete_account(&conn, "acct").unwrap();
    assert_eq!(invite_response(&conn, "acct", "uid-1", 0).unwrap(), None);
}

#[test]
fn a_cached_message_keeps_its_invite() {
    let conn = test_conn();
    let invite = crate::calendar::Invite {
        method: "request".into(),
        uid: "uid-1".into(),
        summary: "Launch review".into(),
        ics: "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".into(),
        ..Default::default()
    };
    save_cached_message(
        &conn,
        "acct",
        "INBOX",
        9,
        &Message {
            body: "You are invited".into(),
            invite: Some(invite.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let cached = get_cached_message(&conn, "acct", "INBOX", 9)
        .unwrap()
        .unwrap();
    assert_eq!(cached.invite, Some(invite));
}
