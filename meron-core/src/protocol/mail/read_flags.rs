//! Mobile mail protocol handlers for read flags.
//!
//! The mobile dispatcher calls these handlers with protocol parameters.
//! They translate between the wire format and the shared mail engine.

use super::*;

pub(crate) fn mark_mobile_thread_read(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let thread_id = required_string(params, "thread_id")?;
    let seen = required_boolean(params, "seen")?;
    if is_rss_thread_id(&thread_id) {
        return mark_mobile_rss_thread_read(data_directory, params);
    }
    let parsed = parse_thread_id_with_request_folder(&thread_id, params)
        .ok_or_else(|| "invalid thread_id".to_string())?;
    let engine = crate::ffi::engine_for(data_directory)?;
    with_mobile_db(data_directory, |conn| {
        let explicit_ids = has_requested_mobile_message_ids(params);
        // Marking a whole thread read reaches every folder it spans and only the
        // messages still unread; everything else acts on the requested folder.
        let whole_thread_read = seen && !explicit_ids && parsed.uid.is_none();
        let by_folder = if whole_thread_read {
            store::thread_uids_by_folder(
                &conn,
                &parsed.account,
                &parsed.folder,
                &parsed.thread_key,
                parsed.subject_filter.as_deref(),
                true,
            )
            .map_err(|error| error.to_string())?
        } else {
            let uids = if !seen && !explicit_ids && parsed.uid.is_none() {
                // Marking a whole thread unread flags its newest message only.
                store::newest_thread_uids(
                    &conn,
                    &parsed.account,
                    &parsed.folder,
                    &parsed.thread_key,
                    parsed.subject_filter.as_deref(),
                )
                .map_err(|error| error.to_string())?
            } else {
                requested_mobile_uids(&conn, &parsed, params)?
            };
            std::collections::BTreeMap::from([(parsed.folder.clone(), uids)])
        };
        // The server first, the store once it took the change — as markStarred
        // and markAllRead do. A store marked ahead of a failed write keeps the
        // messages read on the next reload although the server still has them
        // unread, and leaves a retry nothing to send.
        if by_folder.values().any(|uids| !uids.is_empty()) {
            let creds = load_mobile_account_creds(&conn, &parsed.account)?;
            if account_needs_reconnect(&creds) {
                return Err(format!("account needs reconnect: {}", parsed.account));
            }
            crate::ffi::engine_block_on(engine.store_seen_by_folder(
                &parsed.account,
                &by_folder,
                seen,
            ))?;
        }
        for (folder, uids) in by_folder
            .iter()
            .filter(|(folder, _)| **folder != parsed.folder)
        {
            for uid in uids {
                store::update_message_seen(&conn, &parsed.account, folder, *uid, seen)
                    .map_err(|error| error.to_string())?;
            }
        }
        let own_uids = by_folder
            .get(&parsed.folder)
            .map(Vec::as_slice)
            .unwrap_or_default();
        update_mobile_read_state(&conn, &parsed, params, own_uids, seen)?;
        let mut result = crate::mail_model::mutation_result(
            json!({ "ok": true }),
            &conn,
            &parsed.account,
            &thread_id,
            &parsed.folder,
            None,
            Some(!seen),
            None,
            false,
        )
        .map_err(|error| error.to_string())?;
        crate::mail_model::add_folder_unreads(
            &mut result,
            &conn,
            &parsed.account,
            by_folder.keys(),
        )
        .map_err(|error| error.to_string())?;
        Ok(result)
    })
}

