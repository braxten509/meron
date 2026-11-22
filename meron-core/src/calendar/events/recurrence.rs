//! Edits exceptions and occurrence bounds in a stored iCalendar series.
//! Calendar editing and invitation updates use the same override and exclusion rules.
//! Time-zone components and recurrence identities travel with the edited event.

use super::*;

/// `series_ics` with the occurrence `invite` is about answered: declined, it is excluded (EXDATE);
/// accepted, the organizer's version of it replaces any earlier one (an override VEVENT with its
/// RECURRENCE-ID), which the expansion then shows in place of the series' own. With no `response`,
/// an organizer's update the user has not answered, it takes the series' answer.
pub(super) fn with_occurrence(
    series_ics: &str,
    invite: &Invite,
    response: Option<Response>,
) -> Result<String> {
    let mut series = crate::calendar::parse_ical(series_ics)
        .ok_or_else(|| anyhow!("stored event is not iCalendar"))?;
    let update = crate::calendar::parse_ical(&invite.ics)
        .ok_or_else(|| anyhow!("invitation is not iCalendar"))?;
    let occurrence = update
        .components
        .iter()
        .find(|component| {
            component.component_type == ICalendarComponentType::VEvent
                && recurrence_instant(&update, component) == Some(invite.recurrence_id)
        })
        .ok_or_else(|| anyhow!("invitation names no occurrence"))?;

    // The update's times may name a zone only it defines.
    merge_timezones(&mut series, &update);
    // An earlier answer about the same occurrence gives way to this one.
    remove_overrides(&mut series, invite.recurrence_id);
    remove_exdate(&mut series, invite.recurrence_id);
    if response == Some(Response::Declined) {
        let rid = occurrence
            .property(&ICalendarProperty::RecurrenceId)
            .cloned()
            .ok_or_else(|| anyhow!("invitation names no occurrence"))?;
        let this_and_future = matches!(
            rid.parameter(&calcard::icalendar::ICalendarParameterName::Range),
            Some(calcard::icalendar::ICalendarParameterValue::Bool(true))
        );
        if this_and_future {
            end_before(&mut series, invite.recurrence_id)?;
        } else {
            add_exdate(&mut series, rid)?;
        }
    } else {
        // Accepted again from the date the series was cut at: it runs on.
        if invite.this_and_future
            && ends_before(&series).is_some_and(|cut| cut >= invite.recurrence_id)
        {
            lift_end(&mut series);
        }
        put_override(&mut series, occurrence.clone(), response);
    }
    Ok(series.to_string())
}

/// `series_ics` leaving out the occurrence at `recurrence_id`, unless it
/// already does or has an answer of its own (an override).
pub(super) fn without_occurrence(
    series_ics: &str,
    recurrence_id: i64,
    all_day: bool,
) -> Result<String> {
    let mut series = crate::calendar::parse_ical(series_ics)
        .ok_or_else(|| anyhow!("stored event is not iCalendar"))?;
    let overridden = overrides(&series)
        .any(|component| recurrence_instant(&series, component) == Some(recurrence_id));
    if overridden || exdates(&series).contains(&recurrence_id) {
        return Ok(series_ics.to_string());
    }
    let date = if all_day {
        PartialDateTime::from_date_timestamp(recurrence_id)
    } else {
        PartialDateTime::from_utc_timestamp(recurrence_id)
    };
    add_exdate(
        &mut series,
        calcard::icalendar::ICalendarEntry {
            name: ICalendarProperty::Exdate,
            params: vec![],
            values: vec![ICalendarValue::PartialDateTime(Box::new(date))],
        },
    )?;
    Ok(series.to_string())
}

/// An organizer's series calendar with each override at the series' own
/// SEQUENCE, which calcard needs to pair them (see [`put_override`]).
pub(super) fn with_series_sequence(ics: &str) -> Result<String> {
    let Some(mut series) = crate::calendar::parse_ical(ics) else {
        return Ok(ics.to_string());
    };
    let Some(master) = master_index(&series) else {
        return Ok(ics.to_string());
    };
    let sequence = series.components[master]
        .property(&ICalendarProperty::Sequence)
        .cloned();
    let mut changed = false;
    for component in series.components.iter_mut().filter(|component| {
        component.component_type == ICalendarComponentType::VEvent
            && component.is_recurrence_override()
    }) {
        if component.property(&ICalendarProperty::Sequence) != sequence.as_ref() {
            keep_own_sequence(component);
            component
                .entries
                .retain(|entry| entry.name != ICalendarProperty::Sequence);
            component.entries.extend(sequence.clone());
            changed = true;
        }
    }
    Ok(if changed {
        series.to_string()
    } else {
        ics.to_string()
    })
}

