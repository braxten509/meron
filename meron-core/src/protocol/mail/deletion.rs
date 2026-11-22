//! Mobile mail protocol handlers for deletion.
//!
//! The mobile dispatcher calls these handlers with protocol parameters.
//! They translate between the wire format and the shared mail engine.

use super::*;

/// A local cache write after the server already moved or deleted the mail.
/// Its failure is logged, not returned: the server change stands, so the
/// command reports it done (and its undo still works from the server copies),
/// and the next sync of the folders brings the cache in line with the server.
fn local_after_server_change<T>(action: &str, account: &str, result: anyhow::Result<T>) {
    if let Err(err) = result {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.sync",
            "account={account}: {action} done on the server, local cache update failed: {err:#}"
        );
    }
}

/// Cache a server move within one account the way
/// [`crate::engine::move_within_account`] does: store the target's re-read
/// window (the real copies, under their new UIDs), then drop the source rows.
/// Source rows are never relabelled into the target, since their UIDs name
/// other messages there. Failures are logged and the affected folders resynced,
/// as is the target when the move's copies could not be read or identified.
/// Returns whether the cache now holds the move, so its unread counts are real.
#[allow(clippy::too_many_arguments)]
pub(super) fn cache_server_move(
    data_dir: &str,
    conn: &rusqlite::Connection,
    action: &str,
    account: &str,
    source: &str,
    target: &str,
    uids: &[u32],
    copies: &crate::engine::MovedCopies,
) -> bool {
    let stored = copies.store(conn, account, target);
    let removed = store::delete_messages_by_uid(conn, account, source, uids);
    // The re-read window holds the newest messages, so mail arriving at the
    // same time can push a copy out of it. The target counts are only real
    // when every identified copy is in the stored window.
    let copies_in_batch = copies.batch.as_ref().is_some_and(|batch| {
        let cached: std::collections::HashSet<u32> =
            batch.messages.iter().map(|message| message.uid).collect();
        copies.uids.iter().all(|uid| cached.contains(uid))
    });
    let target_cached = stored.is_ok() && !copies.uids.is_empty() && copies_in_batch;
    let removed_ok = removed.is_ok();
    local_after_server_change(action, account, stored);
    local_after_server_change(action, account, removed);
    let mut resync = Vec::new();
    if !removed_ok {
        resync.push(source);
    }
    if !target_cached {
        resync.push(target);
    }
    if !resync.is_empty() {
        crate::ffi::spawn_folder_resync(data_dir, account, &resync);
    }
    target_cached && removed_ok
}

