//! Parse mail invitations and record the recipient's attendance decision.
//!
//! Calendar invitations (iMIP, RFC 6047): the event a `text/calendar` part describes, the card both
//! frontends render for it, and the `METHOD:REPLY` an Accept / Tentative / Decline sends back to the
//! organizer.
//!
//! Parsing and timezone resolution are calcard's. Outlook names its zones the Windows way ("Pacific Standard
//! Time") and ships a VTIMEZONE; Google sends UTC; Apple sends IANA ids. The resolver it builds from the
//! calendar's own VTIMEZONE blocks covers all three, so an event lands on the same instant whoever sent it.

mod responses;
#[cfg(test)]
use responses::parse_message_ref;
pub use responses::{add_to_calendar, remove_from_calendar, reply_ics, reply_mail, respond};

pub mod events;
#[cfg(test)]
mod tests;

use std::collections::HashSet;

use anyhow::{Result, anyhow};
use calcard::common::PartialDateTime;
use calcard::common::timezone::Tz;
use calcard::icalendar::{
    ICalendar, ICalendarComponent, ICalendarComponentType, ICalendarEntry, ICalendarMethod,
    ICalendarParameter, ICalendarParameterName, ICalendarParameterValue,
    ICalendarParticipationStatus, ICalendarProperty, ICalendarStatus, ICalendarValue, Uri,
};
use serde_json::{Value, json};

/// Calendar parts bigger than this are not read as invites. A real invitation
/// is a few kilobytes even with a long VTIMEZONE; a multi-megabyte `.ics` is a
/// whole exported calendar, which a card cannot describe anyway.
const MAX_ICS_BYTES: usize = 512 * 1024;

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Person {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub addr: String,
}

#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Attendee {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub addr: String,
    /// Lowercase PARTSTAT (`needs-action`, `accepted`, `tentative`,
    /// `declined`, `delegated`), `needs-action` when absent as RFC 5545 says.
    #[serde(default)]
    pub partstat: String,
}

/// One event a calendar part carries, as the card shows it.
#[derive(Debug, Default, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Invite {
    /// Lowercase iTIP method: `request`, `cancel`, `reply`, `publish`, … Empty
    /// for a bare `.ics` with no METHOD, which reads as `publish`.
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub sequence: i64,
    /// Which occurrence of a recurring event this is about, as Unix seconds;
    /// 0 for the whole series.
    #[serde(default)]
    pub recurrence_id: i64,
    /// Whether the occurrence it names stands for every later one too
    /// (`RECURRENCE-ID;RANGE=THISANDFUTURE`).
    #[serde(default)]
    pub this_and_future: bool,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub description: String,
    /// Start and end as Unix seconds. For an all-day event they are UTC
    /// midnights and `end` is exclusive, so clients format them in UTC.
    #[serde(default)]
    pub start: i64,
    #[serde(default)]
    pub end: i64,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub recurring: bool,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub organizer: Person,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    /// The calendar text this was read from. A reply is built from it, so it
    /// keeps the VTIMEZONE the organizer's DTSTART names.
    #[serde(default)]
    pub ics: String,
}

/// An answer to an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    Accepted,
    Tentative,
    Declined,
}

impl Response {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "accepted" | "accept" => Some(Self::Accepted),
            "tentative" => Some(Self::Tentative),
            "declined" | "decline" => Some(Self::Declined),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Tentative => "tentative",
            Self::Declined => "declined",
        }
    }

    fn partstat(self) -> ICalendarParticipationStatus {
        match self {
            Self::Accepted => ICalendarParticipationStatus::Accepted,
            Self::Tentative => ICalendarParticipationStatus::Tentative,
            Self::Declined => ICalendarParticipationStatus::Declined,
        }
    }

    /// The subject prefix Outlook and Thunderbird put on a reply. In English
    /// whatever the UI language: it lands in the organizer's mailbox, not ours.
    fn subject_prefix(self) -> &'static str {
        match self {
            Self::Accepted => "Accepted",
            Self::Tentative => "Tentative",
            Self::Declined => "Declined",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Tentative => "tentatively accepted",
            Self::Declined => "declined",
        }
    }
}

pub(crate) fn parse_ical(ics: &str) -> Option<ICalendar> {
    match calcard::Parser::new(ics).entry() {
        calcard::Entry::ICalendar(ical) => Some(ical),
        _ => None,
    }
}

