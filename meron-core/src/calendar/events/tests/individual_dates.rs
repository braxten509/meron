//! Changes to one occurrence of a recurring invitation.
//! Fixtures move, cancel and restore individual dates in the stored series.
//! The assertions cover identity and local wall time across timezone changes.

use super::*;

#[test]
fn an_answer_about_one_occurrence_leaves_the_rest_of_the_series() {
    let conn = conn();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    assert_eq!(starts(&conn), vec![week(0), week(1), week(2)]);

    // The second week moved an hour later, and accepted.
    let moved = occurrence("DTSTART:20261015T180000Z\r\nDTEND:20261015T190000Z\r\n");
    assert_eq!(moved.recurrence_id, week(1));
    record_invite_event(&conn, "acct", "t", "m", &moved, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(1) + 3600, week(2)]);

    // Then declined: that week alone goes.
    record_invite_event(&conn, "acct", "t", "m", &moved, Response::Declined).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2)]);
    let event = store::invite_event(&conn, "acct", "inv-s")
        .unwrap()
        .unwrap();
    assert_eq!(event.response, "accepted", "the series' own answer stands");
}

#[test]
fn a_changed_occurrence_shows_its_own_title_and_location() {
    let conn = conn();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let moved =
        occurrence("DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\nLOCATION:Room 9\r\n");
    let mut moved_ics = moved.clone();
    moved_ics.ics = moved
        .ics
        .replace("SUMMARY:Review", "SUMMARY:Review (offsite)");
    record_invite_event(&conn, "acct", "t", "m", &moved_ics, Response::Accepted).unwrap();
    let payload = events_payload(&conn, OCT_1, OCT_1 + 60 * DAY, false).unwrap();
    let items = payload["events"].as_array().unwrap();
    let details: Vec<(&str, &str)> = items
        .iter()
        .map(|item| {
            (
                item["title"].as_str().unwrap(),
                item["location"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        details,
        vec![
            ("Review", ""),
            ("Review (offsite)", "Room 9"),
            ("Review", "")
        ]
    );
}

#[test]
fn removing_a_cancelled_occurrence_keeps_the_rest_and_stops_offering_it() {
    let conn = conn();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let cancel = crate::calendar::parse_invite(&format!(
        "BEGIN:VCALENDAR\r\nMETHOD:CANCEL\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID:20261015T170000Z\r\nDTSTART:20261015T170000Z\r\nSTATUS:CANCELLED\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    ))
    .unwrap();
    crate::store::save_cached_message(
        &conn,
        "acct",
        "INBOX",
        7,
        &crate::parse::Message {
            body: "Cancelled".into(),
            invite: Some(cancel.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    let mine = std::collections::HashSet::new();
    let offered = crate::calendar::invite_json_for(&conn, "acct", &cancel, &mine);
    assert_ne!(offered["event_id"], "");

    crate::calendar::remove_from_calendar(
        &conn,
        "acct#INBOX#7",
        std::path::Path::new("/nonexistent"),
    )
    .unwrap();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    assert_eq!(starts(&conn), vec![week(0), week(2)]);
    let after = crate::calendar::invite_json_for(&conn, "acct", &cancel, &mine);
    assert_eq!(after["event_id"], "", "the occurrence is already out");
}

#[test]
fn occurrences_answered_without_their_series_stand_alone_and_fold_in_later() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\nLOCATION:Room 9\r\n",
    );
    let third = occurrence_at(
        "20261022T170000Z",
        "DTSTART:20261022T170000Z\r\nDTEND:20261022T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Accepted).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &third, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(1), week(2)]);

    // Declining one date leaves the other.
    record_invite_event(&conn, "acct", "t", "m", &third, Response::Declined).unwrap();
    assert_eq!(starts(&conn), vec![week(1)]);

    // Accepting the series takes the lone occurrence in, keeping its details.
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(1), week(2)]);
    assert!(
        store::invite_occurrence_events(&conn, "acct", "inv-s")
            .unwrap()
            .is_empty()
    );
    let payload = events_payload(&conn, OCT_1, OCT_1 + 60 * DAY, false).unwrap();
    assert_eq!(payload["events"][1]["location"], "Room 9");

    // Declining the series takes everything.
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Declined).unwrap();
    assert!(starts(&conn).is_empty());
}

#[test]
fn a_moved_occurrence_at_a_newer_sequence_replaces_the_original() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series =
        crate::calendar::parse_invite(&SERIES.replace("RRULE:", "SEQUENCE:0\r\nRRULE:")).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let moved = occurrence_at(
        "20261015T170000Z",
        "SEQUENCE:1\r\nDTSTART:20261015T200000Z\r\nDTEND:20261015T210000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &moved, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(1) + 3 * 3600, week(2)]);
}

#[test]
fn moving_an_occurrence_across_a_daylight_saving_change_keeps_the_series_wall_clock_time() {
    let conn = conn();
    let tz: Tz = "America/New_York".parse().unwrap();
    let at = |day: u32, hour: u32| {
        tz.with_ymd_and_hms(2026, 10, day, hour, 0, 0)
            .single()
            .unwrap()
            .timestamp()
    };
    let nov = |day: u32, hour: u32| {
        tz.with_ymd_and_hms(2026, 11, day, hour, 0, 0)
            .single()
            .unwrap()
            .timestamp()
    };
    let event = store::save_event(
        &conn,
        &CalendarEvent {
            id: "event-dst".into(),
            title: "Run".into(),
            start_at: at(30, 9),
            end_at: at(30, 10),
            recurring: true,
            rrule: "daily".into(),
            tzid: "America/New_York".into(),
            ..Default::default()
        },
    )
    .unwrap();
    // The Oct 31 09:00 occurrence moved a day on, to Nov 1 09:00 (after the
    // clocks go back).
    let input = EventInput {
        occurrence_start: at(31, 9),
        repeat: "daily".into(),
        ..input("Run", nov(1, 9), nov(1, 10))
    };
    let moved = series_input(&event, &input);
    assert_eq!(moved.start, at(31, 9), "still 09:00, a day later");
    assert_eq!(moved.end - moved.start, 3600);
}

#[test]
fn an_occurrence_declined_then_accepted_is_back_and_can_be_cancelled() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Declined).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(1), week(2)]);
    let event = store::invite_event(&conn, "acct", "inv-s")
        .unwrap()
        .unwrap();
    assert!(!excludes_occurrence(&event, week(1)));
}

