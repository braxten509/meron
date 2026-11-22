//! Recovers missing message bodies and media for an open conversation.
//! The thread reader schedules work against the engine and stores completed bodies.
//! Per-message recovery state keeps overlapping requests from repeating work.

use super::*;

pub(super) fn media_recovery_held(hold: &MediaRecoveryHold, key: &str) -> bool {
    match hold.get(key) {
        Some(None) => true,
        Some(Some(until)) => std::time::Instant::now() < *until,
        None => false,
    }
}

/// Record what the already-cached bodies from a thread download mean for each message's attachment recovery (see `Engine::media_recovery_hold`):
///
/// - arrived with its files: no hold;
/// - arrived and files are still missing: held until restart, since another
///   download is not known to help and a reader that keeps re-reading would
///   otherwise be sent to fetch it without end;
/// - already cached, asked for in a folder the download got to, and not
///   delivered: held for
///   [`MEDIA_RECOVERY_COOLDOWN`] after a failure, or until restart when a
///   successful FETCH left it out (an expunged UID);
/// - uncached bodies or a folder the download never reached: no hold.
///
/// Returns each delivered message with its count of missing files.
pub(super) fn store_thread_bodies(
    engine: &Engine,
    account: &str,
    asked: &BTreeMap<String, Vec<u32>>,
    fetched: Vec<(String, u32, parse::Message)>,
    attempted: &HashSet<String>,
    succeeded: bool,
    media_root: &std::path::Path,
) -> Vec<(String, u32, parse::Message, usize)> {
    // The files are looked for before any lock is taken.
    let delivered: Vec<(String, u32, parse::Message, usize)> = fetched
        .into_iter()
        .map(|(folder, uid, message)| {
            let media_missing = parse::missing_media_count(media_root, &message);
            (folder, uid, message, media_missing)
        })
        .collect();
    let cached_bodies = {
        let db = engine.db.lock().unwrap();
        asked
            .iter()
            .flat_map(|(folder, uids)| {
                uids.iter().filter_map(|uid| {
                    store::get_cached_message(&db, account, folder, *uid)
                        .ok()
                        .flatten()
                        .filter(|message| !message.message_id.is_empty())
                        .map(|_| message_key(account, folder, *uid))
                })
            })
            .collect::<HashSet<_>>()
    };
    let now = std::time::Instant::now();
    let mut hold = engine.media_recovery_hold.lock().unwrap();
    hold.retain(|_, until| until.is_none_or(|until| now < until));
    let mut arrived: HashSet<String> = HashSet::new();
    for (folder, uid, _, media_missing) in &delivered {
        let key = message_key(account, folder, *uid);
        if *media_missing == 0 {
            hold.remove(&key);
        } else {
            hold.insert(key.clone(), None);
        }
        arrived.insert(key);
    }
    for (folder, uids) in asked {
        if !attempted.contains(folder) {
            continue;
        }
        for uid in uids {
            let key = message_key(account, folder, *uid);
            // Holds are only for attachment recovery, never first-time bodies.
            if !cached_bodies.contains(&key) {
                hold.remove(&key);
                continue;
            }
            // A hold until restart is not shortened by a later miss.
            if !arrived.contains(&key) && hold.get(&key) != Some(&None) {
                hold.insert(
                    key,
                    if succeeded {
                        None
                    } else {
                        Some(now + MEDIA_RECOVERY_COOLDOWN)
                    },
                );
            }
        }
    }
    delivered
}

/// Fetch the bodies for `indices` from IMAP on one session (each mailbox is
/// SELECTed once), cache them, and mark their slots complete. `limits` are for
/// all of them; what arrived before a failure is cached and marked all the
/// same.
pub(super) async fn fetch_into_slots(
    engine: &Arc<Engine>,
    account: &str,
    headers: &[imap::MessageHeader],
    slots: &mut [Slot],
    indices: &[usize],
    media_root: &std::path::Path,
    limits: TransferLimits,
) -> anyhow::Result<()> {
    let _snapshot_guard = MessageSyncGuard::begin(engine)?;
    let mut by_folder: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for &index in indices {
        by_folder
            .entry(slots[index].folder.clone())
            .or_default()
            .push(headers[index].uid);
    }
    let fetched = crate::engine::fetch_bodies_keeping_partial(
        engine, account, &by_folder, media_root, limits,
    )
    .await;
    let delivered = store_thread_bodies(
        engine,
        account,
        &by_folder,
        fetched.messages,
        &fetched.attempted,
        fetched.result.is_ok(),
        media_root,
    );
    for (folder, uid, message, media_missing) in delivered {
        if let Some(&index) = indices
            .iter()
            .find(|&&index| slots[index].folder == folder && headers[index].uid == uid)
        {
            slots[index].media_missing = media_missing;
            slots[index].cached = Some(message);
            slots[index].complete = true;
        }
    }
    fetched.result
}

