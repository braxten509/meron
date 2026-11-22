//! Moves and copies messages between server mailboxes.
//! The sidecar resolves the selected messages and synchronizes local folder state.
//! Responses identify the conversations affected by each transfer.

use super::*;

pub(super) async fn move_messages(
    engine: &Arc<Engine>,
    parameters: &Value,
) -> anyhow::Result<Value> {
    let account = required_string(parameters, "account")?;
    let folder = canon_folder(
        &required_string(parameters, "folder").unwrap_or_else(|_| "INBOX".to_string()),
    );
    let target_folder = canon_folder(&required_string(parameters, "target_folder")?);
    let (thread_key, subject_filter) =
        store::split_thread_key(&required_string(parameters, "thread_key").unwrap_or_default());
    let uid = optional_message_uid(parameters)?;
    let explicit_uids = explicit_message_uids(parameters)?;
    let changed_thread_id = if thread_key.is_empty() {
        uid.map(|uid| format!("{account}#{folder}#{uid}"))
            .unwrap_or_default()
    } else {
        let key = subject_filter
            .as_deref()
            .map(|subject| store::branch_compound_key(&thread_key, subject))
            .unwrap_or_else(|| thread_key.clone());
        mail_model::format_thread_id(&account, &folder, &key)
    };

    let target_account =
        required_string(parameters, "target_account").unwrap_or_else(|_| account.clone());
    if account == target_account && folder == target_folder {
        return mail_model::mutation_result(
            json!({ "ok": true, "moved": 0, "source_folder": folder, "target_folder": target_folder }),
            &engine.db.lock().unwrap(),
            &account,
            &changed_thread_id,
            &folder,
            Some(&target_folder),
            None,
            None,
            false,
        );
    }

    let uids = {
        let db = engine.db.lock().unwrap();
        store::resolve_message_uids(
            &db,
            &account,
            &folder,
            &thread_key,
            subject_filter.as_deref(),
            uid,
            &explicit_uids,
        )?
    };

    if uids.is_empty() {
        return mail_model::mutation_result(
            json!({ "ok": true, "moved": 0, "source_folder": folder, "target_folder": target_folder }),
            &engine.db.lock().unwrap(),
            &account,
            &changed_thread_id,
            &folder,
            Some(&target_folder),
            None,
            None,
            false,
        );
    }

    // An Undo of an archive, delete or move: what it puts back besides
    // the mail (see `UndoRestore`).
    let undo = meron_core::meron_folders::UndoRestore::from_params(parameters);
    // Archiving or moving the whole conversation takes it out of its
    // Meron folder once the server move succeeded.
    let whole = explicit_uids.is_empty() && (uid.is_none() || thread_key.starts_with("uid:"));
    let (meron_entry, archive_keys) = {
        let db = engine.db.lock().unwrap();
        let mut meron_entry = meron_core::meron_folders::entry_before_action(
            &db,
            &account,
            &folder,
            &thread_key,
            uid.filter(|_| thread_key.starts_with("uid:")),
            whole,
        );
        // Exactly these copies out of the archive mailbox (an Undo of
        // an archive, say): they leave the built-in Archive.
        if meron_entry.is_none() && !explicit_uids.is_empty() && target_account == account {
            meron_entry =
                meron_core::meron_folders::archive_card_of_copies(&db, &account, &folder, &uids);
        }
        // Moved into the archive mailbox as a whole: filed into
        // Archive too.
        let archive_keys = (target_account == account
            && meron_core::meron_folders::is_archive_target(&db, &account, &target_folder))
        .then(|| {
            meron_core::meron_folders::archive_filing_keys(
                &db,
                &account,
                &folder,
                &thread_key,
                &uids,
                whole,
            )
        })
        .flatten();
        (meron_entry, archive_keys)
    };
    if archive_keys.is_some() {
        meron_core::meron_folders::prepare_archive_marks(
            engine,
            &account,
            &meron_entry,
            &folder,
            &uids,
        )
        .await;
    } else {
        meron_core::meron_folders::unmark_before_action(engine, &account, &meron_entry).await;
    }

    if target_account != account {
        let mut result = move_between_accounts(
            engine,
            &account,
            &folder,
            &changed_thread_id,
            &target_account,
            &target_folder,
            &uids,
        )
        .await?;
        let meron_released = meron_core::meron_folders::release_after_action(
            &engine.db.lock().unwrap(),
            meron_entry,
        );
        meron_core::meron_folders::ReleasedCard::annotate(&meron_released, &mut result);
        return Ok(result);
    }

    // One of a bulk run with more to come: the target is re-read once,
    // by the run's last move. Never put off for an archive or an Undo:
    // both read the moved copies from the cache straight after.
    let reread_target = !parameters
        .get("more_coming")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || archive_keys.is_some()
        || undo.is_some();
    // What this move teaches AI sorting (into Spam, back to the
    // Inbox), read while the mail is still cached here.
    let lesson = store::ai_lesson_before_move(
        &engine.db.lock().unwrap(),
        &account,
        &folder,
        &target_folder,
        &uids,
        undo.is_some(),
    );
    let copies = match move_within_account_with(
        engine,
        &account,
        &folder,
        &target_folder,
        &uids,
        reread_target,
    )
    .await
    {
        Ok(copies) => copies,
        Err(err) => {
            if archive_keys.is_some() {
                meron_core::meron_folders::restore_marks_after_failed_archive(
                    engine,
                    &engine.db.lock().unwrap(),
                    &account,
                    &meron_entry,
                    &folder,
                    &uids,
                );
            } else {
                // Out of the archive mailbox: the Archive card keeps
                // its place, so its marks come back.
                meron_core::meron_folders::restore_archive_card_marks(
                    engine,
                    &engine.db.lock().unwrap(),
                    &meron_entry,
                );
            }
            return Err(err);
        }
    };
    let meron_released =
        meron_core::meron_folders::release_after_action(&engine.db.lock().unwrap(), meron_entry);
    if lesson.learn(&engine.db.lock().unwrap()) {
        meron_core::engine::ai_sort::push_lesson_tags(engine);
    }
    {
        let db = engine.db.lock().unwrap();
        if let Some(keys) = &archive_keys {
            meron_core::meron_folders::file_after_archive(
                &db,
                &account,
                &target_folder,
                &copies.uids,
                keys,
            );
        }
        if let Some(undo) = &undo {
            let jobs = undo.apply(&db, &account, &thread_key, &target_folder, &copies.uids);
            meron_core::meron_marks::spawn_writes(engine, jobs);
            // An Undo of a delete: the replies it left out come back.
            if store::bring_replies_back_after_undo(&db, &account, &folder, &uids) {
                spawn_tag_push(engine, &account);
            }
        }
    }
    meron_core::meron_marks::flush_archive_unmarks(engine);
    // The IMAP MOVE above completed for every resolved UID. A concurrent
    // source-folder refresh may already have pruned those rows locally,
    // so the cache DELETE count is not the number moved on the server.
    let moved = uids.len();
    let mut result = mail_model::mutation_result(
        json!({
            "ok": true,
            "moved": moved,
            "source_folder": folder,
            "target_folder": target_folder,
            "target_uids": copies.uids,
        }),
        &engine.db.lock().unwrap(),
        &account,
        &changed_thread_id,
        &folder,
        Some(&target_folder),
        None,
        None,
        true,
    )?;
    // An Undo puts the card back in its Meron folder.
    meron_core::meron_folders::ReleasedCard::annotate(&meron_released, &mut result);
    Ok(result)
}

