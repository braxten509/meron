//! The local calendar: the shape both frontends render for a range of days, and the writes behind its editor.
//!
//! Like Tasks, everything is on this device: events the user typed in, and the invitations they
//! accepted (filed by [`super::respond`]). A month view asks for a range and gets back occurrences
//! — a repeating event expanded, by calcard, from its own rule — with the open tasks due in it
//! alongside.

mod invitations;
pub use invitations::{apply_cached_updates, apply_invite_update, record_invite_event};
mod recurrence;
pub(crate) use recurrence::excludes_occurrence;
use recurrence::{
    carry_exceptions, cut_from, ends_before, exdates, master_index, overrides, recurrence_instant,
    remove_overrides, settle_start, value_instant, with_occurrence, with_series_sequence,
    without_occurrence,
};

#[cfg(test)]
mod tests;

use anyhow::{Result, anyhow};
use calcard::common::PartialDateTime;
use calcard::common::timezone::Tz;
use calcard::icalendar::{ICalendarComponentType, ICalendarProperty, ICalendarValue};
use chrono::{Datelike, NaiveDate, TimeZone};
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{Invite, Response};
use crate::store::{self, CalendarEvent, Task};

const DAY: i64 = 86_400;

/// How many occurrences of one repeating event are walked from its start.
/// Expansion always begins at DTSTART, so this bounds how far a range can lie
/// past it: a daily event reaches more than fifty years.
const MAX_OCCURRENCES: usize = 20_000;

/// The repeat rules the editor offers, as `events.rrule` stores them.
const REPEATS: [&str; 4] = ["daily", "weekly", "monthly", "yearly"];

/// What the editor sends to create or change an event made here.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct EventInput {
    pub title: String,
    pub notes: String,
    pub location: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    /// One of [`REPEATS`], or empty for once.
    pub repeat: String,
    /// When editing a repeating event, the start of the occurrence the editor
    /// was opened on: `start`/`end` are then that occurrence's new times, and
    /// the series moves by the same amount. 0 for the series itself.
    pub occurrence_start: i64,
    /// `None` keeps the event's alerts; `Some(None)` follows the app-wide
    /// default; otherwise see [`CalendarEvent::alerts`].
    pub alerts: Option<Option<Vec<i64>>>,
}

/// Alerts an event can carry on its own, like Apple Calendar's "Alert" and
/// "Second Alert".
pub const MAX_ALERTS: usize = 2;

/// The longest timed alert lead the core accepts, in minutes: one week.
pub const MAX_ALERT_MINUTES: i64 = 7 * 24 * 60;

/// The longest all-day alert lead the core accepts, in days.
pub const MAX_ALL_DAY_ALERT_DAYS: i64 = 7;

/// Keep the alerts a client sent that the core can schedule: in range for the
/// kind of event, no repeats, at most [`MAX_ALERTS`], in the order given.
fn sanitize_alerts(alerts: &[i64], all_day: bool) -> Vec<i64> {
    let max = if all_day {
        MAX_ALL_DAY_ALERT_DAYS
    } else {
        MAX_ALERT_MINUTES
    };
    let mut out = Vec::new();
    for &lead in alerts {
        if (0..=max).contains(&lead) && !out.contains(&lead) {
            out.push(lead);
        }
    }
    out.truncate(MAX_ALERTS);
    out
}

