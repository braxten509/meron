//! C ABI entry points for mobile hosts.
//!
//! This is intentionally small while the full engine is still being extracted
//! from the desktop sidecar binary. Android/JNI and iOS can use these functions
//! to assert protocol compatibility before calling the future command API.

mod initialization;
use initialization::{configure_mobile_core, init_mobile_core, prepare_mobile_store};

mod foreground;
use foreground::{
    emit_mobile_sync_result, engine_background, engine_foreground, ensure_notify_poller,
    set_watch_interest, sync_mobile_folder, watch_interest, watch_is_running,
};
pub(crate) use foreground::{
    forget_deleted_folder_watch, start_live_notify_watches, sync_foreground_notify_watch,
};
mod mobile_idle;
use mobile_idle::{
    dispatch_mobile_watch_request, mobile_db, start_mobile_idle_watch, stop_mobile_idle_watch,
    watch_key,
};

use std::collections::HashMap;
use std::ffi::CStr;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use jni::JNIEnv;
use jni::JavaVM;
use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jstring};
use serde_json::{Value, json};

use crate::engine::{Engine, OAuthDefaults, set_oauth_defaults};
use crate::protocol::{
    MobileHost, PROTOCOL_VERSION, Request, account_needs_reconnect, canon_folder,
    invoke_mobile_protocol_json, is_rss_account, load_mobile_account_creds, mobile_db_key,
    ping_response, ready_event, required_account_id, required_string, sync_mobile_mail,
};

type EventCallback = unsafe extern "C" fn(event_json: *const c_char, user_data: *mut c_void);

#[derive(Clone, Copy)]
struct EventSink {
    callback: EventCallback,
    user_data: usize,
}

static EVENT_SINK: Mutex<Option<EventSink>> = Mutex::new(None);
static MOBILE_CONFIG: Mutex<Option<MobileConfig>> = Mutex::new(None);
static ANDROID_EVENT_DISPATCHER: OnceLock<AndroidEventDispatcher> = OnceLock::new();
#[cfg(target_os = "android")]
static ANDROID_CONTEXT: Mutex<Option<GlobalRef>> = Mutex::new(None);
static MOBILE_IDLE_WATCHES: Mutex<Option<HashMap<String, Arc<AtomicBool>>>> = Mutex::new(None);

/// The shared mail [`Engine`], hosted while the app is foreground so the request path reuses warm
/// IMAP sessions (and, later, drives foreground IDLE) instead of reconnecting per FFI call. Built
/// on `engine.foreground`, kept across calls, and parked (pool cleared) on `engine.background`.
/// `None` until the first foreground transition.
static ENGINE: Mutex<Option<Arc<Engine>>> = Mutex::new(None);

/// Long-lived multi-thread tokio runtime that owns the Engine's async work
/// (pooled session ops and, later, IDLE watcher tasks). Built once, lazily, so
/// individual FFI calls can `block_on` engine ops without spinning up a runtime
/// each time.
static ENGINE_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// The Engine's runtime, built on first use. Returns `None` only if the runtime
/// failed to build (host out of threads/memory), which the caller surfaces.
fn engine_runtime() -> Option<&'static tokio::runtime::Runtime> {
    if ENGINE_RT.get().is_none() {
        match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let _ = ENGINE_RT.set(rt);
            }
            Err(error) => {
                eprintln!("meron-core: engine runtime build failed: {error}");
                return None;
            }
        }
    }
    ENGINE_RT.get()
}

/// A clone of the live Engine handle, if one is hosted (app is foreground).
pub(crate) fn current_engine() -> Option<Arc<Engine>> {
    ENGINE.lock().unwrap().clone()
}

/// Evict `account_id` from the hosted Engine's in-memory creds cache, if an Engine is live.
///
/// Account mutations (add/reconnect, removal, platform token push via `account.updateOAuthToken`)
/// go through the stateless DB path, and `ensure_valid_creds` only hydrates accounts *missing* from
/// its cache — so without eviction a warm foreground Engine keeps authenticating with the stale
/// creds it loaded before the mutation.
pub(crate) fn evict_engine_account(account_id: &str) {
    if let Some(engine) = current_engine() {
        engine.accounts.blocking_lock().remove(account_id);
        // Pooled sessions were authenticated with the creds we just evicted and
        // may even point at a different server after a settings edit; reusing
        // one would defeat the eviction.
        engine.clear_pool(account_id);
    }
}

