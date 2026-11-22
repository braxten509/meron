//! Applies local folder membership changes to the conversations selected by a host.
//! Sender rows are expanded before filing and the prior assignment is retained for undo.
//! Read-state and server mark operations follow the same conversation selection.

use super::*;

/// A sender row stands for every conversation its person has in the row's
/// mailbox: swap it for one id per cached message there, each of which
/// [`resolve_thread`] checks and turns into its conversation. Other ids pass
/// through unchanged.
fn expand_sender_rows(conn: &Connection, ids: Vec<String>) -> Result<Vec<String>> {
    let mut out = Vec::with_capacity(ids.len());
    for thread_id in ids {
        let sender = crate::protocol::parse_thread_id(&thread_id).and_then(|parsed| {
            store::sender_key_addr(&parsed.thread_key).map(|addr| (parsed, addr))
        });
        let Some((parsed, addr)) = sender else {
            out.push(thread_id);
            continue;
        };
        let headers = store::sender_folder_headers(conn, &parsed.account, &parsed.folder, &addr)?;
        out.extend(headers.iter().map(|header| {
            let folder = if header.folder.is_empty() {
                &parsed.folder
            } else {
                &header.folder
            };
            crate::mail_model::format_thread_id(
                &parsed.account,
                folder,
                &format!("uid:{}", header.uid),
            )
        }));
    }
    Ok(out)
}

/// Whether the params say the move is an Undo of an earlier one (`"undo":
/// true`): it then teaches AI sorting nothing.
fn is_undo(params: &Value) -> bool {
    params.get("undo").and_then(Value::as_bool).unwrap_or(false)
}

/// Record what filing `entry` by hand teaches AI sorting (see
/// [`store::ai_lesson_of_entry_in`]). Never fails the filing.
fn teach_filing(conn: &Connection, entry: &str, place: &store::LessonPlace, undo: bool) {
    if let Err(err) = store::ai_lesson_of_entry_in(conn, entry, place, undo) {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.aiSort",
            "could not record a filing's lesson: {err:#}"
        );
    }
}

/// Move whole conversations into a Meron folder (or from one to another), in
/// one transaction. Each change says where the conversation was, so Undo can
/// put it back: `previous_folder_id` null means "out of Meron folders".
pub(super) fn move_in(conn: &Connection, params: &Value) -> Result<(Value, Vec<MarkJob>)> {
    let folder_id = required(params, "folder_id")?;
    anyhow::ensure!(
        store::meron_folder_exists(conn, folder_id)?,
        "That Meron folder is gone."
    );
    let undo = is_undo(params);
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut moved = 0;
    let mut skipped = Vec::new();
    let mut changes = Vec::new();
    let mut touched = BTreeSet::new();
    // Subject branches of one root are one conversation: moved once.
    let mut done: BTreeSet<(String, String)> = BTreeSet::new();
    let mut filed_entries: BTreeSet<String> = BTreeSet::new();
    for thread_id in expand_sender_rows(&tx, thread_ids(params))? {
        let (parsed, key) = match resolve_thread(&tx, &thread_id)? {
            Ok(found) => found,
            Err(reason) => {
                skipped.push(json!({ "thread_id": thread_id, "reason": reason }));
                continue;
            }
        };
        let first = done.insert((parsed.account.clone(), key.clone()));
        let before = store::meron_entry_of_key(&tx, &parsed.account, &key)?;
        // Already filed in this folder (a sender row can reach such mail, from
        // Trash say): nothing moves, so no count and no change for Undo to
        // take back out.
        if before
            .as_ref()
            .is_some_and(|(_, folder)| folder == folder_id)
        {
            continue;
        }
        let result = store::meron_move_in_in(
            &tx,
            &parsed.account,
            &key,
            folder_id,
            store::MeronFiling::User,
        )?;
        filed_entries.insert(result.entry_id.clone());
        // Filed by hand: AI sorting learns where this kind of mail goes (an
        // Undo forgets what the undone action taught). Not for the built-in
        // Archive, which is no sorting target.
        if first && !store::meron_is_builtin(folder_id) {
            let place = store::LessonPlace::Meron(folder_id.to_string());
            teach_filing(&tx, &result.entry_id, &place, undo);
        }
        touched.insert((parsed.account.clone(), parsed.folder.clone()));
        for folder in result.touched {
            touched.insert((parsed.account.clone(), folder));
        }
        // One change per conversation: a later branch would read the folder
        // the first one just moved it into and report no previous folder,
        // which Undo would take as "move it out".
        if first {
            moved += 1;
            changes.push(json!({
                "thread_id": thread_id,
                "account_id": parsed.account,
                "source_folder": parsed.folder,
                "removed": true,
                "previous_folder_id": before.map(|(_, folder)| folder).filter(|folder| folder != folder_id),
            }));
        }
    }
    tx.commit()?;
    let jobs = filed_entries
        .iter()
        .flat_map(|entry| crate::meron_marks::job_for_filed_entry(conn, entry, folder_id))
        .collect();
    Ok((
        json!({
            "ok": true,
            "moved": moved,
            "skipped": skipped,
            "folders": folders_json(conn)?,
            "changes": changes,
            "folder_counts": folder_counts(conn, &touched)?,
        }),
        jobs,
    ))
}