impl EventInput {
    pub fn from_params(params: &Value) -> Self {
        let text = |key: &str| {
            params
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        Self {
            title: text("title"),
            notes: text("notes"),
            location: text("location"),
            start: params.get("start").and_then(Value::as_i64).unwrap_or(0),
            end: params.get("end").and_then(Value::as_i64).unwrap_or(0),
            all_day: params
                .get("all_day")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            repeat: text("repeat"),
            occurrence_start: params
                .get("occurrence_start")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            alerts: params.get("alerts").map(|alerts| {
                alerts
                    .as_array()
                    .map(|alerts| alerts.iter().filter_map(Value::as_i64).collect())
            }),
        }
    }
}

/// Apply `input` to `event`: an all-day event covers whole UTC days, an end
/// never precedes its start, and a repeating timed event remembers the zone
/// its wall-clock time belongs to.
fn apply_input(event: &mut CalendarEvent, input: &EventInput) -> Result<()> {
    if input.start <= 0 {
        return Err(anyhow!("an event needs a start"));
    }
    let repeat = input.repeat.trim().to_ascii_lowercase();
    if !repeat.is_empty() && !REPEATS.contains(&repeat.as_str()) {
        return Err(anyhow!("unknown repeat: {repeat}"));
    }
    let (start, end) = if input.all_day {
        let start = input.start.div_euclid(DAY) * DAY;
        let end = (input.end.max(input.start) + DAY - 1).div_euclid(DAY) * DAY;
        (start, end.max(start + DAY))
    } else {
        (input.start, input.end.max(input.start))
    };
    event.title = input.title.trim().to_string();
    event.notes = input.notes.trim().to_string();
    event.location = input.location.trim().to_string();
    event.start_at = start;
    event.end_at = end;
    set_all_day(event, input.all_day);
    event.recurring = !repeat.is_empty();
    // A repeat keeps the zone it was made in: edited after travelling, it must
    // not take on the new zone's DST rules.
    event.tzid = if !event.recurring || input.all_day {
        String::new()
    } else if event.tzid.is_empty() {
        super::local_tzid().unwrap_or_default()
    } else {
        event.tzid.clone()
    };
    event.rrule = repeat;
    apply_alerts(event, input);
    Ok(())
}

/// Alerts count minutes for a timed event and days for an all-day one, so a
/// switch between the two follows the default until alerts are set again.
fn set_all_day(event: &mut CalendarEvent, all_day: bool) {
    if event.all_day != all_day {
        event.alerts = None;
    }
    event.all_day = all_day;
}

fn apply_alerts(event: &mut CalendarEvent, input: &EventInput) {
    if let Some(alerts) = &input.alerts {
        event.alerts = alerts
            .as_deref()
            .map(|alerts| sanitize_alerts(alerts, event.all_day));
    }
}

pub fn create_event(conn: &Connection, input: &EventInput) -> Result<Value> {
    let mut event = CalendarEvent {
        id: crate::tasks::new_id("event"),
        ..Default::default()
    };
    apply_input(&mut event, input)?;
    let saved = store::save_event(conn, &event)?;
    Ok(json!({ "event": event_json(&saved) }))
}

/// Change an event made here. An accepted invitation's time and title are the
/// organizer's, so only its notes and alerts are the user's to edit.
pub fn update_event(conn: &Connection, id: &str, input: &EventInput) -> Result<Value> {
    let mut event = store::event(conn, id)?.ok_or_else(|| anyhow!("no such event: {id}"))?;
    if event.invite_uid.is_empty() {
        let input = series_input(&event, input);
        apply_input(&mut event, &input)?;
    } else {
        event.notes = input.notes.trim().to_string();
        apply_alerts(&mut event, input);
    }
    let saved = store::save_event(conn, &event)?;
    Ok(json!({ "event": event_json(&saved) }))
}

/// An edit made on one occurrence of a repeating event, restated for the series: the editor shows
/// the occurrence it was opened on, and moving that occurrence moves every one by the same amount,
/// from the series' own start. Without this, saving a later occurrence would make it the first and
/// drop the ones before it.
fn series_input(event: &CalendarEvent, input: &EventInput) -> EventInput {
    if !event.recurring || input.occurrence_start <= 0 || input.start <= 0 {
        return input.clone();
    }
    let duration = (input.end - input.start).max(0);
    let start = shift_series_start(event, input)
        .unwrap_or(event.start_at + (input.start - input.occurrence_start));
    EventInput {
        start,
        end: start + duration,
        occurrence_start: 0,
        ..input.clone()
    }
}

/// The series' new start, moved as far on the calendar and the clock as the edited occurrence moved, all read in the zone the series repeats in.
///
/// Counting seconds instead would carry a DST change between the two into every occurrence: moving
/// a 09:00 across the change by a day would make the whole series 10:00. An all-day bound is a UTC
/// midnight, not a local time, so when either side is all-day — including a switch between the two
/// — the move is counted in whole dates and the time of day comes from the edit.
fn shift_series_start(event: &CalendarEvent, input: &EventInput) -> Option<i64> {
    let tz = event
        .tzid
        .parse::<Tz>()
        .ok()
        .filter(|_| !event.tzid.is_empty())
        .unwrap_or_else(super::local_tz);
    let local = |at: i64| Some(tz.timestamp_opt(at, 0).single()?.naive_local());
    let utc = |at: i64| Some(chrono::DateTime::from_timestamp(at, 0)?.naive_utc());
    let at = |at: i64, all_day: bool| if all_day { utc(at) } else { local(at) };

    if !event.all_day && !input.all_day {
        let shifted =
            local(event.start_at)? + (local(input.start)? - local(input.occurrence_start)?);
        return Some(tz.from_local_datetime(&shifted).earliest()?.timestamp());
    }
    let days =
        at(input.start, input.all_day)?.date() - at(input.occurrence_start, event.all_day)?.date();
    let date = at(event.start_at, event.all_day)?.date() + days;
    if input.all_day {
        Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp())
    } else {
        let time = local(input.start)?.time();
        Some(
            tz.from_local_datetime(&date.and_time(time))
                .earliest()?
                .timestamp(),
        )
    }
}

