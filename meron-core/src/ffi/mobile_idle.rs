//! Handles native mailbox watch commands for the mobile mail core.
//! Each active folder has a worker that can be stopped when interest changes.
//! New mail is synchronized and emitted through the host's existing notification path.

use super::*;

pub(super) fn dispatch_mobile_watch_request(
    data_directory: &str,
    request: &Request,
) -> Result<Value, String> {
    let account = required_account_id(&request.params)?;
    let folder = canon_folder(
        &required_string(&request.params, "folder").unwrap_or_else(|_| "INBOX".to_string()),
    );
    match request.method.as_str() {
        "watch.start" => {
            let _ownership = WATCH_OWNERSHIP.lock().unwrap();
            let result = start_mobile_idle_watch(data_directory, account.clone(), folder.clone())?;
            // The host (the live-push service) now counts on this watch, even
            // one `engine.foreground` started first; going to the background
            // must not stop it.
            if watch_is_running(&result) {
                set_watch_interest(&HOST_WATCHES, &account, &folder, true);
                ensure_notify_poller(data_directory);
            }
            Ok(result)
        }
        "watch.stop" => {
            let _ownership = WATCH_OWNERSHIP.lock().unwrap();
            set_watch_interest(&HOST_WATCHES, &account, &folder, false);
            // The open app keeps the watches its foreground session wants.
            if watch_interest(&ENGINE_OWNED_WATCHES, &account, &folder) {
                return Ok(json!({ "ok": true, "stopped": false }));
            }
            stop_mobile_idle_watch(&account, &folder)
        }
        _ => Err(format!("unknown watch method: {}", request.method)),
    }
}

pub(super) fn watch_key(account: &str, folder: &str) -> String {
    format!("{account}\n{folder}")
}

pub(super) fn mobile_db(data_directory: &str) -> Result<rusqlite::Connection, String> {
    let db_path = Path::new(data_directory.trim()).join("meron.db");
    match mobile_db_key() {
        Some(key) => crate::store::open_at_keyed(&db_path, &key),
        None => crate::store::open_at(&db_path),
    }
    .map_err(|error| format!("{error:#}"))
}

pub(super) fn start_mobile_idle_watch(
    data_directory: &str,
    account: String,
    folder: String,
) -> Result<Value, String> {
    {
        let conn = mobile_db(data_directory)?;
        if is_rss_account(&conn, &account)? {
            return Ok(json!({ "ok": true, "rss": true }));
        }
        if crate::store::account_paused(&conn, &account).map_err(|error| format!("{error:#}"))? {
            return Ok(json!({ "ok": true, "paused": true }));
        }
        let creds = load_mobile_account_creds(&conn, &account)?;
        if account_needs_reconnect(&creds) {
            return Err(format!("account needs reconnect: {account}"));
        }
    }

    let key = watch_key(&account, &folder);
    let mut watches = MOBILE_IDLE_WATCHES.lock().unwrap();
    let watches = watches.get_or_insert_with(HashMap::new);
    if watches.contains_key(&key) {
        return Ok(json!({ "ok": true, "already": true }));
    }
    let stop = Arc::new(AtomicBool::new(false));
    watches.insert(key, stop.clone());
    let data_directory = data_directory.to_string();
    std::thread::Builder::new()
        .name(format!("meron-idle-{account}-{folder}"))
        .spawn({
            let account = account.clone();
            let folder = folder.clone();
            move || mobile_idle_thread(data_directory, account, folder, stop)
        })
        .map_err(|error| format!("spawn idle watcher: {error}"))?;
    Ok(json!({ "ok": true, "already": false }))
}

pub(super) fn stop_mobile_idle_watch(account: &str, folder: &str) -> Result<Value, String> {
    let key = watch_key(account, folder);
    let removed = MOBILE_IDLE_WATCHES
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|watches| watches.remove(&key));
    if let Some(stop) = removed.as_ref() {
        stop.store(true, Ordering::SeqCst);
    }
    Ok(json!({ "ok": true, "stopped": removed.is_some() }))
}