#[test]
fn an_occurrence_keeps_its_own_answer() {
    let conn = conn();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Tentative).unwrap();
    let payload = events_payload(&conn, OCT_1, OCT_1 + 60 * DAY, false).unwrap();
    let answers: Vec<&str> = payload["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["response"].as_str().unwrap())
        .collect();
    assert_eq!(answers, vec!["accepted", "tentative", "accepted"]);
}

#[test]
fn editing_a_repeating_event_keeps_the_zone_it_was_made_in() {
    let conn = conn();
    store::save_event(
        &conn,
        &CalendarEvent {
            id: "event-tz".into(),
            title: "Run".into(),
            start_at: OCT_1,
            end_at: OCT_1 + 3600,
            recurring: true,
            rrule: "daily".into(),
            tzid: "Pacific/Chatham".into(),
            ..Default::default()
        },
    )
    .unwrap();
    update_event(
        &conn,
        "event-tz",
        &EventInput {
            repeat: "daily".into(),
            ..input("Long run", OCT_1, OCT_1 + 3600)
        },
    )
    .unwrap();
    let saved = store::event(&conn, "event-tz").unwrap().unwrap();
    assert_eq!(saved.title, "Long run");
    assert_eq!(saved.tzid, "Pacific/Chatham");
}

#[test]
fn answering_the_series_again_keeps_the_dates_already_dealt_with() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    // The second week declined, the third moved an hour and accepted.
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\n",
    );
    let third = occurrence_at(
        "20261022T170000Z",
        "DTSTART:20261022T180000Z\r\nDTEND:20261022T190000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Declined).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &third, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2) + 3600]);

    record_invite_event(&conn, "acct", "t", "m", &series, Response::Tentative).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2) + 3600]);
}