/// Remove an event. Returns it whole, so the caller can offer an undo that
/// hands it straight back to [`restore_event`].
pub fn delete_event(conn: &Connection, id: &str) -> Result<Value> {
    let event = store::event(conn, id)?;
    store::delete_event(conn, id)?;
    Ok(json!({
        "ok": true,
        "restore": event.map(|event| serde_json::to_value(event).unwrap_or(Value::Null)),
    }))
}

pub fn restore_event(conn: &Connection, restore: &Value) -> Result<Value> {
    let event: CalendarEvent = serde_json::from_value(restore.clone())
        .map_err(|error| anyhow!("not an event to restore: {error}"))?;
    if event.id.is_empty() {
        return Err(anyhow!("not an event to restore"));
    }
    let saved = store::save_event(conn, &event)?;
    Ok(json!({ "event": event_json(&saved) }))
}

pub fn event_payload(conn: &Connection, id: &str) -> Result<Value> {
    Ok(json!({ "event": store::event(conn, id)?.as_ref().map(event_json) }))
}

pub fn event_json(event: &CalendarEvent) -> Value {
    json!({
        "id": event.id,
        "title": event.title,
        "notes": event.notes,
        "location": event.location,
        "start": event.start_at,
        "end": event.end_at,
        "all_day": event.all_day,
        "recurring": event.recurring,
        "repeat": event.rrule,
        "account": event.account,
        "thread_id": event.thread_id,
        "message_id": event.message_id,
        "response": event.response,
        "created_at": event.created_at,
        "reminder_after": event.reminder_after,
        "alerts": event.alerts,
        // The kind `alerts` counts in; an occurrence can differ (see
        // `events_payload`).
        "alerts_all_day": event.all_day,
        "source": if event.invite_uid.is_empty() { "event" } else { "invite" },
    })
}