pub(crate) fn delete_mobile_thread(data_dir: &str, params: &Value) -> Result<Value, String> {
    let thread_id = req_str(params, "thread_id")?;
    let parsed = parse_thread_id_with_request_folder(&thread_id, params)
        .ok_or_else(|| "invalid thread_id".to_string())?;
    let engine = crate::ffi::engine_for(data_dir)?;
    with_mobile_db(data_dir, |conn| {
        let uids = requested_mobile_uids(&conn, &parsed, params)?;
        // Deleting a sender row also takes their older mail in this folder
        // that the cache never held.
        let uids = match store::sender_key_addr(&parsed.thread_key) {
            Some(addr) if !has_requested_mobile_message_ids(params) => {
                crate::ffi::engine_block_on(crate::engine::sender_delete_uids(
                    &engine,
                    &parsed.account,
                    &parsed.folder,
                    &addr,
                    uids,
                ))?
            }
            _ => uids,
        };
        if uids.is_empty() {
            return Ok(json!({ "ok": true, "deleted": 0 }));
        }
        let creds = load_mobile_account_creds(&conn, &parsed.account)?;
        if account_needs_reconnect(&creds) {
            return Err(format!("account needs reconnect: {}", parsed.account));
        }
        let cached_ids = store::cached_message_ids(&conn, &parsed.account, &parsed.folder, &uids)
            .map_err(|err| err.to_string())?;
        let moved_ids = crate::ffi::engine_block_on(async {
            anyhow::Ok(
                crate::engine::moved_message_ids(
                    &engine,
                    &parsed.account,
                    &parsed.folder,
                    &uids,
                    cached_ids,
                )
                .await,
            )
        })?;
        // Deleting the whole conversation takes it out of its Meron folder.
        let meron_entry = crate::meron_folders::entry_before_action(
            &conn,
            &parsed.account,
            &parsed.folder,
            &parsed.thread_key,
            parsed.uid,
            !has_requested_mobile_message_ids(params),
        );
        crate::meron_folders::unmark_before_action_blocking(&engine, &parsed.account, &meron_entry);
        let delete_result = crate::ffi::engine_block_on(crate::engine::delete_to_trash(
            &engine,
            &parsed.account,
            &parsed.folder,
            &uids,
        ))?;
        let meron_released = crate::meron_folders::release_after_action(&conn, meron_entry);
        // Undo moves exactly these copies back out of Trash.
        let trash_copies = match &delete_result {
            Some(trashed) => {
                let copies = crate::ffi::engine_block_on(async {
                    anyhow::Ok(
                        crate::engine::refresh_moved_copies(
                            &engine,
                            &parsed.account,
                            &trashed.folder,
                            uids.len(),
                            trashed.uid_next_before,
                            moved_ids.as_deref(),
                        )
                        .await,
                    )
                })?;
                if let Err(err) = copies.store(&conn, &parsed.account, &trashed.folder) {
                    local_after_server_change("delete", &parsed.account, Err::<(), _>(err));
                    crate::ffi::spawn_folder_resync(
                        data_dir,
                        &parsed.account,
                        &[trashed.folder.as_str()],
                    );
                }
                copies.uids
            }
            None => Vec::new(),
        };
        // Discarding a draft must also drop hidden local copies sharing its
        // Message-ID (stale autosaves the pane deduped away), or the thread
        // card keeps its has_draft badge until the next full sync.
        let siblings = if store::folder_role(&conn, &parsed.account, &parsed.folder)
            .is_ok_and(|role| role == "drafts")
        {
            store::delete_draft_sibling_copies(&conn, &parsed.account, &parsed.folder, &uids)
        } else {
            Ok(0)
        };
        let rows = store::delete_messages_by_uid(&conn, &parsed.account, &parsed.folder, &uids);
        let cache_ok = siblings.is_ok() && rows.is_ok();
        // The whole conversation went: it starts fresh when new mail arrives
        // (see `store::ChatScope`). Needs the deleted rows out of the cache.
        if cache_ok
            && !has_requested_mobile_message_ids(params)
            && store::leave_replies_out_after_delete(
                &conn,
                &parsed.account,
                &parsed.folder,
                &parsed.thread_key,
                parsed.subject_filter.as_deref(),
                delete_result
                    .as_ref()
                    .map(|trashed| (trashed.folder.as_str(), trash_copies.as_slice())),
            )
        {
            crate::ffi::spawn_tag_push(data_dir, &parsed.account);
        }
        if !cache_ok {
            // Deleted rows are still cached: resync the folder, whose server
            // UID list no longer has them, so the sync prunes them.
            local_after_server_change("delete", &parsed.account, siblings);
            local_after_server_change("delete", &parsed.account, rows);
            crate::ffi::spawn_folder_resync(data_dir, &parsed.account, &[parsed.folder.as_str()]);
        }
        // Report the successful server operation, not the racy local row count.
        let deleted = uids.len();
        // A cache still holding the deleted rows would report wrong unread
        // counts: answer without them.
        if !cache_ok {
            let mut out = match &delete_result {
                None => json!({ "ok": true, "deleted": deleted, "permanent": true }),
                Some(crate::engine::Trashed { folder: trash, .. }) => json!({
                    "ok": true,
                    "deleted": deleted,
                    "trash": trash,
                    "thread_id": format_thread_id(&parsed.account, trash, &parsed.thread_key),
                    "target_uids": trash_copies,
                }),
            };
            crate::meron_folders::ReleasedCard::annotate(&meron_released, &mut out);
            return Ok(out);
        }
        match delete_result {
            None => crate::mail_model::mutation_result(
                json!({ "ok": true, "deleted": deleted, "permanent": true }),
                &conn,
                &parsed.account,
                &thread_id,
                &parsed.folder,
                None,
                None,
                None,
                true,
            ),
            Some(crate::engine::Trashed { folder: trash, .. }) => {
                let moved_thread_id = format_thread_id(&parsed.account, &trash, &parsed.thread_key);
                crate::mail_model::mutation_result(
                    json!({
                        "ok": true,
                        "deleted": deleted,
                        "trash": trash,
                        "thread_id": moved_thread_id,
                        "target_uids": trash_copies,
                    }),
                    &conn,
                    &parsed.account,
                    &thread_id,
                    &parsed.folder,
                    Some(&trash),
                    None,
                    None,
                    true,
                )
            }
        }
        .map(|mut out| {
            crate::meron_folders::ReleasedCard::annotate(&meron_released, &mut out);
            out
        })
        .map_err(|err| err.to_string())
    })
}

