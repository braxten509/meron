//! Coordinates the built-in Archive folder with server-side mailbox moves.
//! Message identity connects the copied mail to its local archive entry.
//! Undo uses the recorded source and destination to move only the affected conversations.

use super::*;

/// Before archiving (a move into the account's archive mailbox): the conversation keys of the moved
/// copies, read before the move, when the action files them into the built-in Archive. `None` when
/// it doesn't: not a whole conversation (same rule as [`entry_before_action`]), a sender row, a
/// local or feed account, or nothing cached.
pub fn archive_filing_keys(
    conn: &Connection,
    account: &str,
    folder: &str,
    root: &str,
    uids: &[u32],
    whole_conversation: bool,
) -> Option<BTreeSet<String>> {
    if !whole_conversation
        || uids.is_empty()
        || root.is_empty()
        || store::sender_key_addr(root).is_some()
        || store::is_local_account(account)
        || store::meron_archive_mailbox(conn, account).as_deref() == Some(folder)
    {
        return None;
    }
    match account_engine(conn, account).ok().flatten().as_deref() {
        None | Some("rss") | Some("local") => return None,
        Some(_) => {}
    }
    let keys: BTreeSet<String> = uids
        .iter()
        .filter_map(|uid| {
            store::meron_conversation_key(conn, account, folder, *uid)
                .ok()
                .flatten()
        })
        .collect();
    (!keys.is_empty()).then_some(keys)
}

/// Whether `target` is where archiving puts mail: an archive-role mailbox.
pub fn is_archive_target(conn: &Connection, account: &str, target: &str) -> bool {
    !store::is_local_account(account)
        && (store::folder_role(conn, account, target).is_ok_and(|role| role == "archive")
            || store::meron_archive_mailbox(conn, account).as_deref() == Some(target))
}

/// The built-in Archive card the copies `uids` of archive mailbox `folder`
/// belong to, read before exactly these copies move out (an Undo of an
/// archive): the conversation leaves Archive with them.
pub fn archive_card_of_copies(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
) -> Option<store::MeronCardRelease> {
    if !is_archive_target(conn, account, folder) {
        return None;
    }
    let first = *uids.first()?;
    store::meron_card_release(conn, account, folder, "", Some(first))
        .ok()
        .flatten()
        .filter(|card| card.folder_id == store::MERON_ARCHIVE_FOLDER_ID)
}

/// Coordinate local-folder changes with server-side archive moves.
///
/// `moveIn` and `moveOut`, which touch the servers for the built-in Archive (the desktop's
/// `meronFolders.moveIn`/`.moveOut`, the phone's `mail.moveToMeronFolder`/`mail.moveOutOfMeronFolder`):
///
/// - `moveOut` of a conversation in Archive un-archives it: its archived
///   copies lose the Archive's mark and move back to the Inbox, and it leaves
///   Archive. The answer's `unarchived` names it in the Inbox (`thread_id`,
///   `account_id`), for an Undo. Other ids go out store-only, as before.
/// - `moveIn` into Archive (the Undo of that) archives again: the cached Inbox
///   copies move to the archive mailbox and the conversation is filed. With
///   no Inbox copy cached, it is filed store-only.
/// - Every other `moveIn` is store-only, as before.
///
/// The store lock is only held between the server steps.
pub async fn move_with_server(
    engine: &Arc<Engine>,
    command: &str,
    params: &Value,
) -> Result<Value> {
    let result = match command {
        "moveOut" => move_out_with_server(engine, params).await,
        "moveIn" if string_parameter(params, "folder_id") == store::MERON_ARCHIVE_FOLDER_ID => {
            archive_again(engine, params).await
        }
        _ => {
            let (result, jobs) = dispatch_with_marks(&engine.db.lock().unwrap(), command, params)?;
            crate::meron_marks::spawn_writes(engine, jobs);
            Ok(result)
        }
    };
    // What the filing taught AI sorting travels to the reader's other devices.
    crate::engine::ai_sort::push_lesson_tags(engine);
    result
}