pub(super) async fn copy_messages(
    engine: &Arc<Engine>,
    parameters: &Value,
) -> anyhow::Result<Value> {
    let account = required_string(parameters, "account")?;
    let folder = canon_folder(
        &required_string(parameters, "folder").unwrap_or_else(|_| "INBOX".to_string()),
    );
    let target_account = required_string(parameters, "target_account")?;
    let target_folder = canon_folder(&required_string(parameters, "target_folder")?);
    let (thread_key, subject_filter) =
        store::split_thread_key(&required_string(parameters, "thread_key").unwrap_or_default());
    let uid = optional_message_uid(parameters)?;
    let explicit_uids = explicit_message_uids(parameters)?;

    let uids = {
        let db = engine.db.lock().unwrap();
        store::resolve_message_uids(
            &db,
            &account,
            &folder,
            &thread_key,
            subject_filter.as_deref(),
            uid,
            &explicit_uids,
        )?
    };

    if uids.is_empty() {
        return Ok(json!({
            "ok": true,
            "copied": 0,
            "source_folder": folder,
            "target_account": target_account,
            "target_folder": target_folder
        }));
    }

    let raw_messages = engine
        .with_read_session(&account, |session| {
            let folder = folder.clone();
            let uids = uids.clone();
            Box::pin(
                async move { imap::fetch_raw_messages_for_copy(session, &folder, &uids).await },
            )
        })
        .await?;

    if raw_messages.is_empty() {
        return Ok(json!({
            "ok": true,
            "copied": 0,
            "source_folder": folder,
            "target_account": target_account,
            "target_folder": target_folder
        }));
    }

    let copied = raw_messages.len();
    engine
        .with_write_session(&target_account, |session| {
            let target_folder = target_folder.clone();
            let raw_messages = raw_messages.clone();
            Box::pin(async move {
                imap::append_copied_messages(session, &target_folder, &raw_messages).await
            })
        })
        .await?;
    {
        let db = engine.db.lock().unwrap();
        let copies = std::collections::BTreeMap::from([(folder.clone(), uids.clone())]);
        let tags = store::carried_tags(&db, &account, &copies)?;
        store::apply_carried_tags(&db, &target_account, tags, true)?;
    }
    // Read-only refresh, on its own session; see the move handler above.
    let copied_result = json!({
        "ok": true,
        "copied": copied,
        "source_folder": folder,
        "target_account": target_account,
        "target_folder": target_folder
    });
    let Some(refresh) = meron_core::engine::refresh_written_folder(
        engine,
        &target_account,
        &target_folder,
        50.max(raw_messages.len() as u32),
    )
    .await
    else {
        return Ok(copied_result);
    };
    let target_batch = &refresh.batch;

    {
        let db = engine.db.lock().unwrap();
        store::ensure_folder(&db, &target_account, &target_folder)?;
        store::upsert_window_messages(&db, &target_account, &target_folder, target_batch)?;
        store::set_folder_state(
            &db,
            &target_account,
            &target_folder,
            target_batch.uidvalidity,
            target_batch.uid_next,
        )?;
    }

    Ok(copied_result)
}
