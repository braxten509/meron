//! Recurrence overrides affecting a series or its future dates.
//! Cancellation, duration and all-day changes have different effects at the cut date.
//! Queries must retain each occurrence identity even when two start together.

use super::*;

#[test]
fn an_update_that_excludes_an_answered_occurrence_keeps_it_out() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T180000Z\r\nDTEND:20261015T190000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(1) + 3600, week(2)]);

    // The organizer then drops that date from the series.
    let update = crate::calendar::parse_invite(&SERIES.replace(
        "RRULE:FREQ=WEEKLY;COUNT=3\r\n",
        "RRULE:FREQ=WEEKLY;COUNT=3\r\nEXDATE:20261015T170000Z\r\nSEQUENCE:1\r\n",
    ))
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &update, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2)]);
}

#[test]
fn a_cancelled_override_is_not_shown() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(&SERIES.replace(
        "END:VEVENT\r\nEND:VCALENDAR",
        "END:VEVENT\r\nBEGIN:VEVENT\r\nUID:inv-s\r\nRECURRENCE-ID:20261015T170000Z\r\n\
         DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\nSTATUS:CANCELLED\r\n\
         SUMMARY:Review\r\nEND:VEVENT\r\nEND:VCALENDAR",
    ))
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2)]);
}

#[test]
fn a_moved_rdate_occurrence_replaces_the_original() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(&SERIES.replace(
        "RRULE:FREQ=WEEKLY;COUNT=3\r\n",
        "RDATE:20261015T170000Z\r\n",
    ))
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let moved = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T190000Z\r\nDTEND:20261015T200000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &moved, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(1) + 2 * 3600]);
}

#[test]
fn an_occurrence_update_keeps_the_zone_it_defines() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    // 12:00 in a zone the update alone defines, the way Thunderbird names
    // one: its own TZID, pointing at São Paulo (UTC-3, no DST).
    let moved = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VTIMEZONE\r\nTZID:Custom Three\r\n\
         X-LIC-LOCATION:America/Sao_Paulo\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\n\
         TZOFFSETFROM:-0300\r\nTZOFFSETTO:-0300\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID:20261015T170000Z\r\nDTSTART;TZID=Custom Three:20261016T120000\r\n\
         DTEND;TZID=Custom Three:20261016T130000\r\nSUMMARY:Review\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &moved, Response::Accepted).unwrap();
    // 12:00 -03:00 on the 16th is 15:00Z, 22 hours after the original 17:00Z.
    assert_eq!(moved.start, week(1) + 22 * 3600);
    assert_eq!(starts(&conn), vec![week(0), week(1) + 22 * 3600, week(2)]);
}

#[test]
fn a_cancelled_lone_occurrence_stays_out_when_the_series_is_accepted() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Accepted).unwrap();
    let cancel = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID:20261015T170000Z\r\nDTSTART:20261015T170000Z\r\nSTATUS:CANCELLED\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    crate::store::save_cached_message(
        &conn,
        "acct",
        "INBOX",
        8,
        &crate::parse::Message {
            body: "Cancelled".into(),
            invite: Some(cancel),
            ..Default::default()
        },
    )
    .unwrap();
    crate::calendar::remove_from_calendar(
        &conn,
        "acct#INBOX#8",
        std::path::Path::new("/nonexistent"),
    )
    .unwrap();
    assert!(starts(&conn).is_empty());

    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2)]);
}

