//! Restores mail from a recorded sorting decision.
//! The history action resolves the stored location and schedules its reversal.
//! Items filed in a Meron folder also need their local membership updated.

use super::*;

/// Put an email the AI moved back in the Inbox. It isn't sorted again: it is
/// already observed (not new), decided on this device, and carries the mark.
pub async fn move_back(engine: &Arc<Engine>, id: i64) -> anyhow::Result<()> {
    let entry = store::ai_sort_log_entry(&engine.db.lock().unwrap(), id)?
        .ok_or_else(|| anyhow::anyhow!("That entry is gone from the log."))?;
    anyhow::ensure!(entry.state == "moved", "This email wasn't moved by the AI.");
    if !entry.meron_folder_id.is_empty() {
        return move_back_from_meron(engine, id, &entry);
    }
    let target = entry.to_folder.clone();
    let shown = crate::utf7::decode(&target);
    let not_found = || anyhow::anyhow!("Couldn't find it; look in {shown}");
    if entry.message_id.trim().is_empty() {
        return Err(not_found());
    }
    let cached = store::ai_sort_find_cached(
        &engine.db.lock().unwrap(),
        &entry.account,
        &target,
        &entry.message_id,
    )?;
    let uid = match cached {
        Some(uid) => Some(uid),
        None => {
            let media_root = engine.host.media_root();
            let found = engine
                .with_read_session(&entry.account, |session| {
                    let target = target.clone();
                    let ids = vec![entry.message_id.clone()];
                    let media_root = media_root.clone();
                    let account = entry.account.clone();
                    Box::pin(async move {
                        imap::fetch_by_message_ids(session, &target, &ids, &media_root, &account)
                            .await
                    })
                })
                .await?;
            found.first().map(|message| message.header.uid)
        }
    };
    let uid = uid.ok_or_else(not_found)?;
    move_within_account(engine, &entry.account, &target, "INBOX", &[uid]).await?;
    {
        let db = engine.db.lock().unwrap();
        store::ai_sort_set_log_state(&db, id, "moved_back", "")?;
        // The AI was wrong here: mail like this stays in the Inbox from now on.
        store::ai_lesson_from_log(&db, &entry);
    }
    push_lesson_tags(engine);
    report(engine, &entry.account, "INBOX");
    Ok(())
}

/// Move back for mail the AI filed into a Meron folder: take its conversation
/// out of that folder, on this device only. Not sorted again.
fn move_back_from_meron(
    engine: &Arc<Engine>,
    id: i64,
    entry: &store::AiSortLogEntry,
) -> anyhow::Result<()> {
    let db = engine.db.lock().unwrap();
    let message_id = entry
        .message_id
        .trim()
        .trim_matches(|c| c == ' ' || c == '<' || c == '>')
        .to_lowercase();
    // Where the email was when the AI saw it: the Inbox, or (rows kept since
    // other folders are sorted) the folder a server filter delivered it to.
    let from = if entry.from_folder.is_empty() {
        store::EXISTING_RULE_FOLDER
    } else {
        entry.from_folder.as_str()
    };
    let key = if message_id.is_empty() {
        format!("uid:{from}:{}", entry.uid)
    } else {
        message_id
    };
    let found = store::meron_entry_of_key(&db, &entry.account, &key)?;
    let unmark = match found {
        Some((entry_id, folder_id)) if folder_id == entry.meron_folder_id => {
            let unmark = crate::meron_marks::job_for_released_entry(&db, &entry_id);
            store::meron_release_entry(&db, &entry_id)?;
            unmark
        }
        _ => anyhow::bail!("It's no longer in {}.", entry.to_folder),
    };
    // Back "in the Inbox" for mail from another folder means shown there, on
    // this device; the server copy stays where it is.
    store::ai_sort_inbox_show_again(&db, &entry.account, from, entry.uid, entry.uidvalidity)?;
    store::ai_sort_set_log_state(&db, id, "moved_back", "")?;
    // The AI was wrong here: mail like this stays in the Inbox from now on.
    store::ai_lesson_from_log(&db, entry);
    drop(db);
    push_lesson_tags(engine);
    crate::meron_marks::spawn_writes(engine, unmark);
    report(engine, &entry.account, "INBOX");
    Ok(())
}