/// Archive of a whole conversation on an account with no archive mailbox: Meron-only.
///
/// No server move; the conversation is filed into the built-in Archive here (hidden from the Inbox
/// view, its mail staying where it is), and its Inbox copies get the Archive's mark for the other
/// devices (the returned jobs, for the caller to start).
///
/// `None` when this doesn't apply: a single email (a `uid:` id), a sender row, a feed or local
/// account; the caller keeps its "no archive folder" answer then.
///
/// The answer has `moved`, `meron_only: true`, `folder` and `thread_id` (unchanged: the mail didn't
/// move) and, when it came from another Meron folder, `meron_folder_id` and `meron_source` for an
/// Undo to put it back there.
pub fn archive_meron_only(
    conn: &Connection,
    thread_id: &str,
) -> Result<Option<(Value, Vec<MarkJob>)>> {
    let Some(parsed) = crate::protocol::parse_thread_id(thread_id) else {
        return Ok(None);
    };
    if parsed.uid.is_some() || parsed.thread_key.starts_with("uid:") {
        return Ok(None);
    }
    if resolve_thread(conn, thread_id)?.is_err() {
        return Ok(None);
    }
    let previous = store::meron_entry_of_thread(
        conn,
        &parsed.account,
        &parsed.folder,
        &parsed.thread_key,
        None,
    )?
    .map(|(entry, folder)| {
        let source: String = conn
            .query_row(
                "SELECT source FROM meron_folder_entries WHERE id = ?1",
                params![entry],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "user".to_string());
        (folder, source)
    })
    .filter(|(folder, _)| folder != store::MERON_ARCHIVE_FOLDER_ID);
    let (mut result, jobs) = move_in(
        conn,
        &json!({ "folder_id": store::MERON_ARCHIVE_FOLDER_ID, "thread_ids": [thread_id] }),
    )?;
    if let Some(object) = result.as_object_mut() {
        object.insert("meron_only".to_string(), json!(true));
        object.insert("folder".to_string(), json!(parsed.folder));
        object.insert("thread_id".to_string(), json!(thread_id));
        if let Some((folder, source)) = previous {
            object.insert("meron_folder_id".to_string(), json!(folder));
            object.insert("meron_source".to_string(), json!(source));
        }
    }
    Ok(Some((result, jobs)))
}

