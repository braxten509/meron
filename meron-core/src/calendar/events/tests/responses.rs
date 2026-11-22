//! Responses across edits to a recurring invitation.
//! The fixture combines an invitation series with standalone changed occurrences.
//! Tests exercise restoring attendance, notes and floating occurrence times.

use super::*;

#[test]
fn an_answered_invitation_can_be_put_back_without_answering_again() {
    let conn = conn();
    let invite = crate::calendar::parse_invite(SERIES).unwrap();
    crate::store::save_cached_message(
        &conn,
        "acct",
        "INBOX",
        10,
        &crate::parse::Message {
            body: "Invitation".into(),
            message_id: "inv@x".into(),
            invite: Some(invite.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let missing = std::path::Path::new("/nonexistent");
    // Not answered yet: nothing to put back.
    assert!(crate::calendar::add_to_calendar(&conn, "acct#INBOX#10", "t", missing).is_err());

    store::record_invite_response(&conn, "acct", "inv-s", 0, "tentative", 0).unwrap();
    let id = crate::calendar::add_to_calendar(&conn, "acct#INBOX#10", "t", missing).unwrap();
    let event = store::invite_event(&conn, "acct", "inv-s")
        .unwrap()
        .unwrap();
    assert_eq!(event.id, id);
    assert_eq!(event.response, "tentative");
    assert_eq!(event.message_id, "inv@x");
}

#[test]
fn an_embedded_this_and_future_override_moves_every_later_occurrence() {
    let conn = conn();
    let series = crate::calendar::parse_invite(MOVED_ON).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let day = |n: i64| OCT_1 + (7 + n) * DAY;
    assert_eq!(
        starts(&conn),
        vec![
            day(0) + 17 * 3600,
            day(1) + 19 * 3600,
            day(2) + 19 * 3600,
            day(3) + 19 * 3600,
            day(4) + 19 * 3600,
        ]
    );
}

#[test]
fn a_declined_date_after_a_this_and_future_move_goes() {
    let conn = conn();
    let series = crate::calendar::parse_invite(MOVED_ON).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let third = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-m\r\n\
         RECURRENCE-ID:20261010T170000Z\r\nDTSTART:20261010T190000Z\r\n\
         DTEND:20261010T200000Z\r\nSUMMARY:Daily\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &third, Response::Declined).unwrap();
    let day = |n: i64| OCT_1 + (7 + n) * DAY;
    assert_eq!(
        starts(&conn),
        vec![
            day(0) + 17 * 3600,
            day(1) + 19 * 3600,
            day(3) + 19 * 3600,
            day(4) + 19 * 3600,
        ]
    );
}

#[test]
fn accepting_again_after_a_this_and_future_decline_brings_it_back() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let from_second = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID;RANGE=THISANDFUTURE:20261015T170000Z\r\n\
         DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\nSUMMARY:Review\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &from_second, Response::Declined).unwrap();
    assert_eq!(starts(&conn), vec![week(0)]);

    // Just the third accepted on its own: back, with the second still out.
    let third = occurrence_at(
        "20261022T170000Z",
        "DTSTART:20261022T170000Z\r\nDTEND:20261022T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &third, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2)]);
    let event = store::invite_event(&conn, "acct", "inv-s")
        .unwrap()
        .unwrap();
    assert!(!excludes_occurrence(&event, week(2)));
    assert!(excludes_occurrence(&event, week(1)));

    // The whole range accepted again: everything is back.
    record_invite_event(&conn, "acct", "t", "m", &from_second, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(1), week(2)]);
}

#[test]
fn notes_on_a_lone_occurrence_join_the_series_it_folds_into() {
    let conn = conn();
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Accepted).unwrap();
    let alone = store::invite_occurrence_event(&conn, "acct", "inv-s", second.recurrence_id)
        .unwrap()
        .unwrap();
    update_event(
        &conn,
        &alone.id,
        &EventInput {
            notes: "Bring the Q3 numbers".into(),
            ..input("", 1, 2)
        },
    )
    .unwrap();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let event = store::invite_event(&conn, "acct", "inv-s")
        .unwrap()
        .unwrap();
    assert_eq!(event.notes, "Bring the Q3 numbers");
}

#[test]
fn a_repeating_invitation_without_an_end_lasts_an_instant() {
    let conn = conn();
    let series =
        crate::calendar::parse_invite(&SERIES.replace("DTEND:20261008T180000Z\r\n", "")).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let payload = events_payload(&conn, OCT_1, OCT_1 + 60 * DAY, false).unwrap();
    for item in payload["events"].as_array().unwrap() {
        assert_eq!(item["end"], item["start"], "{payload}");
    }
}

#[test]
fn a_floating_override_in_a_utc_series_is_device_local() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let local_tz = crate::calendar::local_tz();
    let floating = local_tz
        .with_ymd_and_hms(2026, 10, 15, 20, 0, 0)
        .single()
        .unwrap()
        .timestamp();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let moved = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T200000\r\nDTEND:20261015T210000\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &moved, Response::Accepted).unwrap();
    let mut expected = vec![week(0), floating, week(2)];
    expected.sort();
    assert_eq!(starts(&conn), expected);
}

#[test]
fn a_date_accepted_again_past_a_cut_survives_the_series_being_answered_again() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let from_second = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID;RANGE=THISANDFUTURE:20261015T170000Z\r\n\
         DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\nSUMMARY:Review\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    store::record_invite_response(&conn, "acct", "inv-s", week(1), "declined", 0).unwrap();
    store::mark_this_and_future(&conn, "acct", "inv-s", week(1)).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &from_second, Response::Declined).unwrap();
    let third = occurrence_at(
        "20261022T170000Z",
        "DTSTART:20261022T170000Z\r\nDTEND:20261022T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &third, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2)]);

    record_invite_event(&conn, "acct", "t", "m", &series, Response::Tentative).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2)]);
}