/// A newly answered `incoming` series, with the exceptions the user already had on the `stored` one
/// carried over: dates declined or cancelled away, and occurrences answered on their own. Re-filing
/// the series must not bring back a meeting the user took out, nor undo a moved one they accepted.
/// What the organizer's new calendar says about a date itself wins.
pub(super) fn carry_exceptions(stored: &str, incoming: &str) -> Result<String> {
    let mut series = crate::calendar::parse_ical(incoming)
        .ok_or_else(|| anyhow!("invitation is not iCalendar"))?;
    let Some(old) = crate::calendar::parse_ical(stored) else {
        return Ok(incoming.to_string());
    };
    // What is carried over may name a zone only the stored calendar defines.
    merge_timezones(&mut series, &old);
    // A series cut short stays cut: the organizer's own occurrences past the
    // cut go, while the user's own answers carried below stay.
    if let Some(cut) = ends_before(&old) {
        end_before(&mut series, cut)?;
    }
    // An occurrence the user already has at a newer revision than the one
    // this calendar embeds (a late copy of the series) keeps the newer.
    let stored_revision = |rid: i64| {
        overrides(&old)
            .filter(|component| recurrence_instant(&old, component) == Some(rid))
            .map(own_sequence)
            .max()
    };
    let older: Vec<i64> = overrides(&series)
        .filter_map(|component| {
            let rid = recurrence_instant(&series, component)?;
            stored_revision(rid)
                .is_some_and(|stored| stored > own_sequence(component))
                .then_some(rid)
        })
        .collect();
    for rid in &older {
        remove_overrides(&mut series, *rid);
    }
    let overridden: Vec<i64> = overrides(&series)
        .filter_map(|component| recurrence_instant(&series, component))
        .collect();
    // The organizer's own exclusions count first: an override is shown
    // whatever the EXDATEs say, so carrying one over would undo them.
    let excluded = exdates(&series);
    for component in overrides(&old) {
        if recurrence_instant(&old, component)
            .is_some_and(|rid| !overridden.contains(&rid) && !excluded.contains(&rid))
        {
            put_override(&mut series, component.clone(), None);
        }
    }
    if let Some(master) = master_index(&old) {
        for entry in old.components[master].properties(&ICalendarProperty::Exdate) {
            for value in &entry.values {
                let Some(at) = value_instant(&old, entry, value) else {
                    continue;
                };
                if !excluded.contains(&at) && !overridden.contains(&at) {
                    add_exdate(
                        &mut series,
                        calcard::icalendar::ICalendarEntry {
                            values: vec![value.clone()],
                            ..entry.clone()
                        },
                    )?;
                }
            }
        }
    }
    Ok(series.to_string())
}

/// Keep a stored invitation's `start_at` at or before its earliest occurrence.
/// A range query takes a repeating row only once it has started, and an
/// organizer can move one occurrence before the series' first.
pub(super) fn settle_start(event: &mut CalendarEvent) {
    if !event.recurring {
        return;
    }
    let Some(ical) = crate::calendar::parse_ical(&event.ics) else {
        return;
    };
    let earliest = ical
        .components
        .iter()
        .filter(|component| component.component_type == ICalendarComponentType::VEvent)
        .filter_map(|component| {
            let entry = component.property(&ICalendarProperty::Dtstart)?;
            value_instant(&ical, entry, entry.values.first()?)
        })
        .min();
    if let Some(earliest) = earliest {
        event.start_at = event.start_at.min(earliest);
    }
}

/// Copy into `series` the VTIMEZONEs `other` defines and it does not, so an
/// override or EXDATE taken from `other` keeps the zone its TZID names.
fn merge_timezones(
    series: &mut calcard::icalendar::ICalendar,
    other: &calcard::icalendar::ICalendar,
) {
    let tzid = |component: &calcard::icalendar::ICalendarComponent| {
        crate::calendar::text_of(component, ICalendarProperty::Tzid)
    };
    let known: Vec<String> = series
        .components
        .iter()
        .filter(|component| component.component_type == ICalendarComponentType::VTimezone)
        .map(tzid)
        .collect();
    let Some(calendar) = series
        .components
        .iter()
        .position(|component| component.component_type == ICalendarComponentType::VCalendar)
    else {
        return;
    };
    for zone in other.components.iter().filter(|component| {
        component.component_type == ICalendarComponentType::VTimezone
            && !known.contains(&tzid(component))
    }) {
        let id = series.components.len() as u32;
        series
            .components
            .push(calcard::icalendar::ICalendarComponent {
                component_ids: vec![],
                ..zone.clone()
            });
        series.components[calendar].component_ids.push(id);
        // Its STANDARD and DAYLIGHT rules.
        for child in &zone.component_ids {
            if let Some(rule) = other.components.get(*child as usize) {
                let child_id = series.components.len() as u32;
                series.components.push(rule.clone());
                series.components[id as usize].component_ids.push(child_id);
            }
        }
    }
}

