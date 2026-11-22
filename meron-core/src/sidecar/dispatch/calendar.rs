//! Routes calendar RPCs received from the desktop app.
//! Event reads and writes share the same local calendar implementation as mobile.
//! The result goes back to the calendar or invitation view that issued the request.

use serde_json::{Value, json};
use std::sync::Arc;

use meron_core::calendar::events;
use meron_core::engine::Engine;
use meron_core::protocol::Request;

use crate::sidecar::params::*;
use crate::{Writer, emit};

/// Handle the local calendar: `calendar.*`. Local SQLite only, like Tasks.
pub(crate) async fn dispatch(
    engine: &Arc<Engine>,
    request: &Request,
    out: &Writer,
) -> anyhow::Result<Value> {
    let parameters = &request.params;
    let result = match request.method.as_str() {
        "calendar.events" => {
            let from = parameters.get("from").and_then(Value::as_i64).unwrap_or(0);
            let to = parameters.get("to").and_then(Value::as_i64).unwrap_or(0);
            let include_tasks = parameters
                .get("include_tasks")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            return events::events_payload(&engine.db.lock().unwrap(), from, to, include_tasks);
        }

        "calendar.get" => {
            let event_id = req_str(parameters, "event_id")?;
            return events::event_payload(&engine.db.lock().unwrap(), &event_id);
        }

        "calendar.create" => events::create_event(
            &engine.db.lock().unwrap(),
            &events::EventInput::from_params(parameters),
        )?,

        "calendar.update" => {
            let event_id = req_str(parameters, "event_id")?;
            events::update_event(
                &engine.db.lock().unwrap(),
                &event_id,
                &events::EventInput::from_params(parameters),
            )?
        }

        "calendar.delete" => {
            let event_id = req_str(parameters, "event_id")?;
            events::delete_event(&engine.db.lock().unwrap(), &event_id)?
        }

        "invite.addToCalendar" => {
            let message = req_str(parameters, "message")?;
            let thread_id = req_str(parameters, "thread_id").unwrap_or_default();
            let event_id = meron_core::calendar::add_to_calendar(
                &engine.db.lock().unwrap(),
                &message,
                &thread_id,
                &meron_core::parse::media_root(),
            )?;
            json!({ "ok": true, "event_id": event_id })
        }

        "invite.removeFromCalendar" => {
            let message = req_str(parameters, "message")?;
            meron_core::calendar::remove_from_calendar(
                &engine.db.lock().unwrap(),
                &message,
                &meron_core::parse::media_root(),
            )?;
            json!({ "ok": true })
        }

        "calendar.restore" => {
            let restore = parameters.get("restore").cloned().unwrap_or(Value::Null);
            events::restore_event(&engine.db.lock().unwrap(), &restore)?
        }

        other => return Err(anyhow::anyhow!("unknown method: {other}")),
    };

    // Only mutations reach here; any surface showing the calendar re-reads.
    emit(out, "calendar.changed", json!({ "method": request.method })).await;
    Ok(result)
}
