//! Controls mailbox watches while the mobile app is in the foreground.
//! Account and folder interest select IDLE connections and notification polls.
//! Sync results are reported through the native event bridge.

use super::*;

/// Record or drop one side's interest in a watch. Returns whether it changed.
pub(super) fn set_watch_interest(
    interests: &Mutex<Vec<(String, String)>>,
    account: &str,
    folder: &str,
    wanted: bool,
) -> bool {
    let mut interests = interests.lock().unwrap();
    let position = interests
        .iter()
        .position(|(a, f)| a == account && f == folder);
    match (position, wanted) {
        (None, true) => interests.push((account.to_string(), folder.to_string())),
        (Some(index), false) => {
            interests.remove(index);
        }
        _ => return false,
    }
    true
}

pub(super) fn watch_interest(
    interests: &Mutex<Vec<(String, String)>>,
    account: &str,
    folder: &str,
) -> bool {
    interests
        .lock()
        .unwrap()
        .iter()
        .any(|(a, f)| a == account && f == folder)
}

/// Build and host the Engine for the foreground session (idempotent), then start
/// foreground IMAP IDLE on each active account's INBOX so new mail lands in the
/// open view live. The Engine holds its own DB connection and warm session pool
/// for the lifetime of the foreground state.
pub(super) fn engine_foreground() -> Result<Value, String> {
    let data_directory = MOBILE_CONFIG
        .lock()
        .unwrap()
        .as_ref()
        .map(|config| config.data_dir.clone())
        .ok_or_else(|| "mobile core is not initialized".to_string())?;
    // Make sure the runtime is ready so the first routed op doesn't pay for it.
    engine_runtime().ok_or_else(|| "engine runtime unavailable".to_string())?;
    {
        let mut slot = ENGINE.lock().unwrap();
        if slot.is_none() {
            let host = Box::new(MobileHost {
                data_dir: data_directory.clone(),
            });
            let engine = Engine::new(host).map_err(|error| format!("{error:#}"))?;
            let _ = engine.rule_progress.set(Arc::new(|account, folder| {
                emit_event(
                    "mail.synced",
                    json!({ "account": account, "folder": folder, "synced": 0 }),
                );
            }));
            *slot = Some(Arc::new(engine));
        }
    }
    {
        let _ownership = WATCH_OWNERSHIP.lock().unwrap();
        FOREGROUND_SESSION.store(true, Ordering::SeqCst);
    }
    ensure_notify_poller(&data_directory);
    start_foreground_idle(&data_directory);
    Ok(json!({ "ok": true }))
}

/// Park the Engine for the background state: stop the foreground IDLE watches and
/// drop warm pooled sockets (the OS freezes/reclaims them anyway). The Engine
/// itself (DB + creds) is kept so the next foreground transition resumes fast.
pub(super) fn engine_background() -> Result<Value, String> {
    {
        let _ownership = WATCH_OWNERSHIP.lock().unwrap();
        FOREGROUND_SESSION.store(false, Ordering::SeqCst);
        let owned: Vec<(String, String)> = ENGINE_OWNED_WATCHES.lock().unwrap().drain(..).collect();
        for (account, folder) in owned {
            if !watch_interest(&HOST_WATCHES, &account, &folder) {
                let _ = stop_mobile_idle_watch(&account, &folder);
            }
        }
    }
    if let Some(engine) = ENGINE.lock().unwrap().as_ref() {
        engine.clear_all_pools();
    }
    Ok(json!({ "ok": true }))
}