/// The Engine to run a mobile command against: the warm hosted one while foreground, or a transient
/// one (own connection, no pooling) built on demand for background/one-off calls (e.g. a
/// `BGAppRefreshTask` sync before any foreground transition). Either way the command path is
/// identical, so there is only one implementation to maintain.
pub(crate) fn engine_for(data_directory: &str) -> Result<Arc<Engine>, String> {
    if let Some(engine) = current_engine() {
        return Ok(engine);
    }
    let host = Box::new(MobileHost {
        data_dir: data_directory.to_string(),
    });
    Engine::new(host)
        .map(Arc::new)
        .map_err(|error| format!("{error:#}"))
}

/// Run an async engine op on the shared runtime, blocking the calling FFI thread
/// until it completes. Used by the mobile command handlers routed through the
/// Engine.
pub(crate) fn engine_block_on<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
{
    engine_block_on_anyhow(future).map_err(|error| format!("{error:#}"))
}

/// Folder resyncs running, by [`watch_key`]. `true` when another request came
/// in meanwhile, so that folder syncs once more after the current run.
static FOLDER_RESYNCS: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

/// Schedule a folder refresh after a server mutation.
///
/// Sync `folders` in the background: after a server move or delete whose local cache could not follow, the sync
/// prunes the rows the server no longer has and caches the moved copies. The same one-folder sync the notify
/// poller runs, telling listeners with the same event when it is done, so lists and counts refresh. Per account
/// and folder at most one runs and one waits; further requests fold into the waiting one.
pub(crate) fn spawn_folder_resync(data_directory: &str, account: &str, folders: &[&str]) {
    for folder in folders {
        let key = watch_key(account, folder);
        {
            let mut resyncs = FOLDER_RESYNCS.lock().unwrap();
            let resyncs = resyncs.get_or_insert_with(HashMap::new);
            if let Some(pending) = resyncs.get_mut(&key) {
                *pending = true;
                continue;
            }
            resyncs.insert(key.clone(), false);
        }
        let data_directory = data_directory.to_string();
        let account = account.to_string();
        let folder = folder.to_string();
        let thread_key = key.clone();
        let spawned = std::thread::Builder::new()
            .name("meron-resync".to_string())
            .spawn(move || {
                loop {
                    match sync_mobile_folder(&data_directory, &account, &folder) {
                        Ok(sync) => emit_mobile_sync_result(&account, &folder, &sync),
                        Err(err) => crate::mlog!(
                            crate::log::Level::Warn,
                            "mail.sync",
                            "account={account} folder={folder}: resync after a failed cache update failed: {err}"
                        ),
                    }
                    let mut resyncs = FOLDER_RESYNCS.lock().unwrap();
                    let Some(resyncs) = resyncs.as_mut() else {
                        return;
                    };
                    match resyncs.get_mut(&thread_key) {
                        Some(pending) if *pending => *pending = false,
                        _ => {
                            resyncs.remove(&thread_key);
                            return;
                        }
                    }
                }
            });
        if spawned.is_err()
            && let Some(resyncs) = FOLDER_RESYNCS.lock().unwrap().as_mut()
        {
            resyncs.remove(&key);
        }
    }
}

/// Push pending tag changes in the background, on the engine runtime.
pub(crate) fn spawn_tag_push(data_directory: &str, account: &str) {
    let (Ok(engine), Some(runtime)) = (engine_for(data_directory), engine_runtime()) else {
        return;
    };
    let account = account.to_string();
    runtime.spawn(async move {
        if let Err(err) = crate::engine::push_pending_tags(&engine, &account).await {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.tags",
                "account={account}: tag push failed: {err:#}"
            );
        }
    });
}

/// Run `future` in the background on the engine runtime (phone commands are
/// called outside any runtime). Dropped, answering `false`, when the runtime
/// can't be built.
pub(crate) fn spawn_engine_task<F>(future: F) -> bool
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    match engine_runtime() {
        Some(runtime) => {
            runtime.spawn(future);
            true
        }
        None => false,
    }
}