fn mobile_idle_thread(
    data_directory: String,
    account: String,
    folder: String,
    stop: Arc<AtomicBool>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            emit_event(
                "error",
                json!({ "message": format!("idle runtime: {err}") }),
            );
            return;
        }
    };
    runtime.block_on(async {
        while !stop.load(Ordering::SeqCst) {
            match mobile_idle_once(&data_directory, &account, &folder, &stop).await {
                Ok(()) => {}
                Err(err) => {
                    if let Some((name, detail)) =
                        crate::engine::needs_reconnect_event(&account, &err)
                    {
                        emit_event(name, detail);
                    }
                    emit_event(
                        "error",
                        json!({ "message": format!("idle {account}/{folder}: {err:#}") }),
                    );
                    tokio::time::sleep(Duration::from_secs(15)).await;
                }
            }
        }
        // Only our own entry: a stop followed by a quick restart leaves a
        // replacement watcher under the same key, which must stay findable.
        if let Some(watches) = MOBILE_IDLE_WATCHES.lock().unwrap().as_mut() {
            let key = watch_key(&account, &folder);
            if watches
                .get(&key)
                .is_some_and(|current| Arc::ptr_eq(current, &stop))
            {
                watches.remove(&key);
            }
        }
    });
}

async fn mobile_idle_once(
    data_directory: &str,
    account: &str,
    folder: &str,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    {
        let conn = mobile_db(data_directory).map_err(|error| anyhow::anyhow!(error))?;
        if crate::store::account_paused(&conn, account).unwrap_or(false) {
            return Ok(());
        }
    }
    // IDLE used to load the stored credentials directly and authenticate before
    // entering the normal Engine sync path. An expired browser-flow OAuth token
    // therefore failed the connection before Engine::ensure_valid_creds could
    // refresh it. Use the same credential path as every other live IMAP op.
    // Platform-managed Android Gmail is refreshed by the host before it invokes
    // engine.foreground/watch.start; ensure_valid_creds then picks up that token.
    let engine = engine_for(data_directory).map_err(|error| anyhow::anyhow!(error))?;
    let creds = engine.ensure_valid_creds(account).await?;
    if account_needs_reconnect(&creds) {
        anyhow::bail!("account needs reconnect: {account}");
    }
    let mut session = crate::imap::connect(&creds).await?;
    session.select(folder).await?;
    mobile_sync_and_notify(data_directory, account, folder).await?;

    while !stop.load(Ordering::SeqCst) {
        let mut handle = session.idle();
        handle.init().await?;
        let response = {
            let (idle_fut, _stop) = handle.wait_with_timeout(Duration::from_secs(15 * 60));
            tokio::select! {
                biased;
                // Stopped: give the connection back now, without waiting for
                // DONE. A quiet mailbox would otherwise hold it until the IDLE
                // timeout, alongside whatever watch replaced this one.
                _ = wait_for_mobile_watch_stop(stop) => return Ok(()),
                response = idle_fut => response,
            }
        };
        session = handle.done().await?;
        if let async_imap::extensions::idle::IdleResponse::NewData(_) = response? {
            mobile_sync_and_notify(data_directory, account, folder).await?;
        }
    }
    Ok(())
}

/// Resolves once a watch's stop flag is set. The flag is a plain atomic set
/// from other threads, so it is checked on a short tick.
async fn wait_for_mobile_watch_stop(stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn mobile_sync_and_notify(
    data_directory: &str,
    account: &str,
    folder: &str,
) -> anyhow::Result<()> {
    let owned_directory = data_directory.to_string();
    let account_owned = account.to_string();
    let folder_owned = folder.to_string();
    let sync = tokio::task::spawn_blocking(move || {
        sync_mobile_folder(&owned_directory, &account_owned, &folder_owned)
    })
    .await?
    .map_err(|error| anyhow::anyhow!(error))?;
    emit_mobile_sync_result(account, folder, &sync);
    Ok(())
}