/// Everything in `[from, to)` for a calendar view: each event's occurrences
/// there and, with `include_tasks`, the open tasks due in it, soonest first.
/// Each item has a `key` unique within the answer, for list identity.
pub fn events_payload(conn: &Connection, from: i64, to: i64, include_tasks: bool) -> Result<Value> {
    if to <= from {
        return Ok(json!({ "events": [] }));
    }
    let mut items: Vec<(i64, Value)> = Vec::new();
    let mut keys = std::collections::HashSet::new();
    for event in store::events_between(conn, from, to)? {
        for occurrence in occurrences(&event, from, to) {
            let start = occurrence.start;
            let mut item = event_json(&event);
            // Two occurrences can start together (one moved onto another), and
            // clients key their lists by this.
            let base = format!("{}@{start}", event.id);
            let key = (0..)
                .map(|n| {
                    if n == 0 {
                        base.clone()
                    } else {
                        format!("{base}#{n}")
                    }
                })
                .find(|key| !keys.contains(key))
                .unwrap_or(base);
            keys.insert(key.clone());
            item["key"] = json!(key);
            item["start"] = json!(start);
            item["end"] = json!(occurrence.end);
            if let Some(all_day) = occurrence.all_day {
                item["all_day"] = json!(all_day);
            }
            if let Some(title) = occurrence.title {
                item["title"] = json!(title);
            }
            if let Some(location) = occurrence.location {
                item["location"] = json!(location);
            }
            if let Some(response) = occurrence.response {
                item["response"] = json!(response);
            }
            items.push((start, item));
        }
    }
    if include_tasks {
        // A due date is a local day (see `fromDateInputValue`), so look a day
        // either side and keep the ones whose day falls in the range.
        for task in store::tasks_due_between(conn, from - DAY, to + DAY)? {
            let day = local_day_as_utc(task.due_at);
            if task.due_has_time {
                if task.due_at >= from && task.due_at < to {
                    items.push((task.due_at, task_json(&task, day)));
                }
            } else if day < to && day + DAY > from {
                items.push((day, task_json(&task, day)));
            }
        }
    }
    items.sort_by(|(a, left), (b, right)| {
        a.cmp(b)
            .then_with(|| right["all_day"].as_bool().cmp(&left["all_day"].as_bool()))
            .then_with(|| left["key"].as_str().cmp(&right["key"].as_str()))
    });
    Ok(json!({ "events": items.into_iter().map(|(_, item)| item).collect::<Vec<_>>() }))
}

fn task_json(task: &Task, day: i64) -> Value {
    json!({
        "id": task.id,
        "key": format!("task:{}", task.id),
        "source": "task",
        "title": task.title,
        "notes": task.notes,
        "location": "",
        "start": if task.due_has_time { task.due_at } else { day },
        "end": if task.due_has_time { task.due_at } else { day + DAY },
        "all_day": !task.due_has_time,
        "recurring": false,
        "repeat": "",
        "list_id": task.list_id,
        "account": task.account,
        "thread_id": task.thread_id,
        "message_id": task.message_id,
        "response": "",
    })
}

/// The local calendar day `at` falls on, as that date's UTC midnight — how
/// all-day items are placed.
fn local_day_as_utc(at: i64) -> i64 {
    chrono::Local
        .timestamp_opt(at, 0)
        .single()
        .map(|time| {
            let date = time.date_naive();
            NaiveDate::from_ymd_opt(date.year(), date.month(), date.day())
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|midnight| midnight.and_utc().timestamp())
                .unwrap_or(at)
        })
        .unwrap_or(at)
}

/// One time an event happens. An occurrence the organizer changed on its own
/// (an override VEVENT) can carry its own title and location.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Occurrence {
    start: i64,
    end: i64,
    /// The VEVENT it came from, so two occurrences that land on the same
    /// times — one moved onto another — stay two.
    component: u32,
    /// An override that is all-day while its series is timed, or the other
    /// way round.
    all_day: Option<bool>,
    title: Option<String>,
    location: Option<String>,
    /// The user's answer about this occurrence alone, when it differs from
    /// the event's (see [`OCCURRENCE_RESPONSE`]).
    response: Option<String>,
}

/// Where an occurrence's own answer is kept in a stored invitation: a private
/// property on its override VEVENT, which never leaves the device.
const OCCURRENCE_RESPONSE: &str = "X-MERON-RESPONSE";

/// An override's SEQUENCE as the organizer sent it, kept beside the series'
/// one it is given (see [`put_override`]) so a late, older revision of the
/// occurrence is known for one. A private property, like [`OCCURRENCE_RESPONSE`].
const OWN_SEQUENCE: &str = "X-MERON-SEQUENCE";