/// Variant that preserves the concrete error for callers which must reconcile
/// details from a partially applied server mutation.
pub(crate) fn engine_block_on_anyhow<F, T>(future: F) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
{
    let rt = engine_runtime().ok_or_else(|| anyhow::anyhow!("engine runtime unavailable"))?;
    // Inside a bulk run the future runs as this thread's member of it.
    rt.block_on(crate::engine::bulk::with_thread_seat(future))
}

/// IDLE watches the foreground session (`engine.foreground`) wants, as opposed to the ones the host
/// asked for ([`HOST_WATCHES`], the opt-in live-mail-push feature). A watch runs while either side
/// wants it, so each consults the other before stopping one: `engine.background` leaves live-push
/// watches running, and turning live push off leaves the open app's watches running.
static ENGINE_OWNED_WATCHES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// IDLE watches the host asked for through `watch.start`.
static HOST_WATCHES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Held across every "start or stop a watch and update who wants it" step.
/// The two sides run on different threads (the activity backgrounding while
/// the push service acquires its watches), and without it one could stop a
/// watch between the other finding it running and recording its interest.
static WATCH_OWNERSHIP: Mutex<()> = Mutex::new(());

/// Whether the foreground session is up, between `engine.foreground` and
/// `engine.background`. Read and written under [`WATCH_OWNERSHIP`], so a
/// watch start that lost the race with backgrounding is skipped rather than
/// left running with nothing to tear it down.
static FOREGROUND_SESSION: AtomicBool = AtomicBool::new(false);

/// How often opted-in folders past the live set are checked for new mail.
const NOTIFY_POLL_INTERVAL: Duration = Duration::from_secs(120);

/// Whether the poller thread is up. Flipped off under [`WATCH_OWNERSHIP`], as
/// the interests that keep it alive are set, so a start cannot be lost to an
/// exit decided a moment earlier.
static NOTIFY_POLLER_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, PartialEq, Eq)]
struct MobileConfig {
    data_dir: String,
    /// SQLCipher key (64 hex chars), or empty for an unencrypted store.
    db_key: String,
}

struct AndroidEventDispatcher {
    vm: JavaVM,
    class: GlobalRef,
}

