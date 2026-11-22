//! Mobile mail protocol handlers for rules.
//!
//! The mobile dispatcher calls these handlers with protocol parameters.
//! They translate between the wire format and the shared mail engine.

use super::*;

pub(crate) fn mobile_rules(data_directory: &str) -> Result<Value, String> {
    with_mobile_db(data_directory, |conn| {
        let rules = store::rules(&conn).map_err(|error| error.to_string())?;
        Ok(json!({ "rules": rules }))
    })
}

pub(crate) fn set_mobile_rules(data_directory: &str, params: &Value) -> Result<Value, String> {
    let rules = serde_json::from_value(params.get("rules").cloned().unwrap_or_default())
        .map_err(|error| error.to_string())?;
    with_mobile_db(data_directory, |conn| {
        let rules = store::set_rules(&conn, rules).map_err(|error| error.to_string())?;
        Ok(json!({ "rules": rules }))
    })
}

pub(crate) fn mobile_rule_matches(data_directory: &str, params: &Value) -> Result<Value, String> {
    let id = required_string(params, "id")?;
    with_mobile_db(data_directory, |conn| {
        let count =
            store::count_existing_rule_matches(&conn, &id).map_err(|error| error.to_string())?;
        Ok(json!({ "count": count }))
    })
}

pub(crate) fn mobile_rule_status(data_directory: &str) -> Result<Value, String> {
    with_mobile_db(data_directory, |conn| {
        let log = store::rule_log(&conn).map_err(|error| error.to_string())?;
        let running = store::running_rules(&conn).map_err(|error| error.to_string())?;
        Ok(json!({ "log": log, "running": running }))
    })
}

pub(crate) fn stop_mobile_rule(data_directory: &str, params: &Value) -> Result<Value, String> {
    let id = required_string(params, "id")?;
    with_mobile_db(data_directory, |conn| {
        store::stop_rule(&conn, &id).map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

/// The phone's AI sorting commands, through the engine (it holds the keys and
/// the verified Tinfoil client).
pub(crate) fn mobile_ai_sort(
    data_directory: &str,
    command: &str,
    params: &Value,
) -> Result<Value, String> {
    let engine = crate::ffi::engine_for(data_directory)?;
    crate::ffi::engine_block_on(crate::ai_sort::dispatch(&engine, command, params))
}

/// "Copy setup to another device" on the phone, through the engine (it holds
/// the keys); its network work goes on in the engine's runtime.
pub(crate) fn mobile_setup_copy(
    data_directory: &str,
    command: &str,
    params: &Value,
) -> Result<Value, String> {
    let engine = crate::ffi::engine_for(data_directory)?;
    crate::ffi::engine_block_on(crate::setup_copy::dispatch(
        &engine, command, params, "phone",
    ))
}

/// The Meron folder commands on the phone: store only, except Mark all read,
/// which needs the engine for the servers.
pub(crate) fn mobile_meron_folders(
    data_directory: &str,
    command: &str,
    params: &Value,
) -> Result<Value, String> {
    if command == "markAllRead" {
        let engine = crate::ffi::engine_for(data_directory)?;
        return crate::ffi::engine_block_on(crate::meron_folders::mark_all_read(&engine, params));
    }
    // Out of the built-in Archive (and the Undo of that) moves mail on the
    // server.
    if matches!(command, "moveIn" | "moveOut") {
        let engine = crate::ffi::engine_for(data_directory)?;
        return crate::ffi::engine_block_on(crate::meron_folders::move_with_server(
            &engine, command, params,
        ));
    }
    let mut jobs = Vec::new();
    let result = with_mobile_db(data_directory, |conn| {
        let (result, wanted) = crate::meron_folders::dispatch_with_marks(&conn, command, params)
            .map_err(|error| format!("{error:#}"))?;
        jobs = wanted;
        Ok(result)
    })?;
    // Filing changed: the marks other devices follow, in the background.
    if !jobs.is_empty()
        && let Ok(engine) = crate::ffi::engine_for(data_directory)
    {
        crate::meron_marks::spawn_writes(&engine, jobs);
    }
    Ok(result)
}

pub(crate) fn run_mobile_rule(data_directory: &str, params: &Value) -> Result<Value, String> {
    let id = required_string(params, "id")?;
    let engine = crate::ffi::engine_for(data_directory)?;
    let count = crate::ffi::engine_block_on(crate::engine::run_rule_on_existing(&engine, &id))?;
    Ok(json!({ "count": count }))
}