/// Note an override's own SEQUENCE before it takes the series', once: an
/// override filed again keeps what was first noted.
fn keep_own_sequence(component: &mut calcard::icalendar::ICalendarComponent) {
    let name = ICalendarProperty::Other(OWN_SEQUENCE.to_string());
    if component.property(&name).is_some() {
        return;
    }
    // None given is 0 (RFC 5545), not the series' count it is about to take.
    let sequence = component
        .property(&ICalendarProperty::Sequence)
        .and_then(|entry| entry.values.first())
        .and_then(ICalendarValue::as_integer)
        .unwrap_or(0);
    component.add_property(name, ICalendarValue::Text(sequence.to_string()));
}

/// An event's SEQUENCE as the organizer sent it.
fn own_sequence(component: &calcard::icalendar::ICalendarComponent) -> i64 {
    super::text_of(
        component,
        ICalendarProperty::Other(OWN_SEQUENCE.to_string()),
    )
    .parse()
    .ok()
    .or_else(|| {
        component
            .property(&ICalendarProperty::Sequence)
            .and_then(|entry| entry.values.first())
            .and_then(ICalendarValue::as_integer)
    })
    .unwrap_or(0)
}

/// An event's occurrences that overlap `[from, to)`.
fn occurrences(event: &CalendarEvent, from: i64, to: i64) -> Vec<Occurrence> {
    let overlaps =
        |start: i64, end: i64| start < to && (end > from || (end <= start && start >= from));
    let single = || {
        overlaps(event.start_at, event.end_at)
            .then(|| Occurrence {
                start: event.start_at,
                end: event.end_at,
                component: 0,
                all_day: None,
                title: None,
                location: None,
                response: None,
            })
            .into_iter()
            .collect()
    };
    if !event.recurring {
        return single();
    }
    let ics = if event.ics.is_empty() {
        repeat_ics(event)
    } else {
        event.ics.clone()
    };
    let Some(ical) = super::parse_ical(&ics) else {
        return single();
    };
    let expanded = ical.expand_dates(expansion_tz(&ical, event.all_day), MAX_OCCURRENCES);
    // calcard leaves out EXDATEs, and dates an override replaces, from the
    // RRULE's dates but not from RDATE's, and a THISANDFUTURE override's later
    // dates are checked against their moved times rather than their original
    // ones, so every occurrence's original time is checked here.
    let excluded = exdates(&ical);
    let overridden: Vec<i64> = overrides(&ical)
        .filter_map(|component| recurrence_instant(&ical, component))
        .collect();
    let ends_before = ends_before(&ical);
    let endless = !event.all_day
        && master_index(&ical).is_some_and(|master| {
            let master = &ical.components[master];
            !master.has_property(&ICalendarProperty::Dtend)
                && !master.has_property(&ICalendarProperty::Duration)
        });
    // Each override's first expanded start, which its own DTSTART stands for:
    // with RANGE=THISANDFUTURE calcard gives the same override every later
    // occurrence too, each its original time moved by the override's own move.
    let mut override_origins = std::collections::HashMap::new();
    let mut out: Vec<Occurrence> = expanded
        .events
        .iter()
        .filter_map(|occurrence| {
            let component = ical.components.get(occurrence.comp_id as usize)?;
            if component.component_type != ICalendarComponentType::VEvent {
                return None;
            }
            let expanded = occurrence.timestamps().0;
            let rid = component
                .is_recurrence_override()
                .then(|| recurrence_instant(&ical, component))
                .flatten();
            let origin = rid.map(|_| {
                *override_origins
                    .entry(occurrence.comp_id)
                    .or_insert(expanded)
            });
            // The time this occurrence had before any override moved it.
            let original = match (rid, origin) {
                (Some(rid), Some(origin)) => expanded - (origin - rid),
                _ => expanded,
            };
            // Cancelled from a date on (RANGE=THISANDFUTURE) — except an
            // occurrence accepted on its own since, which the cut removed any
            // earlier override of.
            let accepted_since = rid == Some(original);
            if !accepted_since && ends_before.is_some_and(|cut| original >= cut) {
                return None;
            }
            // An override the organizer made all-day (or timed) on its own is
            // read from its own DTSTART and DTEND: calcard reads its dates in
            // the series' zone. With RANGE=THISANDFUTURE calcard gives the
            // same override every later occurrence too, so each of those is
            // the override's own bounds moved by how far its expanded start
            // is from the first one's — whole days for an all-day one.
            let own_bounds = component
                .is_recurrence_override()
                .then(|| override_bounds(&ical, component))
                .flatten()
                .map(|(start, end, all_day)| {
                    // One of the same type is its own bounds, and a later
                    // occurrence of a THISANDFUTURE one is its expanded start
                    // moved as far as the first one's was from them — calcard
                    // reads a floating time in the series' zone rather than
                    // the device's — with the override's length, where calcard
                    // keeps the series'.
                    if all_day == event.all_day {
                        let moved = expanded + (start - origin.unwrap_or(expanded));
                        return (moved, moved + (end - start), all_day);
                    }
                    // Counted from the first occurrence it covers: for an
                    // all-day override in whole dates between the original and
                    // the overridden one in the series' zone — across a DST
                    // change they are 23 or 25 hours apart.
                    let origin = origin.unwrap_or(expanded);
                    let offset = if !all_day {
                        expanded - origin
                    } else if let Some(rid) = rid {
                        let zone = occurrence.start.timezone();
                        let date =
                            |at: i64| zone.timestamp_opt(at, 0).single().map(|t| t.date_naive());
                        match (date(original), date(rid)) {
                            (Some(to), Some(from)) => (to - from).num_days() * DAY,
                            _ => (expanded - origin).div_euclid(DAY) * DAY,
                        }
                    } else {
                        (expanded - origin).div_euclid(DAY) * DAY
                    };
                    (start + offset, end + offset, all_day)
                });
            let (start, end) = own_bounds
                .map(|(start, end, _)| (start, end))
                .unwrap_or_else(|| {
                    let (start, end) = occurrence.timestamps();
                    // A timed series with no end lasts an instant (RFC 5545
                    // 3.6.1); calcard runs each occurrence to the end of its day.
                    (start, if endless { start } else { end })
                });
            // A cancelled occurrence (an override saying so) is not on.
            if !overlaps(start, end)
                || excluded.contains(&original)
                || (!component.is_recurrence_override() && overridden.contains(&original))
                || component.status() == Some(&calcard::icalendar::ICalendarStatus::Cancelled)
            {
                return None;
            }
            let own = |prop: ICalendarProperty| {
                component
                    .is_recurrence_override()
                    .then(|| super::text_of(component, prop))
                    .filter(|text| !text.is_empty())
            };
            Some(Occurrence {
                start,
                end,
                component: occurrence.comp_id,
                all_day: own_bounds
                    .map(|(_, _, all_day)| all_day)
                    .filter(|all_day| *all_day != event.all_day),
                title: own(ICalendarProperty::Summary),
                location: own(ICalendarProperty::Location),
                response: own(ICalendarProperty::Other(OCCURRENCE_RESPONSE.to_string())),
            })
        })
        .collect();
    out.sort();
    // calcard can give one date twice (an RDATE on the DTSTART).
    out.dedup_by(|a, b| a.start == b.start && a.end == b.end && a.component == b.component);
    out
}