/// Take whole conversations out of their Meron folders: they show where their
/// mail is again.
pub(super) fn move_out(conn: &Connection, params: &Value) -> Result<(Value, Vec<MarkJob>)> {
    let undo = is_undo(params);
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let mut released = 0;
    let mut jobs = Vec::new();
    let mut touched = BTreeSet::new();
    for thread_id in thread_ids(params) {
        let Some(parsed) = crate::protocol::parse_thread_id(&thread_id) else {
            continue;
        };
        touched.insert((parsed.account.clone(), parsed.folder.clone()));
        if let Some((entry, entry_folder)) = store::meron_entry_of_thread(
            &tx,
            &parsed.account,
            &parsed.folder,
            &parsed.thread_key,
            parsed.uid,
        )? {
            // Taken out by hand: AI sorting learns to keep this kind of mail
            // in the Inbox (an Undo forgets what the undone filing taught).
            if !store::meron_is_builtin(&entry_folder) {
                teach_filing(&tx, &entry, &store::LessonPlace::Inbox, undo);
            }
            jobs.extend(crate::meron_marks::job_for_released_entry(&tx, &entry));
            let result = store::meron_release_entry_in(&tx, &entry)?;
            released += result.released;
            touched.extend(result.touched);
        }
    }
    tx.commit()?;
    Ok((
        json!({
            "ok": true,
            "released": released,
            "folders": folders_json(conn)?,
            "folder_counts": folder_counts(conn, &touched)?,
        }),
        jobs,
    ))
}

/// "Mark all read" inside a Meron folder: `\Seen` on the servers for every
/// unseen message of its conversations (outside Drafts, Trash and Junk), then
/// the store. Per mailbox; one that fails is reported and the rest go on.
pub async fn mark_all_read(engine: &Arc<Engine>, params: &Value) -> Result<Value> {
    let folder_id = required(params, "folder_id")?.to_string();
    let targets: BTreeMap<String, BTreeMap<String, Vec<u32>>> = {
        let db = engine.db.lock().unwrap();
        anyhow::ensure!(
            store::meron_folder_exists(&db, &folder_id)?,
            "That Meron folder is gone."
        );
        store::meron_folder_unseen_uids(&db, &folder_id)?
    };
    let mut updated = 0;
    let mut failures = Vec::new();
    let mut touched = BTreeSet::new();
    for (account, folders) in targets {
        for (folder, uids) in folders {
            let write = engine
                .with_flag_write_session(
                    &account,
                    |session| {
                        let folder = folder.clone();
                        Box::pin(
                            async move { crate::imap::prepare_flag_update(session, &folder).await },
                        )
                    },
                    |session| {
                        let uids = uids.clone();
                        Box::pin(async move { crate::imap::store_seen(session, &uids, true).await })
                    },
                )
                .await;
            match write {
                Ok(_) => {
                    store::meron_mark_seen(&engine.db.lock().unwrap(), &account, &folder, &uids)?;
                    updated += uids.len();
                    touched.insert((account.clone(), folder.clone()));
                }
                Err(err) => failures.push(json!({
                    "account_id": account,
                    "folder_id": folder,
                    "message": format!("{err:#}"),
                })),
            }
        }
    }
    let db = engine.db.lock().unwrap();
    Ok(json!({
        "ok": failures.is_empty(),
        "updated": updated,
        "failures": failures,
        "folders": folders_json(&db)?,
        "folder_counts": folder_counts(&db, &touched)?,
    }))
}

/// Save the local-folder assignment needed to undo a conversation action.
///
/// Before a whole-conversation Archive, Move to or Delete: the Meron-folder card the conversation is, if
/// any, to [`release_after_action`] once the server step succeeded. From any folder, Drafts, Trash and Junk
/// included (owner decision 9.5: the action makes the conversation leave). Only that card's conversation
/// leaves: an entry holding more conversations keeps the others.
///
/// `None` for a single email, a sender row, the local folders, or an account with nothing in Meron folders.
pub fn entry_before_action(
    conn: &Connection,
    account: &str,
    folder: &str,
    root: &str,
    uid: Option<u32>,
    whole_conversation: bool,
) -> Option<store::MeronCardRelease> {
    if !whole_conversation
        || root.is_empty()
        || store::sender_key_addr(root).is_some()
        || store::is_local_account(account)
        || !store::meron_account_has_entries(conn, account).unwrap_or(false)
    {
        return None;
    }
    store::meron_card_release(conn, account, folder, root, uid)
        .ok()
        .flatten()
}

/// Before the server step of an action [`entry_before_action`] found a card for: take the
/// Meron-folder marks off the card's Inbox copies, so the mail doesn't carry them to its new folder
/// and other devices take it out too. Awaited (within a few seconds) so the marks go before the
/// mail moves; best effort.
pub async fn unmark_before_action(
    engine: &Arc<Engine>,
    account: &str,
    card: &Option<store::MeronCardRelease>,
) {
    let Some(card) = card else {
        return;
    };
    let rows: Vec<(String, u32)> = card.rows.iter().cloned().collect();
    let uids = crate::meron_marks::uids_marked(&engine.db.lock().unwrap(), account, &rows);
    crate::meron_marks::write_all(engine, crate::meron_marks::jobs_for(account, uids, None)).await;
}

/// [`unmark_before_action`] for the phone's blocking commands.
pub fn unmark_before_action_blocking(
    engine: &Arc<Engine>,
    account: &str,
    card: &Option<store::MeronCardRelease>,
) {
    if card.is_none() {
        return;
    }
    let _ = crate::ffi::engine_block_on(async {
        unmark_before_action(engine, account, card).await;
        anyhow::Ok(())
    });
}
