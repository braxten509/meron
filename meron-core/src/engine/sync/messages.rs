//! Messages in the mail synchronization engine.
//!
//! Coordinates the corresponding IMAP and store operations during folder updates.
//! The parent module exposes these operations to foreground and background sync.

use super::*;

pub async fn sync_messages(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> anyhow::Result<SyncMessagesResult> {
    sync_messages_with_policy(
        engine,
        account,
        folder,
        limit,
        false,
        ATTACHMENT_CATCHUP_BUDGET,
    )
    .await
}

/// Automatic desktop refresh: bounded network work, respecting pause state.
/// IDLE recovery and explicit mobile refresh retain their original policy.
pub async fn sync_background_messages(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
) -> anyhow::Result<SyncMessagesResult> {
    sync_messages_with_policy(
        engine,
        account,
        folder,
        limit,
        true,
        ATTACHMENT_CATCHUP_BUDGET,
    )
    .await
}

mod refresh;
pub use refresh::{MessageSyncGuard, WrittenFolderRefresh, refresh_written_folder};

const ATTACHMENT_CATCHUP_BUDGET: Duration = Duration::from_secs(5);

pub(super) async fn sync_messages_with_policy(
    engine: &Arc<Engine>,
    account: &str,
    folder: &str,
    limit: u32,
    background: bool,
    catchup_budget: Duration,
) -> anyhow::Result<SyncMessagesResult> {
    // When this sync's reads began: a Meron-folder mark this device wrote
    // after it may not show in them (see `store::meron_marks_observe`).
    let read_at = store::meron_mark_tick();
    // Read the prior sync position before any network I/O so we can ask the
    // server for only the flag changes since then (CONDSTORE CHANGEDSINCE).
    // Callers must await this operation rather than timing out the whole sync;
    // only the network phase below is cancellable by the sync budget.
    let prepare_engine = engine.clone();
    let prepare_account = account.to_string();
    let prepare_folder = folder.to_string();
    let (
        prior_modseq,
        prior_validity,
        removed_epoch,
        removal_guard,
        missing_files,
        needs_tag_baseline,
        known,
    ) = tokio::task::spawn_blocking(move || {
        let engine = prepare_engine;
        let account = prepare_account.as_str();
        let folder = prepare_folder.as_str();
        let db = crate::log::timed_db_lock(&engine.db, "sync_messages.prepare/backfill");
        if background && store::account_paused(&db, account)? {
            return Err(anyhow::Error::new(BackgroundSyncCancelled));
        }
        let modseq = store::get_folder_modseq(&db, account, folder)?;
        let validity = store::get_folder_state(&db, account, folder)?
            .map(|(v, _)| v)
            .unwrap_or(0);
        // Removals after this point (an archive that lands while the fetch is
        // still on the wire) get a higher epoch. This snapshot must not write
        // those UIDs back.
        let missing_files = store::missing_attachment_metadata_uids(&db, account, folder, limit)?;
        let (id, removed_epoch) = store::begin_message_sync(&db)?;
        let guard = MessageSyncGuard {
            engine: engine.clone(),
            id,
        };
        let tag_baseline = store::needs_tag_baseline(&db, account, folder)?;
        // A folder synced before reads only what changed: the UIDs it already
        // caches are left out of the full FETCH.
        let known = if validity != 0 {
            Some(Arc::new(store::header_synced_uids(&db, account, folder)?))
        } else {
            None
        };
        anyhow::Ok((
            modseq,
            validity,
            removed_epoch,
            guard,
            missing_files,
            tag_baseline,
            known,
        ))
    })
    .await??;

    let fetch = || async {
        let attempt = engine
            .with_read_session(account, |session| {
                let folder = folder.to_string();
                let known = known.clone();
                Box::pin(async move {
                    // Folder synced before: read in full only the window's
                    // messages the cache lacks. Otherwise (first sync, new
                    // UIDVALIDITY, a listing that contradicts its SELECT) the
                    // whole window, as always.
                    let incremental = match known.as_deref() {
                        Some(known) => {
                            imap::fetch_recent_incremental(
                                session,
                                &folder,
                                limit,
                                prior_validity,
                                known,
                            )
                            .await?
                        }
                        None => None,
                    };
                    let (batch, window, listed_uids) = match incremental {
                        Some(read) => (read.batch, Some(read.window), Some(read.server_uids)),
                        None => (
                            imap::fetch_recent(session, &folder, limit).await?,
                            None,
                            None,
                        ),
                    };
                    let flags_for = window
                        .as_deref()
                        .map(|window| cached_in_window(window, &batch));
                    let reads = read_flag_state(
                        session,
                        &folder,
                        batch.uidvalidity,
                        flags_for,
                        listed_uids,
                        prior_modseq,
                        prior_validity,
                        needs_tag_baseline,
                    )
                    .await?;
                    anyhow::Ok(FolderSyncState {
                        batch,
                        flag_sync: reads.flag_sync,
                        server_uids: reads.server_uids,
                        server_unseen: reads.server_unseen,
                        window,
                        window_flags: reads.window_flags,
                    })
                })
            })
            .await;
        // A message whose FETCH response we cannot parse takes the whole batch down
        // with it, and does so again on every later sync. Re-read the window a
        // narrower range at a time so the rest of the folder still syncs.
        let state = match attempt {
            Ok(state) => Ok(state),
            Err(operation_error) if imap::is_unparseable_response(&operation_error) => {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.sync",
                    "account={account} folder={folder}: unparseable FETCH response, \
                 re-reading the window message by message: {operation_error:#}"
                );
                sync_state_isolating_unparseable(
                    engine,
                    account,
                    folder,
                    limit,
                    prior_modseq,
                    prior_validity,
                    known.as_deref(),
                    needs_tag_baseline,
                )
                .await
            }
            Err(operation_error) => Err(operation_error),
        }?;
        Ok(state)
    };
    let network_started = std::time::Instant::now();
    let FolderSyncState {
        batch,
        flag_sync,
        server_uids,
        server_unseen,
        window,
        window_flags,
    } = if background {
        retry_background_sync(&format!("sync {folder} for {account}"), || true, fetch).await?
    } else {
        fetch().await?
    };
    let network_ms = network_started.elapsed().as_millis();
    // What the network phase read: messages in full, and FLAGS-only reads of
    // cached ones. An unchanged folder synced before reads 0 and 0.
    let fetched_full = batch.messages.len();
    let fetched_flags = window_flags.len();
    let mode = if window.is_some() {
        "incremental"
    } else {
        "full"
    };
    crate::mlog!(
        crate::log::Level::Info,
        "sync.fetch",
        "account={account} folder={folder:?} mode={mode} window={} fetched_full={fetched_full} \
         fetched_flags={fetched_flags} condstore_changes={} network_ms={network_ms}",
        window.as_ref().map_or(fetched_full, Vec::len),
        flag_sync
            .as_ref()
            .filter(|sync| sync.complete)
            .map_or(-1, |sync| sync.changes.len() as i64)
    );
    let synced_validity = batch.uidvalidity;
    let catchup_uids: Vec<_> = if prior_validity != 0 && prior_validity == synced_validity {
        missing_files
            .iter()
            .copied()
            .filter(|uid| !batch.messages.iter().any(|header| header.uid == *uid))
            .collect()
    } else {
        Vec::new()
    };
    let attempted: Vec<_> = missing_files
        .into_iter()
        .filter(|uid| batch.messages.iter().any(|header| header.uid == *uid))
        .collect();
    let tag_engine = engine.clone();
    let tag_account = account.to_string();
    let rule_folder = folder.to_string();
    let catchup_engine = engine.clone();
    let catchup_account = account.to_string();
    let catchup_folder = folder.to_string();

    let synced_messages = batch.messages.clone();
    // The window's size either way, so "synced" means what it always did.
    let count = window.as_ref().map_or(synced_messages.len(), Vec::len);
    // The lowest UID from which every message on the server was in the window:
    // all of them when the window wasn't full, else the window's oldest UID.
    let covered_from = if (count as u32) < limit {
        0
    } else {
        window
            .as_ref()
            .and_then(|window| window.iter().min().copied())
            .or_else(|| synced_messages.iter().map(|message| message.uid).min())
            .unwrap_or(u32::MAX)
    };
    crate::mlog!(
        crate::log::Level::Debug,
        "mail.sync",
        "messages account={account} folder={folder}: fetched={count} \
         uidvalidity={} uid_next={} prior_validity={prior_validity} server_uids={}",
        batch.uidvalidity,
        batch.uid_next,
        server_uids
            .as_ref()
            .map_or(-1, |message_uid| message_uid.len() as i64)
    );
    // AI sorting of new Inbox mail, when it is on and set up for this account.
    let ai_plan = ai_sort::plan_for(engine, account, folder).await;
    let ai_hold = ai_plan.is_some();
    let engine = engine.clone();
    let account = account.to_string();
    let folder = folder.to_string();
    // Outside the network timeout: always observe the committed result.
    let (mut result, removal_guard, ai_candidates, new_unread, mark_filed, uncached_marked, deferred_modseq) = tokio::task::spawn_blocking(move || {
    let account = account.as_str();
    let folder = folder.as_str();
    let db = crate::log::timed_db_lock(&engine.db, "sync_messages.persist");
    let current = store::get_folder_state(&db, account, folder)?;
    let notifies = store::folder_notifies(&db, account, folder)?;
    // Also when AI sorting may file this folder's new mail into a Meron folder
    // with notifications on; announced as the folder's own only when it notifies.
    // And when another device's Meron-folder mark may file new Inbox mail into
    // a Meron folder with notifications on.
    let marks_notify = folder.eq_ignore_ascii_case("INBOX") && store::meron_any_folder_notifies(&db)?;
    let new_unread = if current.is_some_and(|(validity, _)| validity == batch.uidvalidity) && (notifies || ai_hold || marks_notify) {
        store::classify_arrivals(&db, account, folder, current.unwrap().1, batch.uid_next, &batch.messages)?
    } else { Vec::new() };
    let arrivals = if notifies { new_unread.clone() } else { Vec::new() };
    // Before the upsert records them as observed. Nothing on a first sync, when
    // everything is new to the store and none of it to the mailbox.
    let first_seen_headers = match current {
        Some((validity, uid_next)) if validity == batch.uidvalidity => {
            store::first_seen_messages(&db, account, folder, uid_next, batch.uid_next, &batch.messages)?
        }
        _ => Vec::new(),
    };
    let first_seen: Vec<u32> = first_seen_headers.iter().map(|message| message.uid).collect();
    // Held back before the upsert, so no other connection lists the row first.
    // Another folder AI sorting watches (mail a server filter delivered
    // there): only what arrived since its first look, never on that look.
    // Its watch only moves past what this sync's window covered, so mail that
    // didn't fit is read later (see `ai_sort::check_watched_folders`).
    let hold = || if folder.eq_ignore_ascii_case("INBOX") {
        store::ai_sort_hold(&db, account, folder, batch.uidvalidity, &first_seen_headers, store::now_unix())
    } else {
        store::ai_sort_hold_folder(&db, account, folder, batch.uidvalidity, batch.uid_next, (covered_from, batch.uid_next), &batch.messages, store::now_unix())
    };
    // New Inbox copies (Gmail) of mail the AI already sent to a server folder
    // from a label: they get that move, without asking the AI again.
    if ai_hold && folder.eq_ignore_ascii_case("INBOX") {
        if let Err(operation_error) = store::ai_sort_reuse_for_inbox(&db, account, batch.uidvalidity, &batch.messages, store::now_unix()) {
            crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account}: reusing decisions skipped: {operation_error:#}");
        }
    }
    // Synced while sorting is off for it: as in the Inbox, that mail is never
    // sorted later, so a watched folder's watch moves past it.
    if !ai_hold && !folder.eq_ignore_ascii_case("INBOX") {
        store::ai_sort_watch_skip(&db, account, folder, batch.uidvalidity, batch.uid_next)?;
    }
    let mut ai_candidates = if ai_hold {
        hold()
            .unwrap_or_else(|operation_error| {
                crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account}: hold skipped: {operation_error:#}");
                Vec::new()
            })
    } else {
        Vec::new()
    };
    let persist_started = std::time::Instant::now();
    let phase_started = std::time::Instant::now();
    if prior_validity != 0 && prior_validity != batch.uidvalidity {
        store::clear_folder_messages(&db, account, folder)?;
        store::ai_sort_clear_folder(&db, account, folder)?;
    }
    let clear_time = phase_started.elapsed();
    let phase_started = std::time::Instant::now();
    let fetched_uids: Vec<u32> = batch.messages.iter().map(|message| message.uid).collect();
    store::release_removed_messages(&db, account, folder, &fetched_uids, removed_epoch)?;
    // Inbox mail that was there before Meron watched, read now only because the
    // window reached further back: kept out of the Inbox views. Decided before
    // the upsert, which would make it look cached already, and marked in the
    // same write that stores it.
    let backfill = store::inbox_backfill_uids(&db, account, folder, batch.uidvalidity, batch.uid_next, &batch.messages)?;
    store::upsert_messages_marking(&db, account, folder, &batch.messages, &backfill)?;
    store::mark_attachment_metadata_attempted(&db, account, folder, &attempted)?;
    let upsert_time = phase_started.elapsed();
    // Make sure the folder is represented in the folders table so its unread
    // count surfaces (tray dot / badges) even before a full folder LIST sync —
    // which, in the unified view, may never run for this account.
    let phase_started = std::time::Instant::now();
    store::ensure_folder(&db, account, folder)?;
    let ensure_time = phase_started.elapsed();
    let phase_started = std::time::Instant::now();
    let mut pruned = 0;
    if let Some(uids) = server_uids.as_ref() {
        let validity_ok = prior_validity == 0 || prior_validity == batch.uidvalidity;
        // An empty UID set means "the server holds nothing here" only if the
        // fetch also came back empty. Having just read `count` messages out of
        // this same mailbox, an empty SEARCH result contradicts itself — the
        // connection lied (see the CONDSTORE desync in imap::sync_flags) — and
        // pruning against it would delete the very messages just stored.
        let uids_credible = !uids.is_empty() || count == 0;
        if validity_ok && uids_credible {
            // The batch's UIDNEXT was read before the UID list, so any row at
            // or above it was cached after this snapshot and is not stale.
            pruned =
                store::prune_missing_messages(&db, account, folder, uids, batch.uid_next)?;
        } else if !uids_credible {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "skipping prune for account={account} folder={folder}: \
                 UID SEARCH returned no UIDs but {count} messages were fetched"
            );
        }
    }
    let prune_time = phase_started.elapsed();
    // Mail the AI shows in the Inbox from another folder follows what this
    // sync stored: gone from its folder, or now in the Inbox itself.
    if let Err(operation_error) = store::ai_sort_inbox_after_sync(&db, account, folder, batch.uidvalidity, batch.uid_next, &batch.messages) {
        crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account} folder={folder}: shown-in-Inbox upkeep skipped: {operation_error:#}");
    }
    // Meron folder releases a failed write left behind, from any account.
    crate::meron_folders::retry_pending_releases(&db);
    let flag_count = flag_sync.as_ref().map_or(0, |fs| fs.changes.len());
    let phase_started = std::time::Instant::now();
    // Meron-folder marks this sync read on the Inbox's copies: new mail, and
    // flag reads of cached mail. Only responses that carried FLAGS count.
    let mut marks_seen: Vec<store::MeronMarkSeen> = Vec::new();
    for message in batch.messages.iter().filter(|message| message.server_tags.is_some()) {
        if let Some(mark) = &message.meron_mark {
            marks_seen.push(store::MeronMarkSeen { uid: message.uid, mark: mark.clone() });
        }
    }
    // Cached window messages an incremental sync did not re-read in full:
    // only their flags and tags change, leaving every other column as is.
    for (uid, seen, starred, tags, mark) in &window_flags {
        store::update_message_seen(&db, account, folder, *uid, *seen)?;
        store::update_message_starred(&db, account, folder, *uid, *starred)?;
        store::observe_server_tags(&db, account, folder, *uid, tags)?;
        if let Some(mark) = mark {
            marks_seen.push(store::MeronMarkSeen { uid: *uid, mark: mark.clone() });
        }
    }
    // The CONDSTORE position this sync reached; stored after the marks are
    // read, as one skipped there must be read again.
    let mut reached_modseq = 0;
    if let Some(fs) = flag_sync {
        for (uid, seen, starred, tags, mark) in &fs.changes {
            if let Some(mark) = mark {
                marks_seen.push(store::MeronMarkSeen { uid: *uid, mark: mark.clone() });
            }
            store::update_message_seen(&db, account, folder, *uid, *seen)?;
            store::update_message_starred(&db, account, folder, *uid, *starred)?;
            store::observe_server_tags(&db, account, folder, *uid, tags)?;
        }
        if let Some(baseline) = &fs.tag_baseline {
            for (uid, tags) in baseline {
                store::observe_server_tags(&db, account, folder, *uid, tags)?;
            }
            store::mark_tag_baseline(&db, account, folder)?;
        }
        reached_modseq = fs.highest_modseq;
    }
    let flags_time = phase_started.elapsed();
    // Another device filed (or took out) Inbox or archived mail in a Meron
    // folder: follow.
    let mut mark_filed = Vec::new();
    let mut uncached_marked = Vec::new();
    if store::meron_reads_marks(&db, account, folder) {
        // Copies gone from the server first: one still remembered would keep
        // its conversation filed when its last other copy loses the mark.
        if let Some(uids) = server_uids.as_ref().filter(|uids| !uids.is_empty()) {
            let _ = store::meron_marks_prune(&db, account, folder, uids);
        }
        match store::meron_marks_observe(&db, account, folder, &marks_seen, read_at, store::now_unix()) {
            Ok(applied) => {
                mark_filed = applied.filed;
                uncached_marked = applied.uncached_marked;
                // Marks left unread: the next sync's CHANGEDSINCE reads them again.
                if applied.skipped_stale {
                    reached_modseq = 0;
                }
            }
            Err(operation_error) => {
                crate::mlog!(crate::log::Level::Warn, "mail.meronMarks", "account={account}: reading Meron folder marks skipped: {operation_error:#}");
                reached_modseq = 0;
            }
        }
    }
    // Archived marked mail this sync couldn't cache is read after it: the
    // CONDSTORE position moves past those marks only once every one is read
    // (see `fetch_uncached_marked`), so none is lost if that read fails.
    let deferred_modseq = if uncached_marked.is_empty() { 0 } else { reached_modseq };
    if reached_modseq > 0 && deferred_modseq == 0 {
        store::set_folder_modseq(&db, account, folder, reached_modseq)?;
    }
    if let Some(unseen) = server_unseen.as_ref() {
        store::set_uncached_unseen(&db, account, folder, unseen)?;
    }
    if let Err(operation_error) = store::apply_rules(&db, account, folder, batch.uidvalidity, &first_seen) {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.rules",
            "account={account} folder={folder}: rules skipped: {operation_error:#}"
        );
    }
    // The user's rules come first: what they queued work for isn't the AI's.
    if !ai_candidates.is_empty() {
        let candidates = std::mem::take(&mut ai_candidates);
        let uids: Vec<u32> = candidates.iter().map(|candidate| candidate.uid).collect();
        ai_candidates = store::ai_sort_skip_rule_actions(&db, account, folder, candidates)
            .unwrap_or_else(|operation_error| {
                crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account}: sort skipped: {operation_error:#}");
                let _ = store::ai_sort_release(&db, account, folder, &uids);
                Vec::new()
            });
    }
    // A conversation already in a Meron folder (the upsert just adopted it) is
    // never re-sorted: shown at once, its cap charge given back.
    if !ai_candidates.is_empty() {
        let candidates = std::mem::take(&mut ai_candidates);
        let uids: Vec<u32> = candidates.iter().map(|candidate| candidate.uid).collect();
        ai_candidates = store::ai_sort_skip_meron_foldered(&db, account, folder, candidates)
            .unwrap_or_else(|operation_error| {
                crate::mlog!(crate::log::Level::Warn, "mail.aiSort", "account={account}: sort skipped: {operation_error:#}");
                let _ = store::ai_sort_release(&db, account, folder, &uids);
                Vec::new()
            });
    }
    let phase_started = std::time::Instant::now();
    store::set_folder_state(&db, account, folder, batch.uidvalidity, batch.uid_next)?;
    let state_time = phase_started.elapsed();
    let total = persist_started.elapsed();
    drop(db);
    if total.as_millis() >= 100 {
        crate::mlog!(
            crate::log::Level::Warn,
            "sync.persist.timing",
            "account={account} folder={folder:?} fetched={count} mode={mode} fetched_full={fetched_full} fetched_flags={fetched_flags} server_uids={} pruned={pruned} flag_changes={flag_count} clear_ms={} upsert_ms={} ensure_ms={} prune_ms={} flags_ms={} state_ms={} total_ms={}",
            server_uids.as_ref().map_or(0, |uids| uids.len()),
            clear_time.as_millis(),
            upsert_time.as_millis(),
            ensure_time.as_millis(),
            prune_time.as_millis(),
            flags_time.as_millis(),
            state_time.as_millis(),
            total.as_millis()
        );
    }
    Ok::<_, anyhow::Error>((SyncMessagesResult {
        count, messages: synced_messages, arrivals, meron_arrivals: Vec::new(),
    }, removal_guard, ai_candidates, new_unread, mark_filed, uncached_marked, deferred_modseq))
    }).await??;
    // A conversation in the built-in Archive that got Inbox mail left it in
    // the upsert above: its marks go now.
    crate::meron_marks::flush_archive_unmarks(&tag_engine);
    // Archived mail another device filed whose copy this sync didn't cache:
    // read and stored, then its mark is acted on.
    if !uncached_marked.is_empty() {
        let all_read =
            fetch_uncached_marked(&tag_engine, &tag_account, &rule_folder, uncached_marked).await;
        // Every one read: the position the sync reached can be kept now. A
        // later sync that already moved further is left alone.
        if all_read && deferred_modseq > 0 {
            let db = tag_engine.db.lock().unwrap();
            let stored = store::get_folder_modseq(&db, &tag_account, &rule_folder).unwrap_or(0);
            if stored < deferred_modseq
                && let Err(operation_error) =
                    store::set_folder_modseq(&db, &tag_account, &rule_folder, deferred_modseq)
            {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "mail.meronMarks",
                    "account={tag_account}: keeping the sync position skipped: {operation_error:#}"
                );
            }
        }
    }

    // The main sync is already committed. Catch-up gets its own small budget;
    // an exhausted deadline cannot discard new mail, flags, or sync markers.
    if !catchup_uids.is_empty() {
        let deadline = tokio::time::Instant::now() + catchup_budget;
        match fetch_headers_isolating_until(
            &catchup_engine,
            &catchup_account,
            &catchup_folder,
            catchup_uids,
            Some(deadline),
        )
        .await
        {
            Ok((headers, skipped)) => {
                let persist = tokio::task::spawn_blocking(move || {
                    let _removal_guard = removal_guard;
                    let db = catchup_engine.db.lock().unwrap();
                    if store::get_folder_state(&db, &catchup_account, &catchup_folder)?
                        .map(|(validity, _)| validity)
                        != Some(synced_validity)
                    {
                        return anyhow::Ok(());
                    }
                    let attempted: Vec<_> = headers
                        .iter()
                        .map(|header| header.uid)
                        .chain(skipped)
                        .collect();
                    store::upsert_messages(&db, &catchup_account, &catchup_folder, &headers)?;
                    store::mark_attachment_metadata_attempted(
                        &db,
                        &catchup_account,
                        &catchup_folder,
                        &attempted,
                    )?;
                    anyhow::Ok(())
                })
                .await;
                if let Err(operation_error) = persist
                    .map_err(anyhow::Error::from)
                    .and_then(|result| result)
                {
                    crate::mlog!(
                        crate::log::Level::Warn,
                        "mail.sync",
                        "attachment metadata persistence skipped: {operation_error:#}"
                    );
                }
            }
            Err(operation_error) => crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "attachment metadata catch-up skipped: {operation_error:#}"
            ),
        }
    }
    let tags_pending = store::has_pending_tags(&tag_engine.db.lock().unwrap(), &tag_account)?;
    if tags_pending {
        spawn_tag_push(&tag_engine, &tag_account);
    }
    // Awaited, unlike the tags, so what the caller announces next is where the
    // rules left it: mail they read or moved doesn't notify, and mail they failed
    // to still does. Whatever doesn't finish stays queued for the next sync.
    // Within its own budget, before the queue runs the moves it decides on.
    let sorted = match ai_plan {
        Some(plan) if !ai_candidates.is_empty() => {
            ai_sort::sort_new_mail(
                &tag_engine,
                &tag_account,
                &rule_folder,
                synced_validity,
                plan,
                ai_candidates,
            )
            .await
        }
        _ => ai_sort::SortedNewMail::default(),
    };
    // What another device's marks filed is announced the same way, and stays
    // silent as the folder's own arrivals.
    let mut filed = sorted.filed;
    filed.extend(mark_filed.iter().cloned());
    result.meron_arrivals = ai_sort::meron_arrivals(&tag_engine, &rule_folder, &new_unread, &filed);
    let mut ai_moved = sorted.silenced;
    ai_moved.extend(mark_filed.iter().map(|(uid, _)| *uid));
    let mut silenced = std::collections::BTreeSet::new();
    let rules_ran = tokio::time::timeout(
        RULE_ACTIONS_BUDGET,
        run_rule_actions(&tag_engine, &tag_account, &rule_folder, &mut silenced),
    )
    .await;
    match rules_ran {
        Ok(Ok(())) => {}
        Ok(Err(operation_error)) => crate::mlog!(
            crate::log::Level::Warn,
            "mail.rules",
            "account={tag_account}: rule actions skipped: {operation_error:#}"
        ),
        // A large message can take longer to copy than any sync may wait.
        Err(_) => spawn_rule_worker(&tag_engine, &tag_account, &rule_folder),
    }
    // Mail the AI is moving doesn't alert, even while the move still waits.
    silenced.extend(ai_moved);
    // An AI move that failed for good alerts now, as a failed rule move does.
    let late = if rule_folder.eq_ignore_ascii_case("INBOX") {
        let db = tag_engine.db.lock().unwrap();
        store::ai_sort_take_alerts(&db, &tag_account, &rule_folder)
            .ok()
            .filter(|_| store::folder_notifies(&db, &tag_account, &rule_folder).unwrap_or(false))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    ai_sort::settle_arrivals(&mut result.arrivals, &silenced, late);
    Ok(result)
}