/// Ignore invitation descriptions, sequence numbers and attendee changes when
/// deciding whether an existing reminder's schedule has changed.
pub(crate) fn same_reminder_schedule(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Schedule {
        kind: ICalendarComponentType,
        entries: Vec<ICalendarEntry>,
        children: Vec<Schedule>,
    }
    fn component(calendar: &ICalendar, index: usize, depth: usize) -> Option<Schedule> {
        let source = calendar.components.get(index)?;
        if depth > 16
            || !matches!(
                source.component_type,
                ICalendarComponentType::VCalendar
                    | ICalendarComponentType::VEvent
                    | ICalendarComponentType::VTimezone
                    | ICalendarComponentType::Standard
                    | ICalendarComponentType::Daylight
            )
        {
            return None;
        }
        let mut entries: Vec<_> = source
            .entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.name,
                    ICalendarProperty::Uid
                        | ICalendarProperty::Dtstart
                        | ICalendarProperty::Rrule
                        | ICalendarProperty::Rdate
                        | ICalendarProperty::Exrule
                        | ICalendarProperty::Exdate
                        | ICalendarProperty::RecurrenceId
                        | ICalendarProperty::Status
                        | ICalendarProperty::Tzid
                        | ICalendarProperty::Tzoffsetfrom
                        | ICalendarProperty::Tzoffsetto
                        | ICalendarProperty::Tzuntil
                        | ICalendarProperty::TzidAliasOf
                )
            })
            .cloned()
            .collect();
        for entry in &mut entries {
            entry.params.sort();
            entry.values.sort();
        }
        entries.sort();
        let mut children: Vec<_> = source
            .component_ids
            .iter()
            .filter_map(|id| component(calendar, *id as usize, depth + 1))
            .collect();
        children.sort();
        Some(Schedule {
            kind: source.component_type.clone(),
            entries,
            children,
        })
    }
    let signature = |ics: &str| -> Option<Vec<Schedule>> {
        let calendar = parse_ical(ics)?;
        let mut roots: Vec<_> = calendar
            .components
            .iter()
            .enumerate()
            .filter(|(_, c)| c.component_type == ICalendarComponentType::VCalendar)
            .filter_map(|(index, _)| component(&calendar, index, 0))
            .collect();
        roots.sort();
        Some(roots)
    };
    matches!((signature(left), signature(right)), (Some(left), Some(right)) if left == right)
}