/// An override's own start, end and whether it is all-day: a date at its UTC
/// midnight (end exclusive, a day when absent), a time in its TZID's zone.
fn override_bounds(
    ical: &calcard::icalendar::ICalendar,
    component: &calcard::icalendar::ICalendarComponent,
) -> Option<(i64, i64, bool)> {
    let entry = component.property(&ICalendarProperty::Dtstart)?;
    let value = entry.values.first()?;
    let all_day = !value.as_partial_date_time()?.has_time();
    let start = value_instant(ical, entry, value)?;
    let end = component
        .property(&ICalendarProperty::Dtend)
        .and_then(|entry| value_instant(ical, entry, entry.values.first()?))
        .or_else(|| {
            match component
                .property(&ICalendarProperty::Duration)?
                .values
                .first()?
            {
                ICalendarValue::Duration(duration) => Some(start + duration.as_seconds()),
                _ => None,
            }
        })
        .unwrap_or(if all_day { start + DAY } else { start })
        .max(start);
    Some((start, end, all_day))
}

/// The zone a calendar's times without a TZID are expanded in. Dates without a time are UTC
/// midnights here, and times without a zone the device's. A time in UTC (`…Z`) or at an offset is
/// in that: calcard expands a repeat in the TZID's zone or the default, not the start's own offset,
/// so a weekly 17:00Z would otherwise land at 17:00 local.
fn expansion_tz(ical: &calcard::icalendar::ICalendar, all_day: bool) -> Tz {
    if all_day {
        return Tz::UTC;
    }
    let offset = ical
        .components
        .iter()
        .filter(|component| component.component_type == ICalendarComponentType::VEvent)
        .filter_map(|component| component.property(&ICalendarProperty::Dtstart))
        .find(|entry| entry.tz_id().is_none())
        .and_then(|entry| entry.values.first()?.as_partial_date_time())
        .and_then(|start| {
            let hours = i32::from(start.tz_hour?);
            let minutes = i32::from(start.tz_minute.unwrap_or(0));
            let seconds = (hours * 3600 + minutes * 60) * if start.tz_minus { -1 } else { 1 };
            chrono::FixedOffset::east_opt(seconds)
        });
    match offset {
        Some(offset) if offset.local_minus_utc() == 0 => Tz::UTC,
        Some(offset) => Tz::Fixed(offset),
        None => super::local_tz(),
    }
}