/// Start the mobile foreground mailbox watches.
///
/// Start IDLE for each active (non-RSS, non-paused) account on INBOX, the account's Sent folder and any
/// folder opted in to notifications — INBOX so received mail appears live, Sent so mail sent from another
/// client (each IDLE connection watches a single mailbox) also shows up live in the cross-folder
/// conversation view.
///
/// Records the watches we actually started so `engine_background` stops exactly those, leaving any opt-in
/// live-push watches alone.
fn start_foreground_idle(data_directory: &str) {
    let accounts = match mobile_db(data_directory) {
        Ok(conn) => crate::store::load_accounts(&conn).unwrap_or_default(),
        Err(_) => return,
    };
    for (id, _creds) in accounts {
        let (skip, sent, notify) = match mobile_db(data_directory) {
            Ok(conn) => {
                let skip = is_rss_account(&conn, &id).unwrap_or(false)
                    || crate::store::account_paused(&conn, &id).unwrap_or(false);
                let sent = crate::store::get_folders(&conn, &id)
                    .ok()
                    .and_then(|folders| {
                        folders
                            .into_iter()
                            .map(|folder| folder.name)
                            .find(|name| crate::imap::looks_like_sent(name))
                    });
                // The rest of the opted-in folders are polled; see
                // `poll_notify_folders_once`.
                let notify = crate::store::notify_folder_plan(&conn, &id)
                    .map(|plan| plan.live)
                    .unwrap_or_default();
                (skip, sent, notify)
            }
            Err(_) => (true, None, Vec::new()),
        };
        if skip {
            continue;
        }
        start_owned_watch(data_directory, &id, "INBOX");
        if let Some(sent) = sent
            && !sent.eq_ignore_ascii_case("INBOX")
        {
            start_owned_watch(data_directory, &id, &sent);
        }
        // Already-running watches (Sent, above) are skipped by the dedupe.
        for folder in notify {
            start_owned_watch(data_directory, &id, &folder);
        }
    }
}

/// Stop watching a folder that no longer exists, whoever wanted the watch.
pub(crate) fn forget_deleted_folder_watch(account: &str, folder: &str) {
    let _ownership = WATCH_OWNERSHIP.lock().unwrap();
    set_watch_interest(&ENGINE_OWNED_WATCHES, account, folder, false);
    set_watch_interest(&HOST_WATCHES, account, folder, false);
    let _ = stop_mobile_idle_watch(account, folder);
}

/// Apply a folder's notification opt-in to the running foreground session:
/// start watching it now, or drop the foreground session's interest in it. A
/// no-op in the background, where the host decides what is watched.
pub(crate) fn sync_foreground_notify_watch(
    data_directory: &str,
    account: &str,
    folder: &str,
    enabled: bool,
) {
    if folder.eq_ignore_ascii_case("INBOX") {
        return;
    }
    // Sent stays watched for the conversation view whatever its opt-in says.
    if !enabled && !crate::imap::looks_like_sent(folder) {
        // A watch the host also wants is the host's to stop once it re-reads the flags.
        let _ownership = WATCH_OWNERSHIP.lock().unwrap();
        if set_watch_interest(&ENGINE_OWNED_WATCHES, account, folder, false)
            && !watch_interest(&HOST_WATCHES, account, folder)
        {
            let _ = stop_mobile_idle_watch(account, folder);
        }
    }
    start_live_notify_watches(data_directory, account);
}

/// Watch an account's live opted-in folders for the foreground session.
/// Called whenever the opt-ins change: an enabled folder inside the live set
/// starts now, and one that was polled moves into a slot a disable or delete
/// freed. Skipped while the session is in the background.
pub(crate) fn start_live_notify_watches(data_directory: &str, account: &str) {
    let live = mobile_db(data_directory)
        .ok()
        .and_then(|conn| crate::store::notify_folder_plan(&conn, account).ok())
        .map(|plan| plan.live)
        .unwrap_or_default();
    for folder in live {
        start_owned_watch(data_directory, account, &folder);
    }
}

/// Make sure the poller runs. Call after recording the interest (foreground
/// session or host watch) that needs it.
pub(super) fn ensure_notify_poller(data_directory: &str) {
    if NOTIFY_POLLER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let data_directory = data_directory.to_string();
    let spawned = std::thread::Builder::new()
        .name("meron-notify-poll".to_string())
        .spawn(move || {
            loop {
                std::thread::sleep(NOTIFY_POLL_INTERVAL);
                {
                    let _ownership = WATCH_OWNERSHIP.lock().unwrap();
                    if !FOREGROUND_SESSION.load(Ordering::SeqCst)
                        && HOST_WATCHES.lock().unwrap().is_empty()
                    {
                        NOTIFY_POLLER_RUNNING.store(false, Ordering::SeqCst);
                        return;
                    }
                }
                poll_notify_folders_once(&data_directory);
            }
        });
    if spawned.is_err() {
        NOTIFY_POLLER_RUNNING.store(false, Ordering::SeqCst);
    }
}