#[test]
fn an_occurrence_made_all_day_is_all_day_on_its_date() {
    let conn = conn();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let all_day = occurrence_at(
        "20261015T170000Z",
        "DTSTART;VALUE=DATE:20261015\r\nDTEND;VALUE=DATE:20261016\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &all_day, Response::Accepted).unwrap();
    let payload = events_payload(&conn, OCT_1, OCT_1 + 60 * DAY, false).unwrap();
    let items = payload["events"].as_array().unwrap();
    assert_eq!(items.len(), 3, "{payload}");
    let day = items.iter().find(|item| item["all_day"] == true).unwrap();
    assert_eq!(day["start"], OCT_1 + 14 * DAY);
    assert_eq!(day["end"], OCT_1 + 15 * DAY);
    assert_eq!(
        items.iter().filter(|item| item["all_day"] == false).count(),
        2
    );
}

#[test]
fn a_this_and_future_override_keeps_every_later_occurrence() {
    let conn = conn();
    let series = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-f\r\n\
         DTSTART:20261008T170000Z\r\nDTEND:20261008T180000Z\r\nSUMMARY:Daily\r\n\
         RRULE:FREQ=DAILY;COUNT=5\r\nORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:inv-f\r\nRECURRENCE-ID;RANGE=THISANDFUTURE:20261009T170000Z\r\n\
         DTSTART:20261009T190000Z\r\nDTEND:20261009T200000Z\r\nSUMMARY:Daily\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    assert_eq!(starts(&conn).len(), 5);
}

#[test]
fn an_all_day_this_and_future_override_makes_every_later_occurrence_all_day() {
    let conn = conn();
    let series = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-d\r\n\
         DTSTART:20261008T170000Z\r\nDTEND:20261008T180000Z\r\nSUMMARY:Daily\r\n\
         RRULE:FREQ=DAILY;COUNT=5\r\nORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:inv-d\r\nRECURRENCE-ID;RANGE=THISANDFUTURE:20261009T170000Z\r\n\
         DTSTART;VALUE=DATE:20261009\r\nDTEND;VALUE=DATE:20261010\r\nSUMMARY:Daily\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let payload = events_payload(&conn, OCT_1, OCT_1 + 30 * DAY, false).unwrap();
    let items: Vec<(i64, i64, bool)> = payload["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["start"].as_i64().unwrap(),
                item["end"].as_i64().unwrap(),
                item["all_day"].as_bool().unwrap(),
            )
        })
        .collect();
    let day = |n: i64| OCT_1 + (7 + n) * DAY;
    assert_eq!(
        items,
        vec![
            (day(0) + 17 * 3600, day(0) + 18 * 3600, false),
            (day(1), day(2), true),
            (day(2), day(3), true),
            (day(3), day(4), true),
            (day(4), day(5), true),
        ]
    );
}

#[test]
fn a_this_and_future_override_lends_its_length_to_every_later_occurrence() {
    let conn = conn();
    let series = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-l\r\n\
         DTSTART:20261008T170000Z\r\nDTEND:20261008T180000Z\r\nSUMMARY:Daily\r\n\
         RRULE:FREQ=DAILY;COUNT=4\r\nORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:inv-l\r\nRECURRENCE-ID;RANGE=THISANDFUTURE:20261009T170000Z\r\n\
         DTSTART:20261009T170000Z\r\nDTEND:20261009T190000Z\r\nSUMMARY:Daily\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let payload = events_payload(&conn, OCT_1, OCT_1 + 30 * DAY, false).unwrap();
    let lengths: Vec<i64> = payload["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["end"].as_i64().unwrap() - item["start"].as_i64().unwrap())
        .collect();
    assert_eq!(lengths, vec![3600, 7200, 7200, 7200]);
}

#[test]
fn occurrences_starting_together_get_their_own_keys() {
    let conn = conn();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    // The second week moved onto the third's start, for longer.
    let moved = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261022T170000Z\r\nDTEND:20261022T190000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &moved, Response::Accepted).unwrap();
    let payload = events_payload(&conn, OCT_1, OCT_1 + 60 * DAY, false).unwrap();
    let keys: Vec<&str> = payload["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys.len(), 3);
    let unique: std::collections::HashSet<&&str> = keys.iter().collect();
    assert_eq!(unique.len(), 3, "{keys:?}");
}

#[test]
fn an_occurrence_moved_onto_another_keeps_both() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let moved = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261022T170000Z\r\nDTEND:20261022T180000Z\r\nLOCATION:Room 9\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &moved, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2), week(2)]);
}