/// The calendar an event made here repeats by: its first occurrence and the
/// rule, timed in the zone it was made in so 09:00 stays 09:00 across DST.
fn repeat_ics(event: &CalendarEvent) -> String {
    let freq = event.rrule.to_ascii_uppercase();
    let (dtstart, dtend) = if event.all_day {
        let date = |at: i64| {
            chrono::DateTime::from_timestamp(at, 0)
                .map(|time| time.format("%Y%m%d").to_string())
                .unwrap_or_default()
        };
        (
            format!("DTSTART;VALUE=DATE:{}", date(event.start_at)),
            format!("DTEND;VALUE=DATE:{}", date(event.end_at)),
        )
    } else {
        match event
            .tzid
            .parse::<Tz>()
            .ok()
            .filter(|_| !event.tzid.is_empty())
        {
            Some(tz) => {
                let local = |at: i64| {
                    tz.timestamp_opt(at, 0)
                        .single()
                        .map(|time| time.naive_local().format("%Y%m%dT%H%M%S").to_string())
                        .unwrap_or_default()
                };
                (
                    format!("DTSTART;TZID={}:{}", event.tzid, local(event.start_at)),
                    format!("DTEND;TZID={}:{}", event.tzid, local(event.end_at)),
                )
            }
            None => {
                let utc = |at: i64| {
                    chrono::DateTime::from_timestamp(at, 0)
                        .map(|time| time.format("%Y%m%dT%H%M%SZ").to_string())
                        .unwrap_or_default()
                };
                (
                    format!("DTSTART:{}", utc(event.start_at)),
                    format!("DTEND:{}", utc(event.end_at)),
                )
            }
        }
    };
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Meron//Meron//EN\r\nBEGIN:VEVENT\r\n\
         UID:{}\r\n{dtstart}\r\n{dtend}\r\nRRULE:FREQ={freq}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        event.id
    )
}

/// Where a series' end is kept once it has been cut short: declined or
/// cancelled with RANGE=THISANDFUTURE, every occurrence from that time on goes.
/// A private property on the series' VEVENT, holding Unix seconds.
const ENDS_BEFORE: &str = "X-MERON-ENDS-BEFORE";