pub(crate) fn text_of(component: &ICalendarComponent, prop: ICalendarProperty) -> String {
    component
        .property(&prop)
        .and_then(|entry| entry.values.first())
        .and_then(ICalendarValue::as_text)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn cn_of(entry: &ICalendarEntry) -> String {
    entry
        .parameter(&ICalendarParameterName::Cn)
        .and_then(ICalendarParameterValue::as_text)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn addr_of(entry: &ICalendarEntry) -> String {
    entry
        .calendar_address()
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The device's IANA zone name, when the platform reports one.
pub(crate) fn local_tzid() -> Option<String> {
    iana_time_zone::get_timezone()
        .ok()
        .filter(|name| name.parse::<Tz>().is_ok())
}

/// The zone a floating time (no TZID, no `Z`) is read in: the device's own,
/// which is what the organizer meant by "10:00 wherever you are". By name when
/// the platform has one, so a repeating event keeps its time across DST.
pub(crate) fn local_tz() -> Tz {
    local_tzid()
        .and_then(|name| name.parse().ok())
        .unwrap_or_else(|| Tz::Fixed(*chrono::Local::now().offset()))
}

/// The VEVENT a card describes: the series master (or a lone event) when the
/// part has one, else the first occurrence override.
fn primary_event(ical: &ICalendar) -> Option<usize> {
    let events = || {
        ical.components
            .iter()
            .enumerate()
            .filter(|(_, component)| component.component_type == ICalendarComponentType::VEvent)
    };
    events()
        .find(|(_, component)| !component.is_recurrence_override())
        .or_else(|| events().next())
        .map(|(index, _)| index)
}

/// Read the event out of one calendar part. `None` when it is not one: no
/// VEVENT (a VTODO, a free/busy answer), or nothing parseable at all.
pub fn parse_invite(ics: &str) -> Option<Invite> {
    if ics.len() > MAX_ICS_BYTES {
        return None;
    }
    let ical = parse_ical(ics)?;
    let event = &ical.components[primary_event(&ical)?];
    let method = ical
        .components
        .iter()
        .find(|component| component.component_type == ICalendarComponentType::VCalendar)
        .map(|calendar| text_of(calendar, ICalendarProperty::Method))
        .unwrap_or_default()
        .to_ascii_lowercase();

    let resolver = ical.build_tz_resolver().with_default(local_tz());
    let instant = |entry: &ICalendarEntry| -> Option<(i64, bool)> {
        let value = entry.values.first()?.as_partial_date_time()?;
        let tz = resolver.resolve_or_default(entry.tz_id());
        let all_day = !value.has_time();
        let time = if all_day {
            value.to_date_time_with_tz(Tz::UTC)?
        } else {
            value.to_date_time_with_tz(tz)?
        };
        Some((time.timestamp(), all_day))
    };

    let (start, all_day) = event
        .property(&ICalendarProperty::Dtstart)
        .and_then(instant)?;
    let end = event
        .property(&ICalendarProperty::Dtend)
        .and_then(instant)
        .map(|(end, _)| end)
        .or_else(|| {
            event
                .property(&ICalendarProperty::Duration)
                .and_then(|entry| match entry.values.first() {
                    Some(ICalendarValue::Duration(duration)) => Some(start + duration.as_seconds()),
                    _ => None,
                })
        })
        // RFC 5545 3.6.1: a date-only DTSTART with no end lasts the day; a
        // timed one with no end is an instant.
        .unwrap_or(if all_day { start + 86_400 } else { start })
        .max(start);
    let recurrence_id = event
        .property(&ICalendarProperty::RecurrenceId)
        .and_then(instant)
        .map(|(at, _)| at)
        .unwrap_or(0);
    let this_and_future = event
        .property(&ICalendarProperty::RecurrenceId)
        .and_then(|entry| entry.parameter(&ICalendarParameterName::Range))
        .is_some_and(|range| matches!(range, ICalendarParameterValue::Bool(true)));

    let organizer = event
        .property(&ICalendarProperty::Organizer)
        .map(|entry| Person {
            name: cn_of(entry),
            addr: addr_of(entry),
        })
        .unwrap_or_default();
    let attendees = event
        .properties(&ICalendarProperty::Attendee)
        .map(|entry| Attendee {
            name: cn_of(entry),
            addr: addr_of(entry),
            partstat: entry
                .parameter(&ICalendarParameterName::Partstat)
                .and_then(ICalendarParameterValue::as_text)
                .unwrap_or("NEEDS-ACTION")
                .to_ascii_lowercase(),
        })
        .filter(|attendee| !attendee.addr.is_empty())
        .collect();

    Some(Invite {
        cancelled: method == "cancel" || event.status() == Some(&ICalendarStatus::Cancelled),
        method,
        uid: text_of(event, ICalendarProperty::Uid),
        sequence: event
            .property(&ICalendarProperty::Sequence)
            .and_then(|entry| entry.values.first())
            .and_then(ICalendarValue::as_integer)
            .unwrap_or(0),
        recurrence_id,
        this_and_future,
        summary: text_of(event, ICalendarProperty::Summary),
        location: text_of(event, ICalendarProperty::Location),
        description: text_of(event, ICalendarProperty::Description),
        start,
        end,
        all_day,
        recurring: event.is_recurrent(),
        organizer,
        attendees,
        ics: ics.to_string(),
    })
}

/// A message cached before invitations were read keeps its `.ics` as a file
/// under `media_root`. Read the event from there so the card shows without a
/// refetch; an invite that only came as an inline calendar alternative has no
/// file and stays plain until the message is downloaded again.
pub fn invite_from_attachments(
    media_root: &std::path::Path,
    attachments: &[crate::parse::Attachment],
) -> Option<Invite> {
    attachments
        .iter()
        .filter(|attachment| crate::parse::is_calendar_mime(&attachment.mime))
        .filter_map(|attachment| attachment.key.as_deref())
        .filter_map(|key| std::fs::read(media_root.join(key)).ok())
        .find_map(|bytes| parse_invite(&String::from_utf8_lossy(&bytes)))
}

fn is_mine(addr: &str, mine: &HashSet<String>) -> bool {
    mine.contains(&addr.trim().to_ascii_lowercase())
}

/// The attendee line that is the user, by any of the account's addresses.
pub fn my_attendee<'a>(invite: &'a Invite, mine: &HashSet<String>) -> Option<&'a Attendee> {
    invite
        .attendees
        .iter()
        .find(|attendee| is_mine(&attendee.addr, mine))
}

/// Whether Accept / Tentative / Decline mean anything here: an open request
/// from someone else, with an organizer to send the answer to. An event the
/// user organized themselves (a copy in Sent, an invite they forwarded) has
/// nobody to reply to.
pub fn can_respond(invite: &Invite, mine: &HashSet<String>) -> bool {
    invite.method == "request"
        && !invite.cancelled
        && !invite.uid.is_empty()
        && !invite.organizer.addr.is_empty()
        && !is_mine(&invite.organizer.addr, mine)
}

/// The answer the user gave, as far as this device knows. A response recorded
/// here wins while it is for this revision of the event; once the organizer
/// sends a newer SEQUENCE (moved it, say), it no longer stands, and the
/// organizer's own idea of the user's PARTSTAT is all there is.
pub fn my_response(
    invite: &Invite,
    mine: &HashSet<String>,
    recorded: Option<&crate::store::InviteResponse>,
) -> String {
    if let Some(recorded) = recorded
        && recorded.sequence >= invite.sequence
    {
        return recorded.response.clone();
    }
    my_attendee(invite, mine)
        .map(|attendee| attendee.partstat.as_str())
        .filter(|partstat| Response::parse(partstat).is_some())
        .unwrap_or_default()
        .to_string()
}

fn person_json(name: &str, addr: &str) -> Value {
    json!({ "name": name, "addr": addr })
}

/// The `invite` object on a thread message. The raw calendar text stays in the
/// core: clients only render.
pub fn invite_json(
    invite: &Invite,
    mine: &HashSet<String>,
    recorded: Option<&crate::store::InviteResponse>,
    event_id: Option<&str>,
) -> Value {
    json!({
        "method": if invite.method.is_empty() { "publish" } else { invite.method.as_str() },
        "uid": invite.uid,
        "summary": invite.summary,
        "location": invite.location,
        "description": invite.description,
        "start": invite.start,
        "end": invite.end,
        "all_day": invite.all_day,
        "recurring": invite.recurring,
        "cancelled": invite.cancelled,
        "organizer": person_json(&invite.organizer.name, &invite.organizer.addr),
        "attendees": invite
            .attendees
            .iter()
            .map(|attendee| json!({
                "name": attendee.name,
                "addr": attendee.addr,
                "partstat": attendee.partstat,
            }))
            .collect::<Vec<_>>(),
        "can_respond": can_respond(invite, mine),
        "response": my_response(invite, mine, recorded),
        // The calendar event this invitation is filed as, so a cancellation
        // can offer to take it out; empty when it is not in the calendar.
        "event_id": event_id.unwrap_or_default(),
        // Which occurrence this is about (Unix seconds); 0 for the event.
        "recurrence_id": invite.recurrence_id,
    })
}

/// [`invite_json`] for a message of `account`, with what the store knows of
/// the user's answer and of the event it filed.
pub fn invite_json_for(
    conn: &rusqlite::Connection,
    account: &str,
    invite: &Invite,
    mine: &HashSet<String>,
) -> Value {
    let recorded = crate::store::invite_response(conn, account, &invite.uid, invite.recurrence_id)
        .ok()
        .flatten();
    let event_id = filed_event_id(conn, account, invite);
    invite_json(
        invite,
        mine,
        recorded.as_ref(),
        (!event_id.is_empty()).then_some(event_id.as_str()),
    )
}

/// The calendar event `invite` is filed as, or empty: an occurrence filed
/// alone is its own; else the series, unless the occurrence it names is one
/// the calendar leaves out.
pub fn filed_event_id(conn: &rusqlite::Connection, account: &str, invite: &Invite) -> String {
    let alone = (invite.recurrence_id != 0)
        .then(|| {
            crate::store::invite_occurrence_event(conn, account, &invite.uid, invite.recurrence_id)
                .ok()
                .flatten()
        })
        .flatten();
    alone
        .or_else(|| {
            crate::store::invite_event(conn, account, &invite.uid)
                .ok()
                .flatten()
        })
        .filter(|event| {
            event.recurrence_id != 0
                || invite.recurrence_id == 0
                || !event.recurring
                || !events::excludes_occurrence(event, invite.recurrence_id)
        })
        .map(|event| event.id)
        .unwrap_or_default()
}

/// What a reply to an invitation is sent as: who it goes to, its subject and
/// readable text, and the calendar part that carries the answer.
pub struct ReplyMail {
    pub to: String,
    pub subject: String,
    pub body: String,
    pub ics: String,
}

/// What [`respond`] did: the account the answer went from, the message sent
/// (for the host to file in Sent), and the calendar event the invitation is
/// now filed as (empty when it is not, as after a decline).
pub struct Responded {
    pub account: String,
    pub raw: Vec<u8>,
    pub event_id: String,
}
