//! Opens and prepares the mobile mail store.
//! Native configuration and initialization use the same database and key.
//! Recovery runs before mail commands are accepted.

use super::*;

/// Everything a host needs to set up without touching the store: safe on a main
/// thread, where a migration would stall the launch until the OS kills it and
/// rolls the migration back. The first command prepares the store.
pub(super) fn configure_mobile_core(
    data_directory: &str,
    db_key: Option<&str>,
) -> serde_json::Value {
    let data_directory = data_directory.trim();
    if data_directory.is_empty() {
        return serde_json::json!({ "error": { "message": "data_dir is required" } });
    }
    let db_key = db_key.map(str::trim).unwrap_or_default();
    *MOBILE_CONFIG.lock().unwrap() = Some(MobileConfig {
        data_dir: data_directory.to_string(),
        db_key: db_key.to_string(),
    });
    // Publish the key so `with_mobile_db` opens the encrypted store. An empty
    // key (unkeyed init) leaves the store plaintext.
    crate::protocol::set_mobile_db_key(if db_key.is_empty() {
        None
    } else {
        Some(db_key.to_string())
    });
    // Route core logs to the platform over the event channel; the host forwards
    // `log` events to os_log / Logcat. Without this the core's logs (which use
    // `eprintln!` on desktop) are invisible on device.
    crate::log::set_sink(Box::new(|level, tag, message| {
        let _ = emit_event(
            "log",
            json!({ "level": level.as_str(), "tag": tag, "message": message }),
        );
    }));
    // A panic unwinding out of an `extern "C"` entry point aborts the process,
    // so the host never sees a Kotlin/Swift exception. The hook runs first and
    // gets the panic into the diagnostic log before the abort.
    crate::log::install_panic_hook();
    serde_json::json!({
        "ok": true,
        "protocol": PROTOCOL_VERSION,
        "data_dir": data_directory,
    })
}

/// [`configure_mobile_core`] plus the store, for hosts already off the main
/// thread that want it ready before their first command.
pub(super) fn init_mobile_core(data_directory: &str, db_key: Option<&str>) -> serde_json::Value {
    let configured = configure_mobile_core(data_directory, db_key);
    if configured.get("error").is_none() {
        prepare_mobile_store(data_directory.trim());
    }
    configured
}

/// Upgrade the store and load what commands assume is loaded. Once per process
/// and data dir; every command comes through here, so none can run ahead of it,
/// and a failure is retried by the next one, which then reports its own error.
pub(super) fn prepare_mobile_store(data_directory: &str) {
    let mut prepared = STORE_PREPARED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if prepared.as_deref() == Some(data_directory) {
        return;
    }
    let db_path = Path::new(data_directory).join("meron.db");
    let key = crate::protocol::mobile_db_key();
    let opened = crate::store::open_at_notifying(&db_path, key.as_deref(), || {
        STORE_UPGRADING.store(true, Ordering::SeqCst);
    });
    STORE_UPGRADING.store(false, Ordering::SeqCst);
    let conn = match opened {
        Ok(conn) => conn,
        Err(operation_error) => {
            crate::mlog!(
                crate::log::Level::Warn,
                "store",
                "prepare: {operation_error:#}"
            );
            return;
        }
    };
    // Publish the app-wide proxy before any command can open a socket. Mobile
    // has stateless command paths that connect without ever building an Engine
    // (background syncs, one-off sends).
    if let Err(operation_error) = crate::proxy::load_global(&conn) {
        crate::mlog!(
            crate::log::Level::Warn,
            "store",
            "proxy: {operation_error:#}"
        );
    }
    crate::store::tidy_at_start(&conn);
    drop(conn);
    *prepared = Some(data_directory.to_string());
    drop(prepared);
    // List cards name attachments from a column older caches lack. Fill it on
    // its own connection, a batch at a time, so commands opening the store in
    // the meantime only ever wait on one short transaction. Once per process.
    static FILES_BACKFILL_STARTED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    if !FILES_BACKFILL_STARTED.swap(true, Ordering::SeqCst) {
        let data_directory = data_directory.to_string();
        std::thread::spawn(move || {
            let result = crate::protocol::with_mobile_db(&data_directory, |conn| {
                while !crate::store::backfill_files_batch(
                    &conn,
                    crate::engine::FILES_BACKFILL_BATCH,
                )
                .map_err(|operation_error| format!("{operation_error:#}"))?
                {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Ok(json!({ "ok": true }))
            });
            if let Err(operation_error) = result {
                crate::mlog!(
                    crate::log::Level::Warn,
                    "store",
                    "attachment backfill: {operation_error}"
                );
            }
        });
    }
}
