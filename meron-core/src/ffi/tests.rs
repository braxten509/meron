//! Regression tests for ffi.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;
use std::ffi::CStr;
use std::sync::Mutex;

static CAPTURED_EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static EVENT_CALLBACK_TEST_LOCK: Mutex<()> = Mutex::new(());
/// Held by tests that set or depend on the process-wide mobile config.
static MOBILE_CONFIG_TEST_LOCK: Mutex<()> = Mutex::new(());

unsafe extern "C" fn capture_event(event_json: *const c_char, _user_data: *mut c_void) {
    assert!(!event_json.is_null());
    let value = unsafe { CStr::from_ptr(event_json) }
        .to_string_lossy()
        .into_owned();
    CAPTURED_EVENTS.lock().unwrap().push(value);
}

unsafe fn take_owned_string(ptr: *mut c_char) -> String {
    assert!(!ptr.is_null());
    let value = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { meron_core_string_free(ptr) };
    value
}

#[test]
fn ffi_exposes_protocol_version() {
    assert_eq!(meron_core_protocol_version(), PROTOCOL_VERSION);
}

#[test]
fn ffi_ready_json_contains_protocol() {
    let raw = unsafe { take_owned_string(meron_core_ready_json()) };
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["protocol"], PROTOCOL_VERSION);
}

#[test]
fn ffi_ping_json_contains_pong() {
    let raw = unsafe { take_owned_string(meron_core_ping_json()) };
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["pong"], true);
    assert_eq!(value["protocol"], PROTOCOL_VERSION);
}

#[test]
fn ffi_invoke_json_wraps_ping() {
    let _guard = MOBILE_CONFIG_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let request = CString::new(r#"{"id":5,"method":"ping"}"#).unwrap();
    let raw = unsafe { take_owned_string(meron_core_invoke_json(request.as_ptr())) };
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["id"], 5);
    assert_eq!(value["result"]["pong"], true);
    assert_eq!(value["result"]["protocol"], PROTOCOL_VERSION);
}

#[test]
fn ffi_invoke_json_handles_null() {
    let raw = unsafe { take_owned_string(meron_core_invoke_json(ptr::null())) };
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(value["error"]["message"].as_str().unwrap().contains("null"));
}

#[test]
fn ffi_init_json_records_mobile_data_dir() {
    let _guard = MOBILE_CONFIG_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let data_dir = CString::new("/tmp/meron-mobile-test").unwrap();
    let raw = unsafe { take_owned_string(meron_core_init_json(data_dir.as_ptr())) };
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["protocol"], PROTOCOL_VERSION);
    assert_eq!(value["data_dir"], "/tmp/meron-mobile-test");
    assert_eq!(
        *MOBILE_CONFIG.lock().unwrap(),
        Some(MobileConfig {
            data_dir: "/tmp/meron-mobile-test".to_string(),
            db_key: String::new(),
        })
    );
}

#[test]
fn configuring_leaves_the_store_for_the_first_command() {
    let _guard = MOBILE_CONFIG_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = std::env::temp_dir().join(format!("meron-configure-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let data_dir = dir.to_str().unwrap();
    let configured = configure_mobile_core(data_dir, None);
    assert_eq!(configured["ok"], true);
    assert!(!dir.join("meron.db").exists());

    let ping = invoke_mobile_request_json(r#"{"id":1,"method":"ping","params":{}}"#);
    assert_eq!(ping["result"]["pong"], true);
    let status = invoke_mobile_request_json(r#"{"id":2,"method":"core.status","params":{}}"#);
    assert_eq!(status["result"]["migrating"], false);
    assert!(!dir.join("meron.db").exists());

    invoke_mobile_request_json(r#"{"id":3,"method":"account.list","params":{}}"#);
    assert!(dir.join("meron.db").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn ffi_init_json_rejects_missing_data_dir() {
    let data_dir = CString::new("  ").unwrap();
    let raw = unsafe { take_owned_string(meron_core_init_json(data_dir.as_ptr())) };
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("data_dir")
    );
}

#[test]
fn ffi_init_json_handles_null() {
    let raw = unsafe { take_owned_string(meron_core_init_json(ptr::null())) };
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(value["error"]["message"].as_str().unwrap().contains("null"));
}

#[test]
fn ffi_free_accepts_null() {
    unsafe { meron_core_string_free(ptr::null_mut()) };
}

#[test]
fn ffi_ready_event_reaches_registered_callback() {
    let _guard = EVENT_CALLBACK_TEST_LOCK.lock().unwrap();
    CAPTURED_EVENTS.lock().unwrap().clear();
    meron_core_register_event_callback(None, ptr::null_mut());
    meron_core_register_event_callback(Some(capture_event), ptr::null_mut());
    assert!(meron_core_emit_ready_event());
    meron_core_register_event_callback(None, ptr::null_mut());

    let events = CAPTURED_EVENTS.lock().unwrap();
    assert_eq!(events.len(), 1);
    let value: serde_json::Value = serde_json::from_str(&events[0]).unwrap();
    assert_eq!(value["event"], "ready");
    assert_eq!(value["detail"]["protocol"], PROTOCOL_VERSION);
}

#[test]
fn ffi_ready_event_reports_false_without_callback() {
    let _guard = EVENT_CALLBACK_TEST_LOCK.lock().unwrap();
    meron_core_register_event_callback(None, ptr::null_mut());
    CAPTURED_EVENTS.lock().unwrap().clear();
    assert!(!meron_core_emit_ready_event());
}

#[test]
fn watch_interest_is_tracked_once_per_folder_and_side() {
    let app: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    let host: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    assert!(set_watch_interest(&app, "acc", "Work", true));
    // Foregrounding again must not double-count the same watch.
    assert!(!set_watch_interest(&app, "acc", "Work", true));
    assert!(set_watch_interest(&host, "acc", "Work", true));

    // The host letting go leaves the open app's interest in place.
    assert!(set_watch_interest(&host, "acc", "Work", false));
    assert!(!watch_interest(&host, "acc", "Work"));
    assert!(watch_interest(&app, "acc", "Work"));
    assert!(!watch_interest(&app, "acc", "Other"));
    assert!(set_watch_interest(&app, "acc", "Work", false));
    assert!(!set_watch_interest(&app, "acc", "Work", false));
}