#[unsafe(no_mangle)]
pub extern "C" fn meron_core_protocol_version() -> u32 {
    PROTOCOL_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreProtocolVersion(
    _env: *mut c_void,
    _class: *mut c_void,
) -> c_int {
    PROTOCOL_VERSION as c_int
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreInitializeAndroidContext(
    env: JNIEnv,
    _class: JClass,
    context: JObject,
) {
    let mut stored_context = ANDROID_CONTEXT.lock().unwrap();
    if stored_context.is_some() {
        return;
    }
    let Ok(vm) = env.get_java_vm() else {
        return;
    };
    let Ok(context) = env.new_global_ref(context) else {
        return;
    };
    // Hickory reads Android's active DNS servers through ndk-context when it
    // performs the SRV fallback used by account autodiscovery. Keep the Java
    // context alive for as long as ndk-context can hand its raw pointer out.
    unsafe {
        ndk_context::initialize_android_context(
            vm.get_java_vm_pointer().cast(),
            context.as_obj().as_raw().cast(),
        );
    }
    *stored_context = Some(context);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreConfigureOAuthDefaults(
    mut env: JNIEnv,
    _class: JClass,
    google_client_id: JString,
    google_token_url: JString,
    outlook_client_id: JString,
) {
    let google_client_id = env
        .get_string(&google_client_id)
        .map(|value| value.to_string_lossy().trim().to_string())
        .unwrap_or_default();
    let google_token_url = env
        .get_string(&google_token_url)
        .map(|value| value.to_string_lossy().trim().to_string())
        .unwrap_or_default();
    let outlook_client_id = env
        .get_string(&outlook_client_id)
        .map(|value| value.to_string_lossy().trim().to_string())
        .unwrap_or_default();
    set_oauth_defaults(OAuthDefaults {
        google_client_id,
        google_token_url,
        outlook_client_id,
        ..OAuthDefaults::default()
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreInvokeJson(
    mut env: JNIEnv,
    _class: JClass,
    request: JString,
) -> jstring {
    let request = match env.get_string(&request) {
        Ok(value) => value.to_string_lossy().into_owned(),
        Err(err) => {
            return java_string(
                &env,
                serde_json::json!({ "error": { "message": format!("bad JNI string: {err}") } })
                    .to_string(),
            );
        }
    };
    java_string(&env, invoke_mobile_request_json(&request).to_string())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreInitJson(
    mut env: JNIEnv,
    _class: JClass,
    data_directory: JString,
) -> jstring {
    let data_directory = match env.get_string(&data_directory) {
        Ok(value) => value.to_string_lossy().into_owned(),
        Err(err) => {
            return java_string(
                &env,
                serde_json::json!({ "error": { "message": format!("bad JNI string: {err}") } })
                    .to_string(),
            );
        }
    };
    java_string(&env, init_mobile_core(&data_directory, None).to_string())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreConfigureJsonKeyed(
    mut env: JNIEnv,
    _class: JClass,
    data_directory: JString,
    db_key: JString,
) -> jstring {
    let strings = env
        .get_string(&data_directory)
        .map(|value| value.to_string_lossy().into_owned())
        .and_then(|data_directory| {
            env.get_string(&db_key)
                .map(|value| (data_directory, value.to_string_lossy().into_owned()))
        });
    let response = match strings {
        Ok((data_directory, db_key)) => configure_mobile_core(&data_directory, Some(&db_key)),
        Err(err) => serde_json::json!({ "error": { "message": format!("bad JNI string: {err}") } }),
    };
    java_string(&env, response.to_string())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreInitJsonKeyed(
    mut env: JNIEnv,
    _class: JClass,
    data_directory: JString,
    db_key: JString,
) -> jstring {
    let data_directory = match env.get_string(&data_directory) {
        Ok(value) => value.to_string_lossy().into_owned(),
        Err(err) => {
            return java_string(
                &env,
                serde_json::json!({ "error": { "message": format!("bad JNI string: {err}") } })
                    .to_string(),
            );
        }
    };
    let db_key = match env.get_string(&db_key) {
        Ok(value) => value.to_string_lossy().into_owned(),
        Err(err) => {
            return java_string(
                &env,
                serde_json::json!({ "error": { "message": format!("bad JNI string: {err}") } })
                    .to_string(),
            );
        }
    };
    java_string(
        &env,
        init_mobile_core(&data_directory, Some(&db_key)).to_string(),
    )
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreRegisterEventCallback(
    env: JNIEnv,
    class: JClass,
) {
    let Ok(vm) = env.get_java_vm() else {
        return;
    };
    let Ok(class) = env.new_global_ref(class) else {
        return;
    };
    let _ = ANDROID_EVENT_DISPATCHER.set(AndroidEventDispatcher { vm, class });
    meron_core_register_event_callback(Some(android_event_callback), ptr::null_mut());
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreUnregisterEventCallback(
    _env: JNIEnv,
    _class: JClass,
) {
    meron_core_register_event_callback(None, ptr::null_mut());
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_jp_nonbili_meron_MeronCoreNative_meronCoreEmitReadyEvent(
    _env: JNIEnv,
    _class: JClass,
) -> jboolean {
    meron_core_emit_ready_event().into()
}

#[unsafe(no_mangle)]
pub extern "C" fn meron_core_ready_json() -> *mut c_char {
    json_to_c_string(ready_event())
}

#[unsafe(no_mangle)]
pub extern "C" fn meron_core_ping_json() -> *mut c_char {
    json_to_c_string(ping_response())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn meron_core_invoke_json(request: *const c_char) -> *mut c_char {
    if request.is_null() {
        return json_to_c_string(serde_json::json!({
            "error": { "message": "request pointer is null" }
        }));
    }
    let request = unsafe { CStr::from_ptr(request) }.to_string_lossy();
    json_to_c_string(invoke_mobile_request_json(&request))
}

/// Set up the core without opening the store; see [`configure_mobile_core`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn meron_core_configure_json_keyed(
    data_directory: *const c_char,
    db_key: *const c_char,
) -> *mut c_char {
    if data_directory.is_null() || db_key.is_null() {
        return json_to_c_string(serde_json::json!({
            "error": { "message": "data_dir and db_key are required" }
        }));
    }
    let data_directory = unsafe { CStr::from_ptr(data_directory) }.to_string_lossy();
    let db_key = unsafe { CStr::from_ptr(db_key) }.to_string_lossy();
    json_to_c_string(configure_mobile_core(&data_directory, Some(&db_key)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn meron_core_init_json(data_directory: *const c_char) -> *mut c_char {
    if data_directory.is_null() {
        return json_to_c_string(serde_json::json!({
            "error": { "message": "data_dir pointer is null" }
        }));
    }
    let data_directory = unsafe { CStr::from_ptr(data_directory) }.to_string_lossy();
    json_to_c_string(init_mobile_core(&data_directory, None))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn meron_core_init_json_keyed(
    data_directory: *const c_char,
    db_key: *const c_char,
) -> *mut c_char {
    if data_directory.is_null() || db_key.is_null() {
        return json_to_c_string(serde_json::json!({
            "error": { "message": "data_dir and db_key pointers are required" }
        }));
    }
    let data_directory = unsafe { CStr::from_ptr(data_directory) }.to_string_lossy();
    let db_key = unsafe { CStr::from_ptr(db_key) }.to_string_lossy();
    json_to_c_string(init_mobile_core(&data_directory, Some(&db_key)))
}

/// Registers the mobile host event callback.
///
/// Passing `None` unregisters the callback. The callback receives a pointer
/// that is valid only for the duration of the call, so platform hosts must copy
/// the JSON string before returning.
#[unsafe(no_mangle)]
pub extern "C" fn meron_core_register_event_callback(
    callback: Option<EventCallback>,
    user_data: *mut c_void,
) {
    let mut sink = EVENT_SINK.lock().unwrap();
    *sink = callback.map(|callback| EventSink {
        callback,
        user_data: user_data as usize,
    });
}

/// Emits the initial ready event through the registered callback.
///
/// Mobile hosts call this after registering their callback to verify event
/// delivery before issuing commands. Later extraction work will route all core
/// events through the same sink.
#[unsafe(no_mangle)]
pub extern "C" fn meron_core_emit_ready_event() -> bool {
    emit_event("ready", ready_event())
}

/// Frees strings returned from Meron core FFI functions.
///
/// # Safety
///
/// `ptr` must be either null or a pointer returned by this library from a
/// function documented as returning an owned C string. Passing any other pointer
/// is undefined behavior.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn meron_core_string_free(pointer: *mut c_char) {
    if pointer.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(pointer));
    }
}

fn json_to_c_string(value: serde_json::Value) -> *mut c_char {
    match CString::new(value.to_string()) {
        Ok(s) => s.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

/// Which data dir's store is ready for commands. Held across the upgrade, so
/// everything that needs the store waits here instead of racing the migration on
/// a second connection.
static STORE_PREPARED: Mutex<Option<String>> = Mutex::new(None);
/// Whether [`prepare_mobile_store`] is upgrading the store right now, for
/// `core.status`, which must answer without waiting on it.
static STORE_UPGRADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn invoke_mobile_request_json(request: &str) -> serde_json::Value {
    let data_directory = MOBILE_CONFIG
        .lock()
        .unwrap()
        .as_ref()
        .map(|config| config.data_dir.clone());
    // Neither needs the store, and they are what a host can ask while it waits.
    let command_request = serde_json::from_str::<Request>(request).ok();
    let method = command_request
        .as_ref()
        .map(|command_request| command_request.method.as_str());
    if method == Some("core.status") {
        let migrating = STORE_UPGRADING.load(Ordering::SeqCst);
        return json!({ "id": command_request.map(|command_request| command_request.id), "result": { "migrating": migrating } });
    }
    if method != Some("ping")
        && let Some(data_directory) = data_directory.as_deref()
    {
        prepare_mobile_store(data_directory);
    }
    if let Some(data_directory) = data_directory.as_deref()
        && let Some(command_request) = command_request
        && matches!(
            command_request.method.as_str(),
            "watch.start" | "watch.stop" | "engine.foreground" | "engine.background"
        )
    {
        let result = match command_request.method.as_str() {
            "engine.foreground" => engine_foreground(),
            "engine.background" => engine_background(),
            _ => dispatch_mobile_watch_request(data_directory, &command_request),
        };
        return match result {
            Ok(result) => json!({ "id": command_request.id, "result": result }),
            Err(message) => json!({ "id": command_request.id, "error": { "message": message } }),
        };
    }
    invoke_mobile_protocol_json(request, data_directory.as_deref())
}

fn event_envelope(name: &str, detail: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "event": name, "detail": detail })
}

pub(crate) fn emit_event(name: &str, detail: serde_json::Value) -> bool {
    let sink = *EVENT_SINK.lock().unwrap();
    let Some(sink) = sink else {
        return false;
    };
    let Ok(event_json) = CString::new(event_envelope(name, detail).to_string()) else {
        return false;
    };
    unsafe {
        (sink.callback)(event_json.as_ptr(), sink.user_data as *mut c_void);
    }
    true
}

/// `mail.newMessages` detail for `headers` (the arrivals from
/// [`mobile_new_unread_inbox_messages`]). Shared by the IDLE watch event and the
/// `mail.sync` response so background workers — which have no live event
/// listener — can notify from the response alone.
pub(crate) fn mobile_new_messages_detail(
    data_directory: &str,
    account: &str,
    folder: &str,
    headers: &[crate::imap::MessageHeader],
) -> Option<Value> {
    let account_name = mobile_account_label(data_directory, account);
    let muted = mobile_account_muted(data_directory, account);
    let conn = mobile_db(data_directory).ok()?;
    crate::mail_model::new_messages_detail(&conn, account, &account_name, folder, muted, headers)
}

/// `mail.newMessages` detail for new mail AI sorting filed into a Meron folder
/// with notifications on (see [`crate::mail_model::meron_new_messages_detail`]).
pub(crate) fn mobile_meron_new_messages_detail(
    data_directory: &str,
    account: &str,
    arrivals: &crate::engine::ai_sort::MeronArrivals,
) -> Option<Value> {
    let account_name = mobile_account_label(data_directory, account);
    let muted = mobile_account_muted(data_directory, account);
    let conn = mobile_db(data_directory).ok()?;
    crate::mail_model::meron_new_messages_detail(&conn, account, &account_name, muted, arrivals)
}

/// Inbox messages that arrived between the two `uid_next` snapshots and are
/// still unread, newest first; None when nothing new landed.
#[cfg(test)]
pub(crate) fn mobile_new_unread_inbox_messages(
    data_directory: &str,
    account: &str,
    uid_next_before: u32,
    uid_next_after: u32,
    synced_messages: &[crate::imap::MessageHeader],
) -> Option<Vec<crate::imap::MessageHeader>> {
    let conn = mobile_db(data_directory).ok()?;
    crate::store::new_unread_inbox_messages(
        &conn,
        account,
        uid_next_before,
        uid_next_after,
        synced_messages,
    )
    .ok()
    .flatten()
}

fn mobile_account_label(data_directory: &str, account: &str) -> String {
    let Ok(conn) = mobile_db(data_directory) else {
        return account.to_string();
    };
    crate::store::account_label(&conn, account)
}

fn mobile_account_muted(data_directory: &str, account: &str) -> bool {
    mobile_db(data_directory)
        .ok()
        .and_then(|conn| crate::store::account_muted(&conn, account).ok())
        .unwrap_or(false)
}

unsafe extern "C" fn android_event_callback(event_json: *const c_char, _user_data: *mut c_void) {
    if event_json.is_null() {
        return;
    }
    let Some(dispatcher) = ANDROID_EVENT_DISPATCHER.get() else {
        return;
    };
    let event_json = unsafe { CStr::from_ptr(event_json) }.to_string_lossy();
    let Ok(mut env) = dispatcher.vm.attach_current_thread() else {
        return;
    };
    let Ok(event) = env.new_string(event_json.as_ref()) else {
        return;
    };
    let event = JObject::from(event);
    let class = unsafe { JClass::from_raw(dispatcher.class.as_raw() as jni::sys::jclass) };
    let _ = env.call_static_method(
        class,
        "dispatchCoreEventFromNative",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&event)],
    );
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
}

fn java_string(env: &JNIEnv, value: String) -> jstring {
    match env.new_string(value) {
        Ok(output) => output.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests;
