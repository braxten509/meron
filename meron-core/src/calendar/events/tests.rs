//! Calendar test fixtures for database queries and invitation changes.
//! The helpers construct event inputs, recurring series and organizer revisions.
//! The child suites exercise those records at different points in their lifecycle.

use super::*;

fn conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    crate::store::run_migrations(&conn).unwrap();
    conn
}

fn input(title: &str, start: i64, end: i64) -> EventInput {
    EventInput {
        title: title.into(),
        start,
        end,
        ..Default::default()
    }
}

fn keys(payload: &Value) -> Vec<(String, i64)> {
    payload["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["title"].as_str().unwrap().to_string(),
                item["start"].as_i64().unwrap(),
            )
        })
        .collect()
}

// 2026-10-01T00:00Z, a Thursday.
const OCT_1: i64 = 1_790_812_800;

const SERIES: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
DTSTART:20261008T170000Z\r\nDTEND:20261008T180000Z\r\nSUMMARY:Review\r\n\
RRULE:FREQ=WEEKLY;COUNT=3\r\nORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn occurrence(extra: &str) -> crate::calendar::Invite {
    crate::calendar::parse_invite(&format!(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID:20261015T170000Z\r\n{extra}SUMMARY:Review\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    ))
    .unwrap()
}

fn starts(conn: &Connection) -> Vec<i64> {
    events_payload(conn, OCT_1, OCT_1 + 60 * DAY, false).unwrap()["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["start"].as_i64().unwrap())
        .collect()
}

fn occurrence_at(rid: &str, extra: &str) -> crate::calendar::Invite {
    crate::calendar::parse_invite(&format!(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-s\r\n\
         RECURRENCE-ID:{rid}\r\n{extra}SUMMARY:Review\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    ))
    .unwrap()
}

const MOVED_ON: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-m\r\n\
     SEQUENCE:0\r\nDTSTART:20261008T170000Z\r\nDTEND:20261008T180000Z\r\nSUMMARY:Daily\r\n\
     RRULE:FREQ=DAILY;COUNT=5\r\nORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\n\
     BEGIN:VEVENT\r\nUID:inv-m\r\nSEQUENCE:1\r\n\
     RECURRENCE-ID;RANGE=THISANDFUTURE:20261009T170000Z\r\n\
     DTSTART:20261009T190000Z\r\nDTEND:20261009T200000Z\r\nSUMMARY:Daily\r\n\
     ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

const MAYA: &str = "maya@example.com";

fn revision(sequence: i64, start: &str, summary: &str) -> Invite {
    crate::calendar::parse_invite(&format!(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-u\r\n\
         SEQUENCE:{sequence}\r\nDTSTART:{start}\r\nDTEND:{start}\r\nSUMMARY:{summary}\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    ))
    .unwrap()
}

/// The series with its second week moved to 20:00 at `sequence`, embedded.
fn series_moving_second(sequence: i64, hour: i64) -> Invite {
    crate::calendar::parse_invite(&format!(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:inv-s\r\nSEQUENCE:0\r\n\
         DTSTART:20261008T170000Z\r\nDTEND:20261008T180000Z\r\nSUMMARY:Review\r\n\
         RRULE:FREQ=WEEKLY;COUNT=3\r\nORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\n\
         BEGIN:VEVENT\r\nUID:inv-s\r\nRECURRENCE-ID:20261015T170000Z\r\nSEQUENCE:{sequence}\r\n\
         DTSTART:20261015T{hour}0000Z\r\nDTEND:20261015T{}0000Z\r\nSUMMARY:Review\r\n\
         ORGANIZER:mailto:maya@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        hour + 1
    ))
    .unwrap()
}

mod individual_dates;
mod query;
mod responses;
mod revisions;
mod series_edits;
