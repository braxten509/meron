//! Prepares and reconciles local-folder marks during a server archive operation.
//! Successful transfers file the new copies and failed transfers restore previous marks.
//! These operations keep the local archive card aligned with the server result.

use super::*;

/// Before the server move of an archive [`archive_filing_keys`] files: the moved copies get the
/// built-in Archive's mark in their source folder (a MOVE carries keywords along), and the card's
/// other mark-folder copies that aren't moved lose theirs. Awaited (a few seconds at most); best
/// effort.
pub async fn prepare_archive_marks(
    engine: &Arc<Engine>,
    account: &str,
    card: &Option<store::MeronCardRelease>,
    source_folder: &str,
    uids: &[u32],
) {
    let mut jobs = vec![MarkJob {
        account: account.to_string(),
        folder: source_folder.to_string(),
        uids: uids.to_vec(),
        target: Some(store::MERON_ARCHIVE_FOLDER_ID.to_string()),
    }];
    if let Some(card) = card {
        let rows: Vec<(String, u32)> = card
            .rows
            .iter()
            .filter(|(folder, uid)| !(folder == source_folder && uids.contains(uid)))
            .cloned()
            .collect();
        let marked = crate::meron_marks::uids_marked(&engine.db.lock().unwrap(), account, &rows);
        jobs.extend(crate::meron_marks::jobs_for(account, marked, None));
    }
    crate::meron_marks::write_all(engine, jobs).await;
}

/// [`prepare_archive_marks`] for the phone's blocking commands.
pub fn prepare_archive_marks_blocking(
    engine: &Arc<Engine>,
    account: &str,
    card: &Option<store::MeronCardRelease>,
    source_folder: &str,
    uids: &[u32],
) {
    let _ = crate::ffi::engine_block_on(async {
        prepare_archive_marks(engine, account, card, source_folder, uids).await;
        anyhow::Ok(())
    });
}

/// The server move after [`prepare_archive_marks`] failed: put the marks
/// back as they were, in the background. A card that was in a Meron folder
/// gets that folder's mark again; else the moved copies lose the Archive's.
pub fn restore_marks_after_failed_archive(
    engine: &Arc<Engine>,
    conn: &Connection,
    account: &str,
    card: &Option<store::MeronCardRelease>,
    source_folder: &str,
    uids: &[u32],
) {
    // First the unmark, so a filed card's mark below is the latest wish.
    let mut jobs = vec![MarkJob {
        account: account.to_string(),
        folder: source_folder.to_string(),
        uids: uids.to_vec(),
        target: None,
    }];
    if let Some(card) = card {
        jobs.extend(crate::meron_marks::job_for_filed_entry(
            conn,
            &card.entry_id,
            &card.folder_id,
        ));
    }
    crate::meron_marks::spawn_writes(engine, jobs);
}

/// The server move of copies out of the archive mailbox failed after the
/// card's built-in Archive marks were taken off (an Undo of an archive, say):
/// still in Archive, so its copies get the Archive's mark back, in the
/// background. Nothing for a card of any other folder.
pub fn restore_archive_card_marks(
    engine: &Arc<Engine>,
    conn: &Connection,
    card: &Option<store::MeronCardRelease>,
) {
    let Some(card) = card
        .as_ref()
        .filter(|card| card.folder_id == store::MERON_ARCHIVE_FOLDER_ID)
    else {
        return;
    };
    let jobs = crate::meron_marks::job_for_filed_entry(conn, &card.entry_id, &card.folder_id);
    crate::meron_marks::spawn_writes(engine, jobs);
}

/// After an archive's server move and caching: file the conversations into the built-in Archive.
///
/// `keys` are [`archive_filing_keys`]'s, read before the move; the archived copies `target_uids`
/// (when known) add their own, and location keys of the old folder are dropped. A conversation that
/// still has cached Inbox mail isn't filed (Archive never holds Inbox mail). The archived copies
/// carry the Archive's mark already: remembered, so this device's next read doesn't act on it.
///
/// Answers the mailboxes whose counts changed.
pub fn file_after_archive(
    conn: &Connection,
    account: &str,
    archive_folder: &str,
    target_uids: &[u32],
    keys: &BTreeSet<String>,
) -> BTreeSet<(String, String)> {
    let mut wanted: BTreeSet<String> = keys
        .iter()
        .filter(|key| {
            key.strip_prefix("uid:")
                .and_then(|rest| rest.rsplit_once(':'))
                .is_none_or(|(folder, _)| folder == archive_folder)
        })
        .cloned()
        .collect();
    // The conversation key of each archived copy, for its mark below.
    let mut key_of_uid: Vec<(u32, Option<String>)> = Vec::with_capacity(target_uids.len());
    for uid in target_uids {
        let key = store::meron_conversation_key(conn, account, archive_folder, *uid)
            .ok()
            .flatten();
        if let Some(key) = &key {
            wanted.insert(key.clone());
        }
        key_of_uid.push((*uid, key));
    }
    let mut touched = BTreeSet::new();
    // Conversations not filed because Inbox mail of theirs is still cached.
    let mut skipped: BTreeSet<String> = BTreeSet::new();
    let filed = (|| -> Result<()> {
        let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
        for key in &wanted {
            if store::meron_conversation_has_inbox_copy(&tx, account, key)? {
                skipped.insert(key.clone());
                continue;
            }
            if store::meron_entry_of_key(&tx, account, key)?
                .is_some_and(|(_, folder)| folder == store::MERON_ARCHIVE_FOLDER_ID)
            {
                continue;
            }
            let moved = store::meron_move_in_in(
                &tx,
                account,
                key,
                store::MERON_ARCHIVE_FOLDER_ID,
                store::MeronFiling::User,
            )?;
            touched.extend(
                moved
                    .touched
                    .into_iter()
                    .map(|folder| (account.to_string(), folder)),
            );
        }
        tx.commit()?;
        Ok(())
    })();
    if let Err(err) = filed {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.meronFolders",
            "could not file an archived conversation into Archive: {err:#}"
        );
        return BTreeSet::new();
    }
    if !target_uids.is_empty() && store::meron_reads_marks(conn, account, archive_folder) {
        // Filed copies: handled. Copies of a conversation not filed yet wait
        // for its Inbox mail to go; every archive-mailbox read re-checks them
        // and files them then (see `store::WAITING_FOR_INBOX`).
        let (waiting, handled): (Vec<_>, Vec<_>) = key_of_uid
            .into_iter()
            .partition(|(_, key)| key.as_ref().is_some_and(|key| skipped.contains(key)));
        let waiting: Vec<u32> = waiting.into_iter().map(|(uid, _)| uid).collect();
        let handled: Vec<u32> = handled.into_iter().map(|(uid, _)| uid).collect();
        for (uids, written_at) in [
            (handled, store::meron_mark_tick()),
            (waiting, store::WAITING_FOR_INBOX),
        ] {
            if uids.is_empty() {
                continue;
            }
            if let Err(err) = store::meron_marks_written(
                conn,
                account,
                archive_folder,
                &uids,
                Some(store::MERON_ARCHIVE_FOLDER_ID),
                written_at,
            ) {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.meronFolders",
                    "could not remember Archive marks: {err:#}"
                );
            }
        }
    }
    touched
}