/// The series' own VEVENT: the one that is not an occurrence override.
pub(super) fn master_index(ical: &calcard::icalendar::ICalendar) -> Option<usize> {
    ical.components.iter().position(|component| {
        component.component_type == ICalendarComponentType::VEvent
            && !component.is_recurrence_override()
    })
}

pub(super) fn overrides(
    ical: &calcard::icalendar::ICalendar,
) -> impl Iterator<Item = &calcard::icalendar::ICalendarComponent> {
    ical.components.iter().filter(|component| {
        component.component_type == ICalendarComponentType::VEvent
            && component.is_recurrence_override()
    })
}

pub(super) fn remove_overrides(series: &mut calcard::icalendar::ICalendar, recurrence_id: i64) {
    let stale: Vec<u32> = series
        .components
        .iter()
        .enumerate()
        .filter(|(_, component)| {
            component.component_type == ICalendarComponentType::VEvent
                && component.is_recurrence_override()
                && recurrence_instant(series, component) == Some(recurrence_id)
        })
        .map(|(index, _)| index as u32)
        .collect();
    series.remove_component_ids(&stale);
}

/// Add an occurrence's own VEVENT to `series`. calcard pairs an override with its series by
/// RECURRENCE-ID and SEQUENCE together, and an organizer who moves one occurrence bumps the
/// SEQUENCE of that occurrence alone, so it takes the series' own — or the moved meeting would show
/// beside the one it replaces. `response`, when given, is the user's answer about it.
fn put_override(
    series: &mut calcard::icalendar::ICalendar,
    mut component: calcard::icalendar::ICalendarComponent,
    response: Option<Response>,
) {
    // Alarms and the like hang off the update's own numbering.
    component.component_ids.clear();
    let master_sequence = master_index(series)
        .and_then(|master| series.components[master].property(&ICalendarProperty::Sequence))
        .cloned();
    keep_own_sequence(&mut component);
    let private = ICalendarProperty::Other(OCCURRENCE_RESPONSE.to_string());
    component.entries.retain(|entry| {
        entry.name != ICalendarProperty::Sequence && (response.is_none() || entry.name != private)
    });
    component.entries.extend(master_sequence);
    if let Some(response) = response {
        component.add_property(private, ICalendarValue::Text(response.as_str().to_string()));
    }
    let id = series.components.len() as u32;
    series.components.push(component);
    if let Some(calendar) = series
        .components
        .iter_mut()
        .find(|component| component.component_type == ICalendarComponentType::VCalendar)
    {
        calendar.component_ids.push(id);
    }
}

/// Exclude a date from the series: `entry` names it (a RECURRENCE-ID or an
/// EXDATE, whichever it came as), and is filed as an EXDATE.
fn add_exdate(
    series: &mut calcard::icalendar::ICalendar,
    entry: calcard::icalendar::ICalendarEntry,
) -> Result<()> {
    let master = master_index(series).ok_or_else(|| anyhow!("stored event has no series"))?;
    series.components[master]
        .entries
        .push(calcard::icalendar::ICalendarEntry {
            name: ICalendarProperty::Exdate,
            ..entry
        });
    Ok(())
}

/// Cut `series` short before `at`: nothing from then on is shown, its
/// overrides included.
fn end_before(series: &mut calcard::icalendar::ICalendar, at: i64) -> Result<()> {
    let cut = ends_before(series).map_or(at, |earlier| earlier.min(at));
    drop_overrides_from(series, cut);
    set_end(series, cut)
}

/// Record that `series` ends before `at`, leaving its overrides: any past the
/// cut were accepted on their own afterwards, and stay (see `occurrences`).
fn set_end(series: &mut calcard::icalendar::ICalendar, at: i64) -> Result<()> {
    let cut = ends_before(series).map_or(at, |earlier| earlier.min(at));
    let master = master_index(series).ok_or_else(|| anyhow!("stored event has no series"))?;
    let name = ICalendarProperty::Other(ENDS_BEFORE.to_string());
    let component = &mut series.components[master];
    component.entries.retain(|entry| entry.name != name);
    component.add_property(name, ICalendarValue::Text(cut.to_string()));
    Ok(())
}

