//! Handles message mutations dispatched by the desktop sidecar.
//! The requested action determines how the engine changes the selected mail.
//! The reply carries the mutation result used by the desktop conversation list.

mod transfers;

use serde_json::{Value, json};
use std::sync::Arc;

use meron_core::engine::Engine;
use meron_core::engine::*;
use meron_core::protocol::Request;
use meron_core::tags::{parse_tag_list, parse_tag_replacement, renamed_queries};
use meron_core::{imap, mail_model, store};

use crate::Writer;
use crate::sidecar::params::*;

/// Handle message flag and placement changes: read, starred, delete, move, copy.
pub(crate) async fn dispatch(
    engine: &Arc<Engine>,
    command_request: &Request,
    out: &Writer,
) -> anyhow::Result<Value> {
    let parameters = &command_request.params;
    match command_request.method.as_str() {
        "messages.markRead" => {
            let account = required_string(parameters, "account")?;
            let folder = canon_folder(
                &required_string(parameters, "folder").unwrap_or_else(|_| "INBOX".to_string()),
            );
            let (thread_key, subject_filter) = store::split_thread_key(
                &required_string(parameters, "thread_key").unwrap_or_default(),
            );
            let uid = optional_message_uid(parameters)?;
            // Defaults to true (mark read); pass seen:false to mark unread.
            let seen = parameters
                .get("seen")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            let explicit_uids = explicit_message_uids(parameters)?;

            // Marking a whole thread read reaches every folder it spans and only
            // the messages still unread; everything else acts on this folder.
            let whole_thread_read = explicit_uids.is_empty() && !thread_key.is_empty() && seen;
            let by_folder = if whole_thread_read {
                let db = engine.db.lock().unwrap();
                // The store is only updated after the server write below, so
                // skipping messages it already has read is safe to retry.
                store::thread_uids_by_folder(
                    &db,
                    &account,
                    &folder,
                    &thread_key,
                    subject_filter.as_deref(),
                    true,
                )?
            } else {
                let uids = if !explicit_uids.is_empty() {
                    explicit_uids
                } else if thread_key.is_empty() {
                    uid.into_iter().collect::<Vec<_>>()
                } else {
                    // Marking a whole thread unread flags its newest message only.
                    let db = engine.db.lock().unwrap();
                    store::newest_thread_uids(
                        &db,
                        &account,
                        &folder,
                        &thread_key,
                        subject_filter.as_deref(),
                    )?
                };
                std::collections::BTreeMap::from([(folder.clone(), uids)])
            };

            engine
                .store_seen_by_folder(&account, &by_folder, seen)
                .await?;

            {
                let db = engine.db.lock().unwrap();
                // Per uid: a whole-thread update would flip sibling subject
                // branches sharing the root thread_key, and marking unread only
                // flagged the newest message.
                for (marked_folder, uids) in &by_folder {
                    for marked_uid in uids {
                        store::update_message_seen(
                            &db,
                            &account,
                            marked_folder,
                            *marked_uid,
                            seen,
                        )?;
                    }
                }
                if whole_thread_read && subject_filter.is_none() {
                    store::update_thread_seen(&db, &account, &folder, &thread_key, seen)?;
                }
            }
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
            let db = engine.db.lock().unwrap();
            let mut result = mail_model::mutation_result(
                json!({ "ok": true }),
                &db,
                &account,
                &changed_thread_id,
                &folder,
                None,
                Some(!seen),
                None,
                false,
            )?;
            mail_model::add_folder_unreads(&mut result, &db, &account, by_folder.keys())?;
            Ok(result)
        }

        "messages.markStarred" => {
            let account = required_string(parameters, "account")?;
            let folder = canon_folder(
                &required_string(parameters, "folder").unwrap_or_else(|_| "INBOX".to_string()),
            );
            let full_key = required_string(parameters, "thread_key").unwrap_or_default();
            let (thread_key, subject_filter) = store::split_thread_key(&full_key);
            let uid = optional_message_uid(parameters)?;
            // Defaults to true (mark starred); pass starred:false to unstar.
            let starred = parameters
                .get("starred")
                .and_then(Value::as_bool)
                .unwrap_or(true);

            let explicit_uids = explicit_message_uids(parameters)?;

            let targets = {
                let db = engine.db.lock().unwrap();
                let message_scoped = parameters.get("uids").is_some() || full_key.is_empty();
                let uids = if parameters.get("uids").is_some() {
                    explicit_uids
                } else {
                    uid.into_iter().collect()
                };
                store::starred_mutation_targets(
                    &db,
                    &account,
                    &folder,
                    (!message_scoped).then_some(full_key.as_str()),
                    &uids,
                    starred,
                )?
            };
            mark_starred_copies(&engine, &account, &targets, starred).await?;
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
            mail_model::mutation_result(
                json!({ "ok": true }),
                &engine.db.lock().unwrap(),
                &account,
                &changed_thread_id,
                &folder,
                None,
                None,
                Some(starred),
                false,
            )
        }

        "messages.setTags" => {
            let account = required_string(parameters, "account")?;
            let folder = canon_folder(
                &required_string(parameters, "folder").unwrap_or_else(|_| "INBOX".to_string()),
            );
            let add = parse_tag_list(parameters.get("add")).map_err(anyhow::Error::msg)?;
            let remove = parse_tag_list(parameters.get("remove")).map_err(anyhow::Error::msg)?;
            let tags = {
                let db = engine.db.lock().unwrap();
                let targets = store::tag_targets(
                    &db,
                    &account,
                    &folder,
                    &required_string(parameters, "thread_key").unwrap_or_default(),
                    &tag_uids(parameters)?,
                )?;
                store::retag(&db, &account, &targets, &add, &remove, true)?
            };
            spawn_tag_push(&engine, &account);
            Ok(json!({ "ok": true, "tags": tags }))
        }

        // Rename a tag everywhere, or delete it. The archive goes first, as
        // opening it reads the store.
        "tags.replace" => {
            let (tag, to) = parse_tag_replacement(parameters).map_err(anyhow::Error::msg)?;
            crate::sidecar::local::replace_tag(engine, &tag, to.as_deref())?;
            let (accounts, rules) =
                store::replace_tag_and_rules(&engine.db.lock().unwrap(), &tag, to.as_deref())?;
            for account in &accounts {
                spawn_tag_push(&engine, account);
            }
            let queries = renamed_queries(parameters, &tag, to.as_deref());
            Ok(json!({ "ok": true, "tag": to, "rules": rules, "queries": queries }))
        }

        "messages.delete" => {
            let account = required_string(parameters, "account")?;
            let folder = canon_folder(
                &required_string(parameters, "folder").unwrap_or_else(|_| "INBOX".to_string()),
            );
            let (thread_key, subject_filter) = store::split_thread_key(
                &required_string(parameters, "thread_key").unwrap_or_default(),
            );
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
            // Deleting a sender row also takes their older mail in this folder
            // that the cache never held.
            let uids = match store::sender_key_addr(&thread_key) {
                Some(addr) if explicit_uids.is_empty() => {
                    sender_delete_uids(engine, &account, &folder, &addr, uids).await?
                }
                _ => uids,
            };

            if uids.is_empty() {
                return mail_model::mutation_result(
                    json!({ "ok": true, "deleted": 0 }),
                    &engine.db.lock().unwrap(),
                    &account,
                    &changed_thread_id,
                    &folder,
                    None,
                    None,
                    None,
                    false,
                );
            }

            let cached_ids =
                store::cached_message_ids(&engine.db.lock().unwrap(), &account, &folder, &uids)?;
            // Deleting the whole conversation takes it out of its Meron folder.
            let meron_entry = meron_core::meron_folders::entry_before_action(
                &engine.db.lock().unwrap(),
                &account,
                &folder,
                &thread_key,
                uid.filter(|_| thread_key.starts_with("uid:")),
                explicit_uids.is_empty() && (uid.is_none() || thread_key.starts_with("uid:")),
            );
            let moved_ids = moved_message_ids(engine, &account, &folder, &uids, cached_ids).await;
            meron_core::meron_folders::unmark_before_action(engine, &account, &meron_entry).await;
            // Mutating, so it never auto-retries.
            let trashed = delete_to_trash(engine, &account, &folder, &uids).await?;
            // One of a bulk run with more to come: Trash is re-read once, by
            // the run's last delete, instead of after every conversation.
            let more_coming = parameters
                .get("more_coming")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let meron_released = meron_core::meron_folders::release_after_action(
                &engine.db.lock().unwrap(),
                meron_entry,
            );
            // Undo moves exactly these copies back out of Trash.
            let copies = match &trashed {
                Some(trash) => {
                    let copies = refresh_moved_copies_with(
                        engine,
                        &account,
                        &trash.folder,
                        uids.len(),
                        trash.uid_next_before,
                        moved_ids.as_deref(),
                        !more_coming,
                    )
                    .await;
                    copies.store(&engine.db.lock().unwrap(), &account, &trash.folder)?;
                    Some(copies)
                }
                None => None,
            };
            let trash_folder = trashed.as_ref().map(|trash| trash.folder.as_str());

            {
                let db = engine.db.lock().unwrap();
                // Discarding a draft must also drop hidden local copies sharing
                // its Message-ID (stale autosaves the pane deduped away), or the
                // thread card keeps its has_draft badge until the next full sync.
                if store::folder_role(&db, &account, &folder)? == "drafts" {
                    store::delete_draft_sibling_copies(&db, &account, &folder, &uids)?;
                }
                store::delete_messages_by_uid(&db, &account, &folder, &uids)?;
                // The whole conversation went: it starts fresh when new mail
                // arrives (see `store::ChatScope`).
                if explicit_uids.is_empty()
                    && uid.is_none()
                    && store::leave_replies_out_after_delete(
                        &db,
                        &account,
                        &folder,
                        &thread_key,
                        subject_filter.as_deref(),
                        trash_folder
                            .zip(copies.as_ref())
                            .map(|(trash, copies)| (trash, copies.uids.as_slice())),
                    )
                {
                    spawn_tag_push(engine, &account);
                }
            }
            // The server delete/move-to-Trash completed for every resolved UID.
            // A concurrent source refresh may already have pruned the cache rows.
            let deleted = uids.len();
            let result = match (trash_folder, &copies) {
                (Some(trash), Some(copies)) => json!({
                    "ok": true,
                    "deleted": deleted,
                    "trash": trash,
                    "target_uids": copies.uids,
                }),
                _ => json!({ "ok": true, "deleted": deleted, "permanent": true }),
            };
            let mut result = mail_model::mutation_result(
                result,
                &engine.db.lock().unwrap(),
                &account,
                &changed_thread_id,
                &folder,
                trash_folder,
                None,
                None,
                true,
            )?;
            // An Undo puts the card back in its Meron folder.
            meron_core::meron_folders::ReleasedCard::annotate(&meron_released, &mut result);
            Ok(result)
        }

        "messages.move" => transfers::move_messages(engine, parameters).await,

        "messages.copy" => transfers::copy_messages(engine, parameters).await,

        // Mark every message in a folder as read: set \Seen on the server for the
        // currently-unseen UIDs, then flip the whole folder seen in the store.
        "messages.markAllRead" => {
            let account = required_string(parameters, "account")?;
            let folder = canon_folder(
                &required_string(parameters, "folder").unwrap_or_else(|_| "INBOX".to_string()),
            );

            // Mail in Meron folders stays unread: each has its own Mark all read.
            let uids = {
                let db = engine.db.lock().unwrap();
                store::get_visible_unseen_uids(&db, &account, &folder)?
            };

            if !uids.is_empty() {
                engine
                    .with_flag_write_session(
                        &account,
                        |session| {
                            let folder = folder.clone();
                            Box::pin(
                                async move { imap::prepare_flag_update(session, &folder).await },
                            )
                        },
                        |session| {
                            let uids = uids.clone();
                            Box::pin(async move { imap::store_seen(session, &uids, true).await })
                        },
                    )
                    .await?;
            }

            {
                let db = engine.db.lock().unwrap();
                store::mark_visible_folder_seen(&db, &account, &folder)?;
            }
            // The Inbox also lists mail the AI left in other folders: mark it
            // read where each message really is.
            let mut updated = uids.len();
            if folder.eq_ignore_ascii_case("INBOX") {
                let shown = store::ai_sort_inbox_unseen(&engine.db.lock().unwrap(), &account)?;
                for (shown_folder, shown_uids) in shown {
                    engine
                        .with_flag_write_session(
                            &account,
                            |session| {
                                let folder = shown_folder.clone();
                                Box::pin(async move {
                                    imap::prepare_flag_update(session, &folder).await
                                })
                            },
                            |session| {
                                let uids = shown_uids.clone();
                                Box::pin(
                                    async move { imap::store_seen(session, &uids, true).await },
                                )
                            },
                        )
                        .await?;
                    store::meron_mark_seen(
                        &engine.db.lock().unwrap(),
                        &account,
                        &shown_folder,
                        &shown_uids,
                    )?;
                    updated += shown_uids.len();
                }
            }
            mail_model::mutation_result(
                json!({ "ok": true, "updated": updated, "folder": folder }),
                &engine.db.lock().unwrap(),
                &account,
                "",
                &folder,
                None,
                Some(false),
                None,
                false,
            )
        }

        "messages.markAllReadUnified" => {
            let role =
                required_string(parameters, "folder").unwrap_or_else(|_| "inbox".to_string());
            let account_folders = {
                let db = engine.db.lock().unwrap();
                store::list_accounts(&db)?
                    .into_iter()
                    .filter(|account| {
                        account
                            .get("included_in_unified")
                            .and_then(Value::as_bool)
                            .unwrap_or(true)
                    })
                    .filter(|account| {
                        account.get("auth_type").and_then(Value::as_str) != Some("rss")
                    })
                    .filter_map(|account| {
                        account
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .map(|account| {
                        let folder = store::folder_for_role(&db, &account, &role)?;
                        Ok(folder.map(|folder| (account, folder)))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            };
            let mut updated = 0_u64;
            let mut failures = Vec::new();
            let mut folder_unreads = serde_json::Map::new();
            let mut folder_counts = Vec::new();
            for (account, folder) in account_folders {
                let request = Request {
                    id: command_request.id,
                    method: "messages.markAllRead".to_string(),
                    params: json!({ "account": account, "folder": folder }),
                };
                match Box::pin(dispatch(engine, &request, out)).await {
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
                    Err(err) => failures
                        .push(json!({ "account_id": account, "message": format!("{err:#}") })),
                }
            }
            Ok(json!({
                "ok": failures.is_empty(),
                "updated": updated,
                "failures": failures,
                "folder_unreads": folder_unreads,
                "folder_counts": folder_counts,
            }))
        }

        other => Err(anyhow::anyhow!("unknown method: {other}")),
    }
}