async fn move_out_with_server(engine: &Arc<Engine>, params: &Value) -> Result<Value> {
    // Which ids are Archive's: read once, before anything moves.
    let mut plain: Vec<String> = Vec::new();
    let mut unarchive: Vec<Unarchive> = Vec::new();
    {
        let db = engine.db.lock().unwrap();
        for thread_id in thread_ids(params) {
            let Some(parsed) = crate::protocol::parse_thread_id(&thread_id) else {
                plain.push(thread_id);
                continue;
            };
            let in_archive = store::meron_entry_of_thread(
                &db,
                &parsed.account,
                &parsed.folder,
                &parsed.thread_key,
                parsed.uid,
            )?
            .is_some_and(|(_, folder)| folder == store::MERON_ARCHIVE_FOLDER_ID);
            let card = in_archive
                .then(|| {
                    store::meron_card_release(
                        &db,
                        &parsed.account,
                        &parsed.folder,
                        &parsed.thread_key,
                        parsed.uid,
                    )
                })
                .transpose()?
                .flatten();
            let Some(card) = card else {
                plain.push(thread_id);
                continue;
            };
            if unarchive
                .iter()
                .any(|known| known.card.entry_id == card.entry_id && known.card.rows == card.rows)
            {
                continue;
            }
            let archive_folder = store::meron_archive_mailbox(&db, &parsed.account);
            let uids: Vec<u32> = card
                .rows
                .iter()
                .filter(|(folder, _)| archive_folder.as_deref() == Some(folder.as_str()))
                .map(|(_, uid)| *uid)
                .collect();
            unarchive.push(Unarchive {
                account: parsed.account.clone(),
                card,
                archive_folder,
                uids,
            });
        }
    }
    let mut released = 0;
    let mut touched: BTreeSet<(String, String)> = BTreeSet::new();
    let mut unarchived = Vec::new();
    let mut failures = Vec::new();
    for item in unarchive {
        let mut inbox_uids: Vec<u32> = Vec::new();
        // Nothing in the archive mailbox (archived Meron-only, on an account
        // with no archive mailbox): no server move. Its marked copies (in the
        // Inbox) lose the Archive's mark, and it leaves Archive where it is.
        if item.uids.is_empty() {
            let rows: Vec<(String, u32)> = item.card.rows.iter().cloned().collect();
            let marked =
                crate::meron_marks::uids_marked(&engine.db.lock().unwrap(), &item.account, &rows);
            crate::meron_marks::write_all(
                engine,
                crate::meron_marks::jobs_for(&item.account, marked, None),
            )
            .await;
            inbox_uids = item
                .card
                .rows
                .iter()
                .filter(|(folder, _)| folder.eq_ignore_ascii_case("INBOX"))
                .map(|(_, uid)| *uid)
                .collect();
        }
        if let Some(archive) = item
            .archive_folder
            .as_deref()
            .filter(|_| !item.uids.is_empty())
        {
            // The Archive's mark first, so the copies don't carry it back.
            crate::meron_marks::write_all(
                engine,
                vec![MarkJob {
                    account: item.account.clone(),
                    folder: archive.to_string(),
                    uids: item.uids.clone(),
                    target: None,
                }],
            )
            .await;
            match crate::engine::move_within_account(
                engine,
                &item.account,
                archive,
                "INBOX",
                &item.uids,
            )
            .await
            {
                Ok(copies) => {
                    inbox_uids = copies.uids;
                    touched.insert((item.account.clone(), archive.to_string()));
                    touched.insert((item.account.clone(), "INBOX".to_string()));
                }
                Err(err) => {
                    // Still archived: it keeps its place and its mark.
                    crate::meron_marks::spawn_writes(
                        engine,
                        vec![MarkJob {
                            account: item.account.clone(),
                            folder: archive.to_string(),
                            uids: item.uids.clone(),
                            target: Some(store::MERON_ARCHIVE_FOLDER_ID.to_string()),
                        }],
                    );
                    failures.push(json!({
                        "account_id": item.account,
                        "message": format!("{err:#}"),
                    }));
                    continue;
                }
            }
        }
        let db = engine.db.lock().unwrap();
        let result = store::meron_release_card(&db, &item.card)?;
        released += result.released.max(1);
        touched.extend(result.touched);
        // Named by its first copy back in the Inbox, for an Undo.
        let inbox_thread = inbox_uids.first().and_then(|uid| {
            store::meron_row_header(&db, &item.account, "INBOX", *uid)
                .ok()
                .flatten()
                .map(|header| {
                    crate::mail_model::format_thread_id(&item.account, "INBOX", &header.thread_key)
                })
        });
        if let Some(thread_id) = inbox_thread {
            unarchived.push(json!({
                "thread_id": thread_id,
                "account_id": item.account,
                "target_uids": inbox_uids,
            }));
        }
    }
    crate::meron_marks::flush_archive_unmarks(engine);
    // The rest leave their Meron folders store-only, as always.
    let mut plain_released = 0;
    if !plain.is_empty() {
        let (result, jobs) = move_out(&engine.db.lock().unwrap(), &json!({ "thread_ids": plain }))?;
        crate::meron_marks::spawn_writes(engine, jobs);
        plain_released = result.get("released").and_then(Value::as_u64).unwrap_or(0) as usize;
        if let Some(counts) = result.get("folder_counts").and_then(Value::as_array) {
            for count in counts {
                if let (Some(account), Some(folder)) = (
                    count.get("account_id").and_then(Value::as_str),
                    count.get("folder_id").and_then(Value::as_str),
                ) {
                    touched.insert((account.to_string(), folder.to_string()));
                }
            }
        }
    }
    let db = engine.db.lock().unwrap();
    Ok(json!({
        "ok": failures.is_empty(),
        "released": released + plain_released,
        "unarchived": unarchived,
        "failures": failures,
        "folders": folders_json(&db)?,
        "folder_counts": folder_counts(&db, &touched)?,
    }))
}