#[test]
fn an_all_day_this_and_future_override_counts_dates_across_spring_forward() {
    let conn = conn();
    let series = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-dst\r\n\
         DTSTART;TZID=America/New_York:20260305T090000\r\n\
         DTEND;TZID=America/New_York:20260305T100000\r\nSUMMARY:Daily\r\n\
         RRULE:FREQ=DAILY;COUNT=5\r\nORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:inv-dst\r\n\
         RECURRENCE-ID;RANGE=THISANDFUTURE;TZID=America/New_York:20260307T090000\r\n\
         DTSTART;VALUE=DATE:20260307\r\nDTEND;VALUE=DATE:20260308\r\nSUMMARY:Daily\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let march = |day: u32| {
        chrono::NaiveDate::from_ymd_opt(2026, 3, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    };
    let payload = events_payload(&conn, march(1), march(20), false).unwrap();
    let all_day: Vec<i64> = payload["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["all_day"] == true)
        .map(|item| item["start"].as_i64().unwrap())
        .collect();
    assert_eq!(all_day, vec![march(7), march(8), march(9)]);
}

#[test]
fn cancelling_this_and_future_takes_out_the_rest_of_the_series() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let cancel = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID;RANGE=THISANDFUTURE:20261015T170000Z\r\n\
         DTSTART:20261015T170000Z\r\nSTATUS:CANCELLED\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &cancel, Response::Declined).unwrap();
    assert_eq!(starts(&conn), vec![week(0)]);
    let event = store::invite_event(&conn, "acct", "inv-s")
        .unwrap()
        .unwrap();
    assert!(excludes_occurrence(&event, week(1)));
    assert!(excludes_occurrence(&event, week(2)));
    assert!(!excludes_occurrence(&event, week(0)));
}

#[test]
fn a_series_cut_short_stays_cut_when_answered_again() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let cancel = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID;RANGE=THISANDFUTURE:20261015T170000Z\r\n\
         DTSTART:20261015T170000Z\r\nSTATUS:CANCELLED\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &cancel, Response::Declined).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Tentative).unwrap();
    assert_eq!(starts(&conn), vec![week(0)]);
}

#[test]
fn cancelling_this_and_future_takes_out_later_lone_occurrences() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    for (rid, start) in [
        ("20261008T170000Z", "20261008"),
        ("20261015T170000Z", "20261015"),
        ("20261022T170000Z", "20261022"),
    ] {
        let occurrence = occurrence_at(
            rid,
            &format!("DTSTART:{start}T170000Z\r\nDTEND:{start}T180000Z\r\n"),
        );
        record_invite_event(&conn, "acct", "t", "m", &occurrence, Response::Accepted).unwrap();
    }
    assert_eq!(starts(&conn), vec![week(0), week(1), week(2)]);
    let cancel = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID;RANGE=THISANDFUTURE:20261015T170000Z\r\n\
         DTSTART:20261015T170000Z\r\nSTATUS:CANCELLED\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    assert!(cancel.this_and_future);
    record_invite_event(&conn, "acct", "t", "m", &cancel, Response::Declined).unwrap();
    assert_eq!(starts(&conn), vec![week(0)]);
}

#[test]
fn a_this_and_future_cancellation_before_the_series_keeps_the_rest_out() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Accepted).unwrap();
    let cancel = crate::calendar::parse_invite(
        "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID;RANGE=THISANDFUTURE:20261015T170000Z\r\n\
         DTSTART:20261015T170000Z\r\nSTATUS:CANCELLED\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
    )
    .unwrap();
    crate::store::save_cached_message(
        &conn,
        "acct",
        "INBOX",
        9,
        &crate::parse::Message {
            body: "Cancelled".into(),
            invite: Some(cancel),
            ..Default::default()
        },
    )
    .unwrap();
    crate::calendar::remove_from_calendar(
        &conn,
        "acct#INBOX#9",
        std::path::Path::new("/nonexistent"),
    )
    .unwrap();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0)]);
}