#[test]
fn accepting_one_date_of_a_listed_exdate_keeps_the_others_excluded() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(&SERIES.replace(
        "RRULE:FREQ=WEEKLY;COUNT=3\r\n",
        "RRULE:FREQ=WEEKLY;COUNT=4\r\nEXDATE:20261015T170000Z,20261022T170000Z\r\n",
    ))
    .unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(3)]);
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(1), week(3)]);
}

#[test]
fn a_declined_rdate_occurrence_goes() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    let series = crate::calendar::parse_invite(&SERIES.replace(
        "RRULE:FREQ=WEEKLY;COUNT=3\r\n",
        "RDATE:20261015T170000Z\r\n",
    ))
    .unwrap();
    assert!(series.recurring);
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(1)]);
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Declined).unwrap();
    assert_eq!(starts(&conn), vec![week(0)]);
}

#[test]
fn an_occurrence_moved_before_the_series_start_is_still_found() {
    let conn = conn();
    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    // The first week, moved back into September.
    let early = occurrence_at(
        "20261008T170000Z",
        "DTSTART:20260924T170000Z\r\nDTEND:20260924T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &early, Response::Accepted).unwrap();
    let sept = OCT_1 - 30 * DAY;
    let payload = events_payload(&conn, sept, OCT_1, false).unwrap();
    let found: Vec<i64> = payload["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["start"].as_i64().unwrap())
        .collect();
    assert_eq!(found, vec![OCT_1 - 7 * DAY + 17 * 3600]);
}

#[test]
fn a_date_declined_before_the_series_stays_out_once_it_is_accepted() {
    let conn = conn();
    let week = |n: i64| OCT_1 + 7 * DAY + 17 * 3600 + n * 7 * DAY;
    // What `respond` records for a decline about one date.
    store::record_invite_response(&conn, "acct", "inv-s", week(1), "declined", 0).unwrap();
    let second = occurrence_at(
        "20261015T170000Z",
        "DTSTART:20261015T170000Z\r\nDTEND:20261015T180000Z\r\n",
    );
    record_invite_event(&conn, "acct", "t", "m", &second, Response::Declined).unwrap();

    let series = crate::calendar::parse_invite(SERIES).unwrap();
    record_invite_event(&conn, "acct", "t", "m", &series, Response::Accepted).unwrap();
    assert_eq!(starts(&conn), vec![week(0), week(2)]);
}

#[test]
fn turning_an_occurrence_all_day_moves_the_series_by_whole_dates() {
    let conn = conn();
    let tz: Tz = "America/New_York".parse().unwrap();
    let local = |month: u32, day: u32, hour: u32| {
        tz.with_ymd_and_hms(2026, month, day, hour, 0, 0)
            .single()
            .unwrap()
            .timestamp()
    };
    let utc_date = |month: u32, day: u32| {
        chrono::NaiveDate::from_ymd_opt(2026, month, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp()
    };
    let event = store::save_event(
        &conn,
        &CalendarEvent {
            id: "event-convert".into(),
            title: "Run".into(),
            start_at: local(10, 1, 9),
            end_at: local(10, 1, 10),
            recurring: true,
            rrule: "daily".into(),
            tzid: "America/New_York".into(),
            ..Default::default()
        },
    )
    .unwrap();

    // The Nov 5 occurrence (after the clocks went back) made all-day, same date.
    let all_day = series_input(
        &event,
        &EventInput {
            all_day: true,
            occurrence_start: local(11, 5, 9),
            repeat: "daily".into(),
            ..input("Run", utc_date(11, 5), utc_date(11, 6))
        },
    );
    assert_eq!(all_day.start, utc_date(10, 1));
    assert_eq!(all_day.end, utc_date(10, 2));

    // And back: an all-day series made timed on one occurrence, a day later.
    let all_day_event = CalendarEvent {
        start_at: utc_date(10, 1),
        end_at: utc_date(10, 2),
        all_day: true,
        tzid: "America/New_York".into(),
        ..event
    };
    let timed = series_input(
        &all_day_event,
        &EventInput {
            occurrence_start: utc_date(11, 5),
            repeat: "daily".into(),
            ..input("Run", local(11, 6, 9), local(11, 6, 10))
        },
    );
    assert_eq!(timed.start, local(10, 2, 9));
    assert_eq!(timed.end - timed.start, 3600);
}