pub(crate) fn mark_mobile_folder_all_read(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let account_id = required_account_id(params)?;
    let folder = params
        .get("folder_id")
        .or_else(|| params.get("folder"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(canon_folder)
        .unwrap_or_else(|| "INBOX".to_string());

    if account_id == "unified" {
        let accounts = with_mobile_db(data_directory, |conn| {
            let accounts = store::list_accounts(&conn).map_err(|error| error.to_string())?;
            Ok(json!(
                accounts
                    .into_iter()
                    .filter(|account| account
                        .get("included_in_unified")
                        .and_then(Value::as_bool)
                        .unwrap_or(true))
                    .filter(
                        |account| account.get("auth_type").and_then(Value::as_str) != Some("rss")
                    )
                    .filter_map(|account| account
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_string))
                    .collect::<Vec<_>>()
            ))
        })?
        .as_array()
        .cloned()
        .unwrap_or_default();
        let accounts = accounts
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        let account_folders = with_mobile_db(data_directory, |conn| {
            let mut resolved = Vec::new();
            for account in accounts {
                if let Some(account_folder) = store::folder_for_role(&conn, &account, &folder)
                    .map_err(|error| error.to_string())?
                {
                    resolved.push((account, account_folder));
                }
            }
            Ok(json!(resolved))
        })?;
        let account_folders = serde_json::from_value::<Vec<(String, String)>>(account_folders)
            .map_err(|error| format!("{error:#}"))?;
        let mut updated = 0_u64;
        let mut failures = Vec::new();
        let mut folder_unreads = serde_json::Map::new();
        let mut folder_counts = Vec::new();
        for (account, account_folder) in account_folders {
            let account_params = json!({ "account_id": account, "folder_id": account_folder });
            match mark_mobile_folder_all_read(data_directory, &account_params) {
                Ok(result) => {
                    updated += result
                        .get("updated")
                        .and_then(Value::as_u64)
                        .unwrap_or_default();
                    if let Some(counts) = result
                        .get("folder_unreads")
                        .and_then(|all| all.get(&account))
                    {
                        folder_unreads.insert(account.clone(), counts.clone());
                    }
                    folder_counts.extend(
                        result
                            .get("folder_counts")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default(),
                    );
                }
                Err(message) => failures.push(json!({ "account_id": account, "message": message })),
            }
        }
        return Ok(json!({
            "ok": failures.is_empty(),
            "updated": updated,
            "failures": failures,
            "folder_unreads": folder_unreads,
            "folder_counts": folder_counts,
        }));
    }

    let engine = crate::ffi::engine_for(data_directory)?;
    with_mobile_db(data_directory, |conn| {
        if is_rss_account(&conn, &account_id)? {
            return Ok(json!({ "ok": true, "updated": 0, "rss": true }));
        }
        // Mail in Meron folders stays unread: each has its own Mark all read.
        let uids = store::get_visible_unseen_uids(&conn, &account_id, &folder)
            .map_err(|error| error.to_string())?;
        let updated = uids.len();
        if !uids.is_empty() {
            let creds = load_mobile_account_creds(&conn, &account_id)?;
            if account_needs_reconnect(&creds) {
                return Err(format!("account needs reconnect: {account_id}"));
            }
            let select_folder = folder.clone();
            crate::ffi::engine_block_on(engine.with_flag_write_session(
                &account_id,
                move |session| {
                    let folder = select_folder.clone();
                    Box::pin(async move { imap::prepare_flag_update(session, &folder).await })
                },
                move |session| {
                    let uids = uids.clone();
                    Box::pin(async move {
                        imap::store_seen(session, &uids, true).await?;
                        anyhow::Ok(())
                    })
                },
            ))?;
        }
        store::mark_visible_folder_seen(&conn, &account_id, &folder)
            .map_err(|error| error.to_string())?;
        // The Inbox also lists mail the AI left in other folders: mark it read
        // where each message really is.
        let mut updated = updated;
        if folder.eq_ignore_ascii_case("INBOX") {
            let shown = store::ai_sort_inbox_unseen(&conn, &account_id)
                .map_err(|error| error.to_string())?;
            for (shown_folder, shown_uids) in shown {
                let select_folder = shown_folder.clone();
                let seen_uids = shown_uids.clone();
                crate::ffi::engine_block_on(engine.with_flag_write_session(
                    &account_id,
                    move |session| {
                        let folder = select_folder.clone();
                        Box::pin(async move { imap::prepare_flag_update(session, &folder).await })
                    },
                    move |session| {
                        let uids = seen_uids.clone();
                        Box::pin(async move {
                            imap::store_seen(session, &uids, true).await?;
                            anyhow::Ok(())
                        })
                    },
                ))?;
                store::meron_mark_seen(&conn, &account_id, &shown_folder, &shown_uids)
                    .map_err(|error| error.to_string())?;
                updated += shown_uids.len();
            }
        }
        crate::mail_model::mutation_result(
            json!({ "ok": true, "updated": updated, "folder": folder }),
            &conn,
            &account_id,
            "",
            &folder,
            None,
            Some(false),
            None,
            false,
        )
        .map_err(|error| error.to_string())
    })
}

// Permanently delete every message in a folder, server side and in the store.
// Restricted to Trash and Junk, and re-checked here rather than trusting the
// caller: the delete is unrecoverable.
pub(crate) fn empty_mobile_folder(data_directory: &str, params: &Value) -> Result<Value, String> {
    let account_id = required_account_id(params)?;
    let folder = params
        .get("folder_id")
        .or_else(|| params.get("folder"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(canon_folder)
        .ok_or_else(|| "folder_id is required".to_string())?;
    if account_id == "unified" {
        return Err("Only Trash and Junk folders can be emptied".to_string());
    }

    let engine = crate::ffi::engine_for(data_directory)?;
    with_mobile_db(data_directory, |conn| {
        if is_rss_account(&conn, &account_id)? {
            return Err("Only Trash and Junk folders can be emptied".to_string());
        }
        let role =
            store::folder_role(&conn, &account_id, &folder).map_err(|error| error.to_string())?;
        if role != "trash" && role != "junk" {
            return Err("Only Trash and Junk folders can be emptied".to_string());
        }
        let creds = load_mobile_account_creds(&conn, &account_id)?;
        if account_needs_reconnect(&creds) {
            return Err(format!("account needs reconnect: {account_id}"));
        }
        let expunged = {
            let folder = folder.clone();
            crate::ffi::engine_block_on(engine.with_write_session(&account_id, move |session| {
                let folder = folder.clone();
                Box::pin(async move { imap::empty_folder(session, &folder).await })
            }))?
        };
        let deleted = store::delete_folder_messages(&conn, &account_id, &folder)
            .map_err(|error| error.to_string())?;
        crate::mail_model::mutation_result(
            json!({
                "ok": true,
                "deleted": deleted,
                "expunged": expunged,
                "folder": folder,
                "role": role,
            }),
            &conn,
            &account_id,
            "",
            &folder,
            None,
            None,
            None,
            true,
        )
        .map_err(|error| error.to_string())
    })
}
