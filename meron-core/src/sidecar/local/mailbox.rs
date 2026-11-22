//! Serves mailbox requests for the local archive account.
//! Folder and thread operations work against the archive database.
//! Copying mail to a server account uses its authenticated engine.

use super::*;

pub(super) async fn local_mail(
    main: &Arc<Engine>,
    archive: &Arc<Engine>,
    command_request: &Request,
    out: &Writer,
) -> anyhow::Result<Value> {
    let parameters = &command_request.params;
    let account = LOCAL_ACCOUNT_ID;
    let media_root = parse::media_root();
    match command_request.method.as_str() {
        "folders.list" => {
            let folders = store::get_folders(&archive.db.lock().unwrap(), account)?;
            Ok(json!({ "folders": serde_json::to_value(folders)? }))
        }

        "folders.create" => {
            let folder = store::create_local_folder(
                &archive.db.lock().unwrap(),
                &required_string(parameters, "name")?,
            )?;
            Ok(json!({ "folders": serde_json::to_value(vec![folder])? }))
        }

        "folders.delete" => {
            let folder = request_folder(parameters)?;
            let db = archive.db.lock().unwrap();
            let (removed, deleted) = store::delete_local_folder(&db, &folder)?;
            for target in &removed {
                parse::remove_folder_media(&media_root, account, target);
            }
            Ok(json!({
                "ok": true,
                "folder": folder,
                "removed": removed,
                "deleted": deleted,
                "folders": serde_json::to_value(store::get_folders(&db, account)?)?,
            }))
        }

        "folders.rename" => {
            let folder = request_folder(parameters)?;
            let db = archive.db.lock().unwrap();
            let renamed = store::rename_local_folder(
                &db,
                &folder,
                &required_string(parameters, "name")?,
                &media_root,
            )?;
            Ok(json!({
                "ok": true,
                "folder": folder,
                "renamed": renamed,
                "folders": serde_json::to_value(store::get_folders(&db, account)?)?,
            }))
        }

        "folders.setNotify" => Err(anyhow::anyhow!(
            "Local folders get no new mail to notify about"
        )),
        "folders.archive" => Err(anyhow::anyhow!("Local folders have no Archive folder")),
        "messages.emptyFolder" => Err(anyhow::anyhow!(
            "Only Trash and Junk folders can be emptied"
        )),
        // .eml export: the kept originals, byte for byte.
        "messages.exportUids" => {
            let folder = canon_local(parameters);
            let db = archive.db.lock().unwrap();
            let mut uids = if parameters
                .get("all")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                store::local_folder_uids(&db, &folder)?
            } else {
                request_uids(&db, parameters, &folder)?
            };
            uids.sort_unstable();
            uids.dedup();
            Ok(json!({ "uids": uids }))
        }

        "messages.saveRaw" => {
            let folder = canon_local(parameters);
            let uid = required_u32(parameters, "uid")?;
            let path = required_string(parameters, "path")?;
            let raw = store::raw_message(&archive.db.lock().unwrap(), &folder, uid)?
                .ok_or_else(|| anyhow::anyhow!("message {uid} not found in {folder}"))?;
            write_private(std::path::Path::new(&path), &raw)?;
            Ok(json!({ "saved": true, "size": raw.len() }))
        }

        "messages.saveRawBatch" => {
            let folder = canon_local(parameters);
            let directory = std::path::PathBuf::from(required_string(parameters, "dir")?);
            let uids = explicit_message_uids(parameters)?;
            let db = archive.db.lock().unwrap();
            let mut saved = Vec::with_capacity(uids.len());
            for uid in uids {
                let Some(raw) = store::raw_message(&db, &folder, uid)? else {
                    continue;
                };
                write_private(&directory.join(format!("{uid}.eml")), &raw)?;
                saved.push(uid);
            }
            Ok(json!({ "saved": saved }))
        }

        // Nothing to sync or watch: a local folder changes only through Meron.
        "messages.sync" | "watch.start" | "watch.stop" => Ok(json!({ "ok": true })),

        "messages.recent" => {
            let request = thread_list::ThreadListQuery::from_params(parameters, "folder");
            let folder = request.folder.clone();
            let db = archive.db.lock().unwrap();
            let (messages, next_cursor) = match request.source() {
                thread_list::MailSource::Starred => (
                    store::get_starred(&db, account, &folder, request.limit)?,
                    None,
                ),
                thread_list::MailSource::Recent { filter } => store::get_recent_page(
                    &db,
                    account,
                    &folder,
                    request.limit,
                    request.before_cursor,
                    filter,
                )?,
                thread_list::MailSource::Search => {
                    let folders = engine::query_folders(&db, account, &folder, &request.query);
                    let messages = store::search_messages_in_folders(
                        &db,
                        account,
                        &folders,
                        &request.query,
                        request.limit,
                        request.search_before_cursor.as_ref(),
                    )?;
                    let next_cursor = store::search_next_cursor(&messages, request.limit, 0);
                    (messages, next_cursor)
                }
            };
            let mut page = thread_list::mail_page(
                &db,
                account,
                &folder,
                messages,
                next_cursor,
                parameters
                    .get("group")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )?;
            page["folder_synced"] = Value::Bool(true);
            request.retain_attachment_threads(&mut page);
            Ok(page)
        }

        "messages.read" | "messages.threadHeaders" => {
            if command_request.method == "messages.read" {
                let folder = canon_local(parameters);
                let uid = required_u32(parameters, "uid")?;
                store::restore_local_media(
                    &archive.db.lock().unwrap(),
                    &[(folder, uid)],
                    &media_root,
                )?;
            }
            refresh_identity(main, archive);
            Box::pin(crate::sidecar::dispatch::messages::dispatch(
                archive,
                command_request,
                out,
            ))
            .await
        }

        "messages.thread" => {
            let folder = canon_local(parameters);
            let (thread_key, subject_filter) =
                store::split_thread_key(&required_string(parameters, "thread_key")?);
            let limit = parameters
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            let thread_id = required_string(parameters, "thread_id")
                .unwrap_or_else(|_| mail_model::format_thread_id(account, &folder, &thread_key));
            // Only the page being read: an old conversation may be long.
            {
                let db = archive.db.lock().unwrap();
                let (headers, _) = thread_read::thread_page_headers(
                    &db,
                    account,
                    &folder,
                    &thread_key,
                    subject_filter.as_deref(),
                    limit,
                    parameters.get("before_cursor").and_then(Value::as_str),
                )?;
                let messages = headers
                    .into_iter()
                    .map(|header| {
                        let folder = if header.folder.is_empty() {
                            folder.clone()
                        } else {
                            header.folder
                        };
                        (folder, header.uid)
                    })
                    .collect::<Vec<_>>();
                store::restore_local_media(&db, &messages, &media_root)?;
            }
            refresh_identity(main, archive);
            thread_read::read_thread_page(
                archive,
                thread_read::ThreadReadArgs {
                    account,
                    folder: &folder,
                    thread_id: &thread_id,
                    thread_key: &thread_key,
                    subject_filter: subject_filter.as_deref(),
                    limit,
                    for_print: parameters
                        .get("for_print")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    before_cursor: parameters.get("before_cursor").and_then(Value::as_str),
                    media_root,
                },
                None,
            )
            .await
        }

        "messages.markRead" | "messages.markStarred" => {
            let folder = canon_local(parameters);
            let read = command_request.method == "messages.markRead";
            let value = parameters
                .get(if read { "seen" } else { "starred" })
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let db = archive.db.lock().unwrap();
            let explicit = parameters.get("uids").is_some() || parameters.get("uid").is_some();
            let (thread_key, subject_filter) = store::split_thread_key(
                &required_string(parameters, "thread_key").unwrap_or_default(),
            );
            // A whole thread is flagged in every local folder it spans.
            let by_folder = if !explicit && !thread_key.is_empty() {
                store::thread_uids_by_folder(
                    &db,
                    account,
                    &folder,
                    &thread_key,
                    subject_filter.as_deref(),
                    false,
                )?
            } else {
                std::collections::BTreeMap::from([(
                    folder.clone(),
                    request_uids(&db, parameters, &folder)?,
                )])
            };
            for (marked_folder, uids) in &by_folder {
                let (seen, starred) = if read {
                    (Some(value), None)
                } else {
                    (None, Some(value))
                };
                store::set_local_flags(&db, marked_folder, uids, seen, starred)?;
            }
            let mut result = mail_model::mutation_result(
                json!({ "ok": true }),
                &db,
                account,
                &changed_thread_id(parameters, account, &folder),
                &folder,
                None,
                read.then_some(!value),
                (!read).then_some(value),
                false,
            )?;
            mail_model::add_folder_unreads(&mut result, &db, account, by_folder.keys())?;
            Ok(result)
        }

        "tags.list" => {
            let counts = store::tag_counts(&archive.db.lock().unwrap(), Some(account))?;
            Ok(store::tag_counts_json(counts))
        }

        "messages.setTags" => {
            let folder = canon_local(parameters);
            let add = meron_core::tags::parse_tag_list(parameters.get("add"))
                .map_err(anyhow::Error::msg)?;
            let remove = meron_core::tags::parse_tag_list(parameters.get("remove"))
                .map_err(anyhow::Error::msg)?;
            let db = archive.db.lock().unwrap();
            let targets = store::tag_targets(
                &db,
                account,
                &folder,
                &required_string(parameters, "thread_key").unwrap_or_default(),
                &tag_uids(parameters)?,
            )?;
            let tags = store::retag(&db, account, &targets, &add, &remove, false)?;
            Ok(json!({ "ok": true, "tags": tags }))
        }

        "messages.markAllRead" => {
            let folder = canon_local(parameters);
            let db = archive.db.lock().unwrap();
            let updated = store::set_local_folder_seen(&db, &folder, true)?;
            mail_model::mutation_result(
                json!({ "ok": true, "updated": updated, "folder": folder }),
                &db,
                account,
                "",
                &folder,
                None,
                Some(false),
                None,
                false,
            )
        }

        // Local folders have no Trash: a delete is permanent, and the UI says so.
        "messages.delete" => {
            let folder = canon_local(parameters);
            let db = archive.db.lock().unwrap();
            let uids = request_uids(&db, parameters, &folder)?;
            let deleted = store::delete_local_messages(&db, &folder, &uids)?;
            for uid in &uids {
                parse::remove_message_media(&media_root, account, &folder, *uid);
            }
            mail_model::mutation_result(
                json!({ "ok": true, "deleted": deleted, "permanent": true }),
                &db,
                account,
                &changed_thread_id(parameters, account, &folder),
                &folder,
                None,
                None,
                None,
                true,
            )
        }

        "messages.move" | "messages.copy" => {
            let moving = command_request.method == "messages.move";
            let folder = canon_local(parameters);
            let target_account = required_string(parameters, "target_account")
                .unwrap_or_else(|_| account.to_string());
            let target_folder = required_string(parameters, "target_folder")?;
            let thread_id = changed_thread_id(parameters, account, &folder);
            if target_account == account {
                let target_folder = store::normalize_local_folder_name(&target_folder)?;
                let db = archive.db.lock().unwrap();
                let uids = request_uids(&db, parameters, &folder)?;
                if target_folder == folder {
                    return Ok(json!({ "ok": true, "moved": 0 }));
                }
                let target_uids = if moving {
                    store::move_local_messages(&db, &folder, &target_folder, &uids, &media_root)?
                } else {
                    store::copy_local_messages(&db, &folder, &target_folder, &uids, &media_root)?
                };
                if moving {
                    for uid in &uids {
                        parse::remove_message_media(&media_root, account, &folder, *uid);
                    }
                }
                let count = target_uids.len();
                return mail_model::mutation_result(
                    json!({
                        "ok": true,
                        "moved": if moving { count } else { 0 },
                        "copied": if moving { 0 } else { count },
                        "source_folder": folder,
                        "target_account": account,
                        "target_folder": target_folder,
                        "target_uids": target_uids,
                    }),
                    &db,
                    account,
                    &thread_id,
                    &folder,
                    Some(&target_folder),
                    None,
                    None,
                    moving,
                );
            }

            // Back to a server: upload the originals, then forget the local
            // copies only once every APPEND was acknowledged.
            let target_folder = engine::canon_folder(&target_folder);
            let (uids, messages) = {
                let db = archive.db.lock().unwrap();
                let uids = request_uids(&db, parameters, &folder)?;
                let messages = store::raw_messages_for_copy(&db, &folder, &uids)?;
                (uids, messages)
            };
            engine::require_complete_fetch(&uids, &messages)?;
            if messages.is_empty() {
                return Ok(json!({ "ok": true, "moved": 0, "copied": 0 }));
            }
            main.with_write_session(&target_account, |session| {
                let target_folder = target_folder.clone();
                let messages = messages.clone();
                Box::pin(async move {
                    imap::append_copied_messages(session, &target_folder, &messages).await
                })
            })
            .await?;
            let copies = std::collections::BTreeMap::from([(folder.clone(), uids.clone())]);
            let tags = store::carried_tags(&archive.db.lock().unwrap(), account, &copies)?;
            store::apply_carried_tags(&main.db.lock().unwrap(), &target_account, tags, true)?;
            if moving {
                let db = archive.db.lock().unwrap();
                store::delete_local_messages(&db, &folder, &uids)?;
                for uid in &uids {
                    parse::remove_message_media(&media_root, account, &folder, *uid);
                }
            }
            // Best effort: the uploads are done either way.
            if let Some(refresh) = engine::refresh_written_folder(
                main,
                &target_account,
                &target_folder,
                50.max(messages.len() as u32),
            )
            .await
            {
                let batch = &refresh.batch;
                let db = main.db.lock().unwrap();
                let _ = store::ensure_folder(&db, &target_account, &target_folder)
                    .and_then(|_| {
                        store::upsert_window_messages(&db, &target_account, &target_folder, batch)
                    })
                    .and_then(|_| {
                        store::set_folder_state(
                            &db,
                            &target_account,
                            &target_folder,
                            batch.uidvalidity,
                            batch.uid_next,
                        )
                    });
            }
            let count = messages.len();
            mail_model::mutation_result(
                json!({
                    "ok": true,
                    "moved": if moving { count } else { 0 },
                    "copied": if moving { 0 } else { count },
                    "source_folder": folder,
                    "target_account": target_account,
                    "target_folder": target_folder,
                }),
                &archive.db.lock().unwrap(),
                account,
                &thread_id,
                &folder,
                None,
                None,
                None,
                moving,
            )
        }

        other => Err(anyhow::anyhow!("unknown method: {other}")),
    }
}