/// Check the opted-in folders that have no IDLE connection of their own, over
/// pooled sessions. Covers every active account while the app is in the
/// foreground, and otherwise the accounts the host (live push) is watching.
fn poll_notify_folders_once(data_directory: &str) {
    let Ok(conn) = mobile_db(data_directory) else {
        return;
    };
    let accounts = crate::store::load_accounts(&conn).unwrap_or_default();
    let mut targets = Vec::new();
    for (account, _creds) in accounts {
        if is_rss_account(&conn, &account).unwrap_or(true)
            || crate::store::account_paused(&conn, &account).unwrap_or(true)
        {
            continue;
        }
        let wanted = FOREGROUND_SESSION.load(Ordering::SeqCst)
            || HOST_WATCHES
                .lock()
                .unwrap()
                .iter()
                .any(|(a, _)| *a == account);
        if !wanted {
            continue;
        }
        let polled = crate::store::notify_folder_plan(&conn, &account)
            .map(|plan| plan.polled)
            .unwrap_or_default();
        targets.extend(polled.into_iter().map(|folder| (account.clone(), folder)));
    }
    drop(conn);
    for (account, folder) in targets {
        let watched = MOBILE_IDLE_WATCHES
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|watches| watches.contains_key(&watch_key(&account, &folder)));
        if watched {
            continue;
        }
        match sync_mobile_folder(data_directory, &account, &folder) {
            Ok(sync) => emit_mobile_sync_result(&account, &folder, &sync),
            Err(err) => crate::mlog!(
                crate::log::Level::Warn,
                "mail.sync",
                "poll of {account}/{folder} failed: {err}"
            ),
        }
    }
}

/// One folder's `mail.sync`, without relisting the account's folders.
pub(super) fn sync_mobile_folder(
    data_directory: &str,
    account: &str,
    folder: &str,
) -> Result<Value, String> {
    sync_mobile_mail(
        data_directory,
        &json!({
            "account_id": account,
            "folder_id": folder,
            "limit": 50,
            "folders": false,
        }),
    )
}

/// Tell listeners what a sync found: the arrivals, or just that it ran.
pub(super) fn emit_mobile_sync_result(account: &str, folder: &str, sync: &Value) {
    // New mail AI sorting filed into Meron folders with notifications on.
    for detail in sync
        .get("meron_new_messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|value| value.is_object())
    {
        emit_event("mail.newMessages", detail.clone());
    }
    if let Some(detail) = sync.get("new_messages").filter(|value| value.is_object()) {
        emit_event("mail.newMessages", detail.clone());
    } else {
        let synced = sync.get("synced").and_then(Value::as_u64).unwrap_or(0);
        emit_event(
            "mail.synced",
            json!({ "account": account, "folder": folder, "synced": synced }),
        );
    }
}

/// Start one IDLE watch for the foreground session and, unless it was skipped
/// as rss/paused, record the session's interest in it — also when the host
/// already had it running, so it survives the host letting go. Does nothing
/// once the session has gone to the background.
fn start_owned_watch(data_directory: &str, account: &str, folder: &str) {
    let _ownership = WATCH_OWNERSHIP.lock().unwrap();
    if !FOREGROUND_SESSION.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(result) =
        start_mobile_idle_watch(data_directory, account.to_string(), folder.to_string())
        && watch_is_running(&result)
    {
        set_watch_interest(&ENGINE_OWNED_WATCHES, account, folder, true);
    }
}

/// Whether a `start_mobile_idle_watch` answer means the watch is up, rather
/// than skipped for an RSS or paused account.
pub(super) fn watch_is_running(result: &Value) -> bool {
    result.get("rss").is_none() && result.get("paused").is_none()
}