/// A bulk Delete: `items` holds the params of one `mail.delete` per picked conversation.
///
/// Each runs as that single delete, all together as one bulk run, so their server steps are done
/// once for all of them (see [`crate::engine::bulk`]). `results` answers each item in order: the
/// single delete's result, or `{ "error": "…" }`: the message as plain text, so the answer as a
/// whole does not read as a failed command.
pub(crate) fn delete_mobile_threads(data_dir: &str, params: &Value) -> Result<Value, String> {
    let items = params
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing items".to_string())?;
    let engine = crate::ffi::engine_for(data_dir)?;
    let results: Vec<Value> =
        crate::engine::bulk::run_together_blocking(&engine, items.len(), |index| {
            delete_mobile_thread(data_dir, &items[index])
        })
        .into_iter()
        .map(|result| result.unwrap_or_else(|err| json!({ "error": err })))
        .collect();
    Ok(json!({ "ok": true, "results": results }))
}

pub(crate) fn archive_mobile_thread(data_dir: &str, params: &Value) -> Result<Value, String> {
    let thread_id = req_str(params, "thread_id")?;
    let parsed = parse_thread_id(&thread_id).ok_or_else(|| "invalid thread_id".to_string())?;
    let engine = crate::ffi::engine_for(data_dir)?;
    with_mobile_db(data_dir, |conn| {
        let target_folder = match find_cached_archive_folder(&conn, &parsed.account) {
            Ok(folder) => folder,
            // No archive mailbox: a whole conversation is archived Meron-only
            // (filed into the built-in Archive here, no server move).
            Err(no_archive) => {
                let Some((result, jobs)) =
                    crate::meron_folders::archive_meron_only(&conn, &thread_id)
                        .map_err(|err| format!("{err:#}"))?
                else {
                    return Err(no_archive);
                };
                crate::meron_marks::spawn_writes(&engine, jobs);
                return Ok(result);
            }
        };
        if parsed.folder == target_folder {
            return Ok(json!({
                "ok": true,
                "moved": 0,
                "folder": target_folder,
                "thread_id": thread_id,
            }));
        }
        let uids = cached_thread_uids(&conn, &parsed)?;
        if uids.is_empty() {
            return Ok(json!({
                "ok": true,
                "moved": 0,
                "folder": target_folder,
                "thread_id": thread_id,
            }));
        }
        let creds = load_mobile_account_creds(&conn, &parsed.account)?;
        if account_needs_reconnect(&creds) {
            return Err(format!("account needs reconnect: {}", parsed.account));
        }
        let cached_ids = store::cached_message_ids(&conn, &parsed.account, &parsed.folder, &uids)
            .map_err(|err| err.to_string())?;
        let moved_ids = crate::ffi::engine_block_on(async {
            anyhow::Ok(
                crate::engine::moved_message_ids(
                    &engine,
                    &parsed.account,
                    &parsed.folder,
                    &uids,
                    cached_ids,
                )
                .await,
            )
        })?;
        // Archiving or moving the whole conversation takes it out of its
        // Meron folder once the server move succeeded.
        let meron_entry = crate::meron_folders::entry_before_action(
            &conn,
            &parsed.account,
            &parsed.folder,
            &parsed.thread_key,
            parsed.uid,
            true,
        );
        // A whole conversation archived goes into the built-in Archive too.
        let archive_keys = crate::meron_folders::archive_filing_keys(
            &conn,
            &parsed.account,
            &parsed.folder,
            &parsed.thread_key,
            &uids,
            true,
        );
        if archive_keys.is_some() {
            crate::meron_folders::prepare_archive_marks_blocking(
                &engine,
                &parsed.account,
                &meron_entry,
                &parsed.folder,
                &uids,
            );
        } else {
            crate::meron_folders::unmark_before_action_blocking(
                &engine,
                &parsed.account,
                &meron_entry,
            );
        }
        let op_folder = parsed.folder.clone();
        let op_target = target_folder.clone();
        let op_uids = uids.clone();
        let moved_on_server = crate::ffi::engine_block_on(engine.with_write_session(
            &parsed.account,
            move |session| {
                let source = op_folder.clone();
                let target = op_target.clone();
                let uids = op_uids.clone();
                Box::pin(
                    async move { imap::move_to_folder(session, &source, &target, &uids).await },
                )
            },
        ));
        let uid_next_before = match moved_on_server {
            Ok(uid_next_before) => uid_next_before,
            Err(err) => {
                if archive_keys.is_some() {
                    crate::meron_folders::restore_marks_after_failed_archive(
                        &engine,
                        &conn,
                        &parsed.account,
                        &meron_entry,
                        &parsed.folder,
                        &uids,
                    );
                }
                return Err(err);
            }
        };
        let meron_released = crate::meron_folders::release_after_action(&conn, meron_entry);
        let copies = crate::ffi::engine_block_on(async {
            anyhow::Ok(
                crate::engine::refresh_moved_copies(
                    &engine,
                    &parsed.account,
                    &target_folder,
                    uids.len(),
                    uid_next_before,
                    moved_ids.as_deref(),
                )
                .await,
            )
        })?;
        let cache_ok = cache_server_move(
            data_dir,
            &conn,
            "archive",
            &parsed.account,
            &parsed.folder,
            &target_folder,
            &uids,
            &copies,
        );
        if let Some(keys) = &archive_keys {
            crate::meron_folders::file_after_archive(
                &conn,
                &parsed.account,
                &target_folder,
                &copies.uids,
                keys,
            );
        }
        crate::meron_marks::flush_archive_unmarks(&engine);
        // The server MOVE succeeded for all resolved UIDs. A concurrent sync
        // can remove a source cache row before this local reconciliation runs.
        let moved = uids.len();
        let moved_thread_id = format_thread_id(&parsed.account, &target_folder, &parsed.thread_key);
        let mut result = json!({
            "ok": true,
            "moved": moved,
            "folder": target_folder,
            "thread_id": moved_thread_id,
            "target_uids": copies.uids,
        });
        crate::meron_folders::ReleasedCard::annotate(&meron_released, &mut result);
        // A cache that missed the move would report wrong unread counts.
        if !cache_ok {
            return Ok(result);
        }
        crate::mail_model::mutation_result(
            result,
            &conn,
            &parsed.account,
            &thread_id,
            &parsed.folder,
            Some(&target_folder),
            None,
            None,
            true,
        )
        .map_err(|err| err.to_string())
    })
}