fn drop_overrides_from(series: &mut calcard::icalendar::ICalendar, cut: i64) {
    let later: Vec<u32> = series
        .components
        .iter()
        .enumerate()
        .filter(|(_, component)| {
            component.component_type == ICalendarComponentType::VEvent
                && component.is_recurrence_override()
                && recurrence_instant(series, component).is_some_and(|rid| rid >= cut)
        })
        .map(|(index, _)| index as u32)
        .collect();
    series.remove_component_ids(&later);
}

/// [`set_end`] on stored calendar text, for a decline replayed when its series
/// is filed: overrides there by then were accepted on their own afterwards.
pub(super) fn cut_from(series_ics: &str, at: i64) -> Result<String> {
    let mut series = crate::calendar::parse_ical(series_ics)
        .ok_or_else(|| anyhow!("stored event is not iCalendar"))?;
    set_end(&mut series, at)?;
    Ok(series.to_string())
}

/// Undo [`end_before`].
fn lift_end(series: &mut calcard::icalendar::ICalendar) {
    if let Some(master) = master_index(series) {
        let name = ICalendarProperty::Other(ENDS_BEFORE.to_string());
        series.components[master]
            .entries
            .retain(|entry| entry.name != name);
    }
}

/// The time a series was cut short before, if it was.
pub(super) fn ends_before(ical: &calcard::icalendar::ICalendar) -> Option<i64> {
    let master = master_index(ical)?;
    crate::calendar::text_of(
        &ical.components[master],
        ICalendarProperty::Other(ENDS_BEFORE.to_string()),
    )
    .parse()
    .ok()
}

/// Bring a date back into the series. An EXDATE may list several dates;
/// only the one is dropped, and the property only when nothing is left of it.
fn remove_exdate(series: &mut calcard::icalendar::ICalendar, recurrence_id: i64) {
    let Some(master) = master_index(series) else {
        return;
    };
    let kept: Vec<Option<Vec<ICalendarValue>>> = series.components[master]
        .entries
        .iter()
        .map(|entry| {
            (entry.name == ICalendarProperty::Exdate).then(|| {
                entry
                    .values
                    .iter()
                    .filter(|value| value_instant(series, entry, value) != Some(recurrence_id))
                    .cloned()
                    .collect()
            })
        })
        .collect();
    let mut kept = kept.into_iter();
    series.components[master]
        .entries
        .retain_mut(|entry| match kept.next().flatten() {
            Some(values) => {
                entry.values = values;
                !entry.values.is_empty()
            }
            None => true,
        });
}

/// Every date the series' own VEVENT excludes.
pub(super) fn exdates(ical: &calcard::icalendar::ICalendar) -> Vec<i64> {
    master_index(ical)
        .map(|master| {
            ical.components[master]
                .properties(&ICalendarProperty::Exdate)
                .flat_map(|entry| {
                    entry
                        .values
                        .iter()
                        .filter_map(|value| value_instant(ical, entry, value))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The instant a component's RECURRENCE-ID names, read the way
/// [`crate::calendar::parse_invite`] reads it.
pub(super) fn recurrence_instant(
    ical: &calcard::icalendar::ICalendar,
    component: &calcard::icalendar::ICalendarComponent,
) -> Option<i64> {
    let entry = component.property(&ICalendarProperty::RecurrenceId)?;
    value_instant(ical, entry, entry.values.first()?)
}

/// The instant one value of a date or date-time property names: in its
/// TZID's zone (or the device's, when floating), a bare date at its UTC
/// midnight.
pub(super) fn value_instant(
    ical: &calcard::icalendar::ICalendar,
    entry: &calcard::icalendar::ICalendarEntry,
    value: &ICalendarValue,
) -> Option<i64> {
    let value = value.as_partial_date_time()?;
    let tz = if value.has_time() {
        ical.build_tz_resolver()
            .with_default(crate::calendar::local_tz())
            .resolve_or_default(entry.tz_id())
    } else {
        Tz::UTC
    };
    Some(value.to_date_time_with_tz(tz)?.timestamp())
}

/// Whether `event` (an invitation's series) already leaves out the occurrence
/// starting at `recurrence_id`.
pub(crate) fn excludes_occurrence(event: &CalendarEvent, recurrence_id: i64) -> bool {
    crate::calendar::parse_ical(&event.ics).is_some_and(|ical| {
        let accepted_since = overrides(&ical)
            .any(|component| recurrence_instant(&ical, component) == Some(recurrence_id));
        exdates(&ical).contains(&recurrence_id)
            || (!accepted_since && ends_before(&ical).is_some_and(|cut| recurrence_id >= cut))
    })
}
