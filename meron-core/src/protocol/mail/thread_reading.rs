//! Mobile mail protocol handlers for thread reading.
//!
//! The mobile dispatcher calls these handlers with protocol parameters.
//! They translate between the wire format and the shared mail engine.

use super::*;

pub(crate) fn read_mobile_thread(data_directory: &str, params: &Value) -> Result<Value, String> {
    let thread_id = required_string(params, "thread_id")?;
    if is_rss_thread_id(&thread_id) {
        return read_mobile_rss_thread(data_directory, params);
    }
    let parsed = parse_thread_id(&thread_id).ok_or_else(|| "invalid thread_id".to_string())?;
    let engine = crate::ffi::engine_for(data_directory)?;
    // Mobile sync stores headers only, so opening a thread routinely finds
    // uncached bodies; the shared reader pulls the newest one inline and fills
    // the rest in the background. `mail.tailSynced` is the body-only refresh
    // signal: the mobile UI answers it by re-reading just the open thread.
    let thread_key = match parsed.uid {
        Some(uid) => format!("uid:{uid}"),
        None => parsed.thread_key.clone(),
    };
    let limit = optional_u32(params, "limit");
    let before_cursor = params
        .get("before_cursor")
        .and_then(Value::as_str)
        .map(str::to_string);
    let on_bodies_fetched: crate::thread_read::BodiesFetchedHook = {
        let account = parsed.account.clone();
        Box::new(move || {
            let _ = crate::ffi::emit_event(
                "mail.tailSynced",
                serde_json::json!({ "account": account }),
            );
        })
    };
    // With look-ahead: the reader pages back a page at a time and would
    // otherwise meet every older page as placeholders filling in under it.
    crate::ffi::engine_block_on(crate::thread_read::read_thread_page_with_lookahead(
        &engine,
        crate::thread_read::ThreadReadArgs {
            account: &parsed.account,
            folder: &parsed.folder,
            thread_id: &thread_id,
            thread_key: &thread_key,
            subject_filter: parsed.subject_filter.as_deref(),
            limit,
            for_print: params
                .get("for_print")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            before_cursor: before_cursor.as_deref(),
            media_root: std::path::PathBuf::from(data_directory).join("attachments"),
        },
        Some(on_bodies_fetched),
    ))
}

pub(crate) fn list_mobile_starred_items(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let limit = optional_u32(params, "limit").unwrap_or(200);
    with_mobile_db(data_directory, |conn| {
        let mut items = crate::mail_model::starred_thread_cards(&conn, 2_000)
            .map_err(|error| format!("{error:#}"))?;
        items.extend(rss::starred_items(&conn, 2_000).map_err(|error| format!("{error:#}"))?);
        Ok(crate::mail_model::starred_page(
            items,
            params
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            params
                .get("filter")
                .and_then(Value::as_str)
                .unwrap_or("all"),
            params
                .get("attachments")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            limit as usize,
            params.get("before_cursor").and_then(Value::as_str),
        ))
    })
}

/// Which of the given messages are still unread, for clients withdrawing new-mail notifications.
///
/// `messages` holds `{account_id, folder_id, message_id}` entries (a feed item: its subscription
/// and item key); the result lists the ones still unread, echoed exactly as asked so a client can
/// match them to its own keys. Reads the cache only, so it reflects other devices as of the last
/// sync.
pub fn messages_still_unread(conn: &rusqlite::Connection, params: &Value) -> Result<Value, String> {
    let entries = params
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "messages required".to_string())?;
    let mut by_account: std::collections::BTreeMap<&str, Vec<(&str, &str)>> =
        std::collections::BTreeMap::new();
    for entry in entries {
        let field = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or_default();
        let (account, folder, message) =
            (field("account_id"), field("folder_id"), field("message_id"));
        if account.is_empty() || folder.is_empty() || message.is_empty() {
            continue;
        }
        by_account
            .entry(account)
            .or_default()
            .push((folder, message));
    }
    let mut unread = Vec::new();
    for (account, messages) in by_account {
        // A `mail.newMessages` detail names the inbox "inbox"; the store keeps
        // it as "INBOX".
        let lookups: Vec<(String, String)> = messages
            .iter()
            .map(|(folder, message)| (canon_folder(folder), message.to_string()))
            .collect();
        let flags = store::messages_still_unread(conn, account, &lookups)
            .map_err(|error| error.to_string())?;
        unread.extend(messages.iter().zip(flags).filter(|(_, unread)| *unread).map(
            |((folder, message), _)| {
                json!({ "account_id": account, "folder_id": folder, "message_id": message })
            },
        ));
    }
    Ok(json!({ "unread": unread }))
}

pub(crate) fn mobile_messages_still_unread(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    with_mobile_db(data_directory, |conn| messages_still_unread(&conn, params))
}