/// Fetch a thread's uncached message bodies in the background, then run the
/// platform hook so the open reader re-reads and the bodies appear. Keeps the
/// per-message IMAP fetches off the read's request path. Deduped per message
/// via `engine.body_fetches`.
pub(crate) fn spawn_fill_thread_bodies(
    engine: &Arc<Engine>,
    account: &str,
    missing: Vec<(String, u32)>,
    media_root: PathBuf,
    on_bodies_fetched: Option<BodiesFetchedHook>,
) {
    let missing: Vec<_> = {
        let mut filling = engine.body_fetches.lock().unwrap();
        missing
            .into_iter()
            .filter(|(folder, uid)| filling.insert(message_key(account, folder, *uid)))
            .collect()
    };
    if missing.is_empty() {
        return;
    }
    let keys: Vec<_> = missing
        .iter()
        .map(|(folder, uid)| message_key(account, folder, *uid))
        .collect();
    let engine = engine.clone();
    let account = account.to_string();
    tokio::spawn(async move {
        let mut by_folder: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for (folder, uid) in missing {
            by_folder.entry(folder).or_default().push(uid);
        }
        let mut delivered = false;
        let fill = async {
            let _guard = MessageSyncGuard::begin(&engine)?;
            let fetched = crate::engine::fetch_bodies_keeping_partial(
                &engine,
                &account,
                &by_folder,
                &media_root,
                BACKGROUND_BODY_LIMITS,
            )
            .await;
            delivered = !store_thread_bodies(
                &engine,
                &account,
                &by_folder,
                fetched.messages,
                &fetched.attempted,
                fetched.result.is_ok(),
                &media_root,
            )
            .is_empty();
            fetched.result
        };
        let result = fill.await;
        if let Err(fetch_error) = &result {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail",
                "background thread body fetch for {account}: {fetch_error:#}"
            );
        }
        let error = result
            .as_ref()
            .err()
            .map(|fetch_error| format!("{fetch_error:#}"));
        let uncached: Vec<(&String, u32)> = {
            let db = engine.db.lock().unwrap();
            by_folder
                .iter()
                .flat_map(|(folder, uids)| uids.iter().map(move |uid| (folder, *uid)))
                .filter(|(folder, uid)| {
                    !store::has_cached_body(&db, &account, folder, *uid).unwrap_or(false)
                })
                .collect()
        };
        // A FETCH that succeeded yet left a message out means the server no
        // longer has it there: another client (or a write whose cache update
        // failed) moved or deleted it, and folders nobody watches never sync
        // the stale row away. Resync those folders before the readers wake, so
        // the re-read shows the message's current copy instead of a dead row.
        if result.is_ok() {
            let stale: BTreeSet<&String> = uncached.iter().map(|(folder, _)| *folder).collect();
            for folder in stale {
                if let Err(fetch_error) =
                    crate::engine::sync_messages(&engine, &account, folder, 50).await
                {
                    crate::mlog!(
                        crate::log::Level::Warn,
                        "mail.sync",
                        "account={account} folder={folder}: resync after a vanished message failed: {fetch_error:#}"
                    );
                }
            }
        }
        for (folder, uid) in &uncached {
            engine.record_body_failure(
                message_key(&account, folder, *uid),
                error.clone().unwrap_or_else(|| {
                    format!("message uid {uid} not returned by FETCH in {folder}")
                }),
            );
        }
        {
            let mut filling = engine.body_fetches.lock().unwrap();
            for key in keys {
                filling.remove(&key);
            }
        }
        engine.body_fetch_done.notify_waiters();
        // Wake readers that returned placeholders while this fill was pending:
        // the next read shows what arrived, or consumes a recorded failure
        // instead of starting a loop. A fill that changed nothing a reader can
        // see (attachment files still missing, their bodies cached all along)
        // stays quiet, or the re-read would only start it again.
        if (delivered || !uncached.is_empty())
            && let Some(notify) = on_bodies_fetched
        {
            notify();
        }
    });
}
