//! Mobile mail protocol handlers for tags.
//!
//! The mobile dispatcher calls these handlers with protocol parameters.
//! They translate between the wire format and the shared mail engine.

use super::*;

pub(crate) fn mark_mobile_thread_starred(data_dir: &str, params: &Value) -> Result<Value, String> {
    let thread_id = req_str(params, "thread_id")?;
    let starred = req_bool(params, "starred")?;
    if is_rss_thread_id(&thread_id) {
        return mark_mobile_rss_thread_starred(data_dir, params);
    }
    let parsed = parse_thread_id_with_request_folder(&thread_id, params)
        .ok_or_else(|| "invalid thread_id".to_string())?;
    let engine = crate::ffi::engine_for(data_dir)?;
    with_mobile_db(data_dir, |conn| {
        let uids = requested_mobile_uids(&conn, &parsed, params)?;
        let key = parsed
            .subject_filter
            .as_deref()
            .map(|subject| store::branch_compound_key(&parsed.thread_key, subject))
            .unwrap_or_else(|| parsed.thread_key.clone());
        let message_scoped = has_requested_mobile_message_ids(params) || parsed.uid.is_some();
        let targets = store::starred_mutation_targets(
            &conn,
            &parsed.account,
            &parsed.folder,
            (!message_scoped).then_some(key.as_str()),
            &uids,
            starred,
        )
        .map_err(|err| err.to_string())?;
        if !targets.is_empty() {
            let creds = load_mobile_account_creds(&conn, &parsed.account)?;
            if account_needs_reconnect(&creds) {
                return Err(format!("account needs reconnect: {}", parsed.account));
            }
            crate::ffi::engine_block_on(crate::engine::mark_starred_copies(
                &engine,
                &parsed.account,
                &targets,
                starred,
            ))?;
        }
        crate::mail_model::mutation_result(
            json!({ "ok": true }),
            &conn,
            &parsed.account,
            &thread_id,
            &parsed.folder,
            None,
            None,
            Some(starred),
            false,
        )
        .map_err(|err| err.to_string())
    })
}

/// Save a tag change locally, then let the server catch up on its own: a tag
/// needs no connection, and a failed push is retried by the next sync.
pub(crate) fn set_mobile_thread_tags(data_dir: &str, params: &Value) -> Result<Value, String> {
    let thread_id = req_str(params, "thread_id")?;
    let parsed = parse_thread_id_with_request_folder(&thread_id, params)
        .ok_or_else(|| "invalid thread_id".to_string())?;
    let add = crate::tags::parse_tag_list(params.get("add"))?;
    let remove = crate::tags::parse_tag_list(params.get("remove"))?;
    let key = parsed
        .subject_filter
        .as_deref()
        .map(|subject| store::branch_compound_key(&parsed.thread_key, subject))
        .unwrap_or_else(|| parsed.thread_key.clone());
    let result = with_mobile_db(data_dir, |conn| {
        let uids = if has_requested_mobile_message_ids(params) || parsed.uid.is_some() {
            requested_mobile_uids(&conn, &parsed, params)?
        } else {
            Vec::new()
        };
        let targets = store::tag_targets(&conn, &parsed.account, &parsed.folder, &key, &uids)
            .map_err(|err| err.to_string())?;
        let tags = store::retag(&conn, &parsed.account, &targets, &add, &remove, true)
            .map_err(|err| err.to_string())?;
        Ok(json!({ "ok": true, "tags": tags }))
    })?;
    crate::ffi::spawn_tag_push(data_dir, &parsed.account);
    Ok(result)
}

pub(crate) fn list_mobile_tags(data_dir: &str) -> Result<Value, String> {
    with_mobile_db(data_dir, |conn| {
        store::tag_counts(&conn, None)
            .map(store::tag_counts_json)
            .map_err(|err| err.to_string())
    })
}

/// Rename a tag everywhere, or delete it, then let the server catch up on its
/// own, as a tag change on one thread does.
pub(crate) fn replace_mobile_tag(data_dir: &str, params: &Value) -> Result<Value, String> {
    let (tag, to) = crate::tags::parse_tag_replacement(params)?;
    let mut accounts = Vec::new();
    let result = with_mobile_db(data_dir, |conn| {
        let (changed, rules) = store::replace_tag_and_rules(&conn, &tag, to.as_deref())
            .map_err(|err| err.to_string())?;
        accounts = changed;
        let queries = crate::tags::renamed_queries(params, &tag, to.as_deref());
        Ok(json!({ "ok": true, "tag": to, "rules": rules, "queries": queries }))
    })?;
    for account in &accounts {
        crate::ffi::spawn_tag_push(data_dir, account);
    }
    Ok(result)
}
