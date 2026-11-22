//! Routes mobile calendar commands to local event storage.
//! The host selects a command and supplies its event or date-range parameters.
//! The protocol returns the event payload expected by the calendar screen.

use super::*;

use crate::calendar::events;

/// Route one `calendar.*` request. Mutations emit `calendar.changed` so a
/// screen that isn't the one showing the calendar re-reads.
pub(crate) fn dispatch_mobile_calendar(
    data_directory: &str,
    method: &str,
    params: &Value,
) -> Option<Result<Value, String>> {
    let string_error = |result: anyhow::Result<Value>| result.map_err(|error| format!("{error:#}"));
    let result = match method {
        "calendar.events" => {
            let from = params.get("from").and_then(Value::as_i64).unwrap_or(0);
            let to = params.get("to").and_then(Value::as_i64).unwrap_or(0);
            let include_tasks = params
                .get("include_tasks")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            return Some(with_mobile_db(data_directory, |conn| {
                string_error(events::events_payload(&conn, from, to, include_tasks))
            }));
        }
        "calendar.get" => {
            let event_id = match required_string(params, "event_id") {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            return Some(with_mobile_db(data_directory, |conn| {
                string_error(events::event_payload(&conn, &event_id))
            }));
        }
        "calendar.create" => {
            let input = events::EventInput::from_params(params);
            with_mobile_db(data_directory, |conn| {
                string_error(events::create_event(&conn, &input))
            })
        }
        "calendar.update" => {
            let event_id = match required_string(params, "event_id") {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            let input = events::EventInput::from_params(params);
            with_mobile_db(data_directory, |conn| {
                string_error(events::update_event(&conn, &event_id, &input))
            })
        }
        "calendar.delete" => {
            let event_id = match required_string(params, "event_id") {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            with_mobile_db(data_directory, |conn| {
                string_error(events::delete_event(&conn, &event_id))
            })
        }
        "invite.addToCalendar" => {
            let message = match required_string(params, "message") {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            let thread_id = optional_string(params, "thread_id");
            let media_root = std::path::PathBuf::from(data_directory).join("attachments");
            with_mobile_db(data_directory, |conn| {
                string_error(
                    crate::calendar::add_to_calendar(&conn, &message, &thread_id, &media_root)
                        .map(|event_id| json!({ "ok": true, "event_id": event_id })),
                )
            })
        }
        "invite.removeFromCalendar" => {
            let message = match required_string(params, "message") {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            let media_root = std::path::PathBuf::from(data_directory).join("attachments");
            with_mobile_db(data_directory, |conn| {
                string_error(
                    crate::calendar::remove_from_calendar(&conn, &message, &media_root)
                        .map(|()| json!({ "ok": true })),
                )
            })
        }
        "calendar.restore" => {
            let restore = params.get("restore").cloned().unwrap_or(Value::Null);
            with_mobile_db(data_directory, |conn| {
                string_error(events::restore_event(&conn, &restore))
            })
        }
        _ => return None,
    };
    if result.is_ok() {
        crate::ffi::emit_event("calendar.changed", json!({ "method": method }));
    }
    Some(result)
}

/// Empty answers for every `calendar.*` method, for the stub dispatcher that
/// runs before a data dir is bound.
pub(crate) fn stub_mobile_calendar(method: &str) -> Option<Value> {
    Some(match method {
        "calendar.events" => json!({ "events": [] }),
        "calendar.get" | "calendar.create" | "calendar.update" | "calendar.restore" => {
            json!({ "event": Value::Null })
        }
        "calendar.delete" => json!({ "ok": true, "restore": Value::Null }),
        "invite.removeFromCalendar" | "invite.addToCalendar" => json!({ "ok": true }),
        _ => return None,
    })
}