/// `moveIn` into the built-in Archive: archive the conversations again (the
/// Undo of "Move to Inbox").
async fn archive_again(engine: &Arc<Engine>, params: &Value) -> Result<Value> {
    struct Job {
        thread_id: String,
        account: String,
        root: String,
        inbox_uids: Vec<u32>,
        archive_folder: String,
        keys: BTreeSet<String>,
        card: Option<store::MeronCardRelease>,
    }
    let mut jobs: Vec<Job> = Vec::new();
    let mut store_only: Vec<String> = Vec::new();
    let mut skipped = Vec::new();
    {
        let db = engine.db.lock().unwrap();
        for thread_id in thread_ids(params) {
            let (parsed, key) = match resolve_thread(&db, &thread_id)? {
                Ok(found) => found,
                Err(reason) => {
                    skipped.push(json!({ "thread_id": thread_id, "reason": reason }));
                    continue;
                }
            };
            // No archive mailbox: archiving is Meron-only, the mail stays
            // where it is and is filed (and marked) here.
            let Some(archive_folder) = store::meron_archive_mailbox(&db, &parsed.account) else {
                store_only.push(thread_id);
                continue;
            };
            let inbox_uids = if parsed.folder.eq_ignore_ascii_case("INBOX") {
                match parsed.uid {
                    Some(uid) => vec![uid],
                    None => store::resolve_message_uids(
                        &db,
                        &parsed.account,
                        "INBOX",
                        &parsed.thread_key,
                        parsed.subject_filter.as_deref(),
                        None,
                        &[],
                    )?,
                }
            } else {
                Vec::new()
            };
            if inbox_uids.is_empty() {
                store_only.push(thread_id);
                continue;
            }
            let mut keys = archive_filing_keys(
                &db,
                &parsed.account,
                "INBOX",
                &parsed.thread_key,
                &inbox_uids,
                true,
            )
            .unwrap_or_default();
            keys.insert(key);
            let card = entry_before_action(
                &db,
                &parsed.account,
                "INBOX",
                &parsed.thread_key,
                parsed.uid,
                true,
            );
            jobs.push(Job {
                thread_id,
                account: parsed.account,
                root: parsed.thread_key,
                inbox_uids,
                archive_folder,
                keys,
                card,
            });
        }
    }
    let mut moved = 0;
    let mut changes = Vec::new();
    let mut touched: BTreeSet<(String, String)> = BTreeSet::new();
    let mut failures = Vec::new();
    for job in jobs {
        prepare_archive_marks(engine, &job.account, &job.card, "INBOX", &job.inbox_uids).await;
        let copies = match crate::engine::move_within_account(
            engine,
            &job.account,
            "INBOX",
            &job.archive_folder,
            &job.inbox_uids,
        )
        .await
        {
            Ok(copies) => copies,
            Err(err) => {
                restore_marks_after_failed_archive(
                    engine,
                    &engine.db.lock().unwrap(),
                    &job.account,
                    &job.card,
                    "INBOX",
                    &job.inbox_uids,
                );
                failures.push(json!({ "thread_id": job.thread_id, "message": format!("{err:#}") }));
                continue;
            }
        };
        let db = engine.db.lock().unwrap();
        release_after_action(&db, job.card);
        touched.extend(file_after_archive(
            &db,
            &job.account,
            &job.archive_folder,
            &copies.uids,
            &job.keys,
        ));
        touched.insert((job.account.clone(), "INBOX".to_string()));
        touched.insert((job.account.clone(), job.archive_folder.clone()));
        moved += 1;
        let archived_thread = copies.uids.first().and_then(|uid| {
            store::meron_row_header(&db, &job.account, &job.archive_folder, *uid)
                .ok()
                .flatten()
                .map(|header| header.thread_key)
        });
        changes.push(json!({
            "thread_id": job.thread_id,
            "account_id": job.account,
            "source_folder": "INBOX",
            "removed": true,
            "previous_folder_id": Value::Null,
            "archived_thread_id": crate::mail_model::format_thread_id(
                &job.account,
                &job.archive_folder,
                archived_thread.as_deref().unwrap_or(&job.root),
            ),
        }));
    }
    crate::meron_marks::flush_archive_unmarks(engine);
    // No Inbox copy cached: filed store-only, its archived copies marked.
    if !store_only.is_empty() {
        let (result, mark_jobs) = move_in(
            &engine.db.lock().unwrap(),
            &json!({ "folder_id": store::MERON_ARCHIVE_FOLDER_ID, "thread_ids": store_only }),
        )?;
        crate::meron_marks::spawn_writes(engine, mark_jobs);
        moved += result.get("moved").and_then(Value::as_u64).unwrap_or(0) as usize;
        if let Some(more) = result.get("changes").and_then(Value::as_array) {
            changes.extend(more.iter().cloned());
        }
        if let Some(more) = result.get("skipped").and_then(Value::as_array) {
            skipped.extend(more.iter().cloned());
        }
        if let Some(counts) = result.get("folder_counts").and_then(Value::as_array) {
            for count in counts {
                if let (Some(account), Some(folder)) = (
                    count.get("account_id").and_then(Value::as_str),
                    count.get("folder_id").and_then(Value::as_str),
                ) {
                    touched.insert((account.to_string(), folder.to_string()));
                }
            }
        }
    }
    let db = engine.db.lock().unwrap();
    Ok(json!({
        "ok": failures.is_empty(),
        "moved": moved,
        "skipped": skipped,
        "failures": failures,
        "folders": folders_json(&db)?,
        "changes": changes,
        "folder_counts": folder_counts(&db, &touched)?,
    }))
}
