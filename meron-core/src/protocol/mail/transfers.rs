//! Mobile mail protocol handlers for transfers.
//!
//! The mobile dispatcher calls these handlers with protocol parameters.
//! They translate between the wire format and the shared mail engine.

use super::*;

pub(crate) fn move_mobile_thread(data_dir: &str, params: &Value) -> Result<Value, String> {
    let thread_id = req_str(params, "thread_id")?;
    let target_folder = canon_folder(&req_str(params, "target_folder_id")?);
    let parsed = parse_thread_id(&thread_id).ok_or_else(|| "invalid thread_id".to_string())?;
    // An Undo moves back exactly the copies the action made. Without them it
    // would fall back to the thread id, which in Archive or Trash names other
    // mail (a `uid:` id is another message there; a real thread takes the
    // whole conversation out): refuse instead of guessing.
    let undo = crate::meron_folders::UndoRestore::from_params(params);
    if (undo.is_some() || requests_empty_message_ids(params))
        && !has_requested_mobile_message_ids(params)
    {
        return Err("undo unavailable: the moved copies are unknown".to_string());
    }
    let target_account = params
        .get("target_account_id")
        .and_then(Value::as_str)
        .unwrap_or(&parsed.account);
    if target_account != parsed.account {
        let engine = crate::ffi::engine_for(data_dir)?;
        return with_mobile_db(data_dir, |conn| {
            let uids = requested_mobile_uids(&conn, &parsed, params)?;
            if !uids.is_empty() {
                for account in [parsed.account.as_str(), target_account] {
                    let creds = load_mobile_account_creds(&conn, account)?;
                    if account_needs_reconnect(&creds) {
                        return Err(format!("account needs reconnect: {account}"));
                    }
                }
            }
            let meron_entry = crate::meron_folders::entry_before_action(
                &conn,
                &parsed.account,
                &parsed.folder,
                &parsed.thread_key,
                parsed.uid,
                !has_requested_mobile_message_ids(params),
            );
            crate::meron_folders::unmark_before_action_blocking(
                &engine,
                &parsed.account,
                &meron_entry,
            );
            let mut result = crate::ffi::engine_block_on(crate::engine::move_between_accounts(
                &engine,
                &parsed.account,
                &parsed.folder,
                &thread_id,
                target_account,
                &target_folder,
                &uids,
            ))?;
            let meron_released = crate::meron_folders::release_after_action(&conn, meron_entry);
            crate::meron_folders::ReleasedCard::annotate(&meron_released, &mut result);
            Ok(result)
        });
    }
    if parsed.folder == target_folder {
        return Ok(
            json!({ "ok": true, "moved": 0, "folder": target_folder, "thread_id": thread_id }),
        );
    }
    let engine = crate::ffi::engine_for(data_dir)?;
    with_mobile_db(data_dir, |conn| {
        let uids = requested_mobile_uids(&conn, &parsed, params)?;
        if uids.is_empty() {
            return Ok(
                json!({ "ok": true, "moved": 0, "folder": target_folder, "thread_id": thread_id }),
            );
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
        let whole = !has_requested_mobile_message_ids(params);
        let mut meron_entry = crate::meron_folders::entry_before_action(
            &conn,
            &parsed.account,
            &parsed.folder,
            &parsed.thread_key,
            parsed.uid,
            whole,
        );
        // Exactly these copies out of the archive mailbox (an Undo of an
        // archive, say): they leave the built-in Archive.
        if meron_entry.is_none() && !whole {
            meron_entry = crate::meron_folders::archive_card_of_copies(
                &conn,
                &parsed.account,
                &parsed.folder,
                &uids,
            );
        }
        // Moved into the archive mailbox as a whole: filed into Archive too.
        let archive_keys =
            if crate::meron_folders::is_archive_target(&conn, &parsed.account, &target_folder) {
                crate::meron_folders::archive_filing_keys(
                    &conn,
                    &parsed.account,
                    &parsed.folder,
                    &parsed.thread_key,
                    &uids,
                    whole,
                )
            } else {
                None
            };
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
        // What this move teaches AI sorting (into Spam, back to the Inbox),
        // read while the mail is still cached here.
        let lesson = store::ai_lesson_before_move(
            &conn,
            &parsed.account,
            &parsed.folder,
            &target_folder,
            &uids,
            undo.is_some(),
        );
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
                } else {
                    // Out of the archive mailbox: the Archive card keeps its
                    // place, so its marks come back.
                    crate::meron_folders::restore_archive_card_marks(&engine, &conn, &meron_entry);
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
            "move",
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
        if lesson.learn(&conn) {
            crate::engine::ai_sort::push_lesson_tags(&engine);
        }
        if let Some(undo) = &undo {
            let jobs = undo.apply(
                &conn,
                &parsed.account,
                &parsed.thread_key,
                &target_folder,
                &copies.uids,
            );
            crate::meron_marks::spawn_writes(&engine, jobs);
            // An Undo of a delete: the replies it left out come back.
            if store::bring_replies_back_after_undo(&conn, &parsed.account, &parsed.folder, &uids) {
                crate::ffi::spawn_tag_push(data_dir, &parsed.account);
            }
        }
        // Report the successful server operation, not the racy local row count.
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

pub(crate) fn copy_mobile_thread(data_dir: &str, params: &Value) -> Result<Value, String> {
    let thread_id = req_str(params, "thread_id")?;
    let target_account = req_str(params, "target_account_id")?;
    let target_folder = canon_folder(&req_str(params, "target_folder_id")?);
    let parsed = parse_thread_id(&thread_id).ok_or_else(|| "invalid thread_id".to_string())?;
    let engine = crate::ffi::engine_for(data_dir)?;
    with_mobile_db(data_dir, |conn| {
        let uids = cached_thread_uids(&conn, &parsed)?;
        if uids.is_empty() {
            return Ok(json!({
                "ok": true,
                "copied": 0,
                "source_folder": parsed.folder,
                "target_account": target_account,
                "target_folder": target_folder,
            }));
        }
        let source_creds = load_mobile_account_creds(&conn, &parsed.account)?;
        if account_needs_reconnect(&source_creds) {
            return Err(format!("account needs reconnect: {}", parsed.account));
        }
        let target_creds = load_mobile_account_creds(&conn, &target_account)?;
        if account_needs_reconnect(&target_creds) {
            return Err(format!("account needs reconnect: {target_account}"));
        }
        let source_folder = parsed.folder.clone();
        let fetch_uids = uids.clone();
        let raw_messages = crate::ffi::engine_block_on(engine.with_read_session(
            &parsed.account,
            move |session| {
                let folder = source_folder.clone();
                let uids = fetch_uids.clone();
                Box::pin(
                    async move { imap::fetch_raw_messages_for_copy(session, &folder, &uids).await },
                )
            },
        ))?;
        let copied = raw_messages.len();
        let target_folder_op = target_folder.clone();
        crate::ffi::engine_block_on(engine.with_write_session(&target_account, move |session| {
            let target = target_folder_op.clone();
            let raw_messages = raw_messages.clone();
            Box::pin(
                async move { imap::append_copied_messages(session, &target, &raw_messages).await },
            )
        }))?;
        let copies = std::collections::BTreeMap::from([(parsed.folder.clone(), uids.clone())]);
        store::carried_tags(&conn, &parsed.account, &copies)
            .and_then(|tags| store::apply_carried_tags(&conn, &target_account, tags, true))
            .map_err(|err| err.to_string())?;
        // Read-only refresh, on its own session; see the move handlers above.
        let copied_result = json!({
            "ok": true,
            "copied": copied,
            "source_folder": parsed.folder,
            "target_account": target_account,
            "target_folder": target_folder,
        });
        let refresh = crate::ffi::engine_block_on(async {
            anyhow::Ok(
                crate::engine::refresh_written_folder(
                    &engine,
                    &target_account,
                    &target_folder,
                    50.max(copied as u32),
                )
                .await,
            )
        })?;
        let Some(refresh) = refresh else {
            return Ok(copied_result);
        };
        let batch = &refresh.batch;
        store::ensure_folder(&conn, &target_account, &target_folder)
            .map_err(|err| err.to_string())?;
        store::upsert_window_messages(&conn, &target_account, &target_folder, batch)
            .map_err(|err| err.to_string())?;
        store::set_folder_state(
            &conn,
            &target_account,
            &target_folder,
            batch.uidvalidity,
            batch.uid_next,
        )
        .map_err(|err| err.to_string())?;
        Ok(copied_result)
    })
}
