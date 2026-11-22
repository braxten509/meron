//! "Copy setup to another device": one device's whole setup replaces the other's, once, over the local network.
//!
//! Nothing goes through the internet.
//!
//! The computer listens on its network address on a random port for five minutes and shows a QR
//! code (`meron-setup://<addresses>:<port>/<code>`) plus the address and an 8-character code to
//! type. The phone connects; the two agree on keys with SPAKE2 using the code, and each proves it
//! got the same key (HMAC) before anything else is said.
//!
//! Neither side ever sends anything a code could be guessed against offline, not even to a fake
//! computer, so a wrong code learns nothing and a listener on the network sees only ciphertext.
//! Five wrong codes end the session, and pairing must finish within the five minutes. Everything
//! after is AES-256-GCM.
//!
//! Both screens then show the same thing: which device is kept, which erased, with a swap button
//! (the computer decides; the phone asks) and a Copy button on each.
//!
//! The kept device sends a [`SetupPayload`] (the config backup with secrets, plus AI sorting keys);
//! the erased device checks it, saves one locked safety copy of its own setup, wipes its accounts,
//! cached mail, shared settings, Meron folders, tasks, calendar and tags, and applies the copy in
//! one transaction.
//!
//! Ids stay the same (Meron folder ids too, for the `$MeronF_` marks, see [`crate::meron_marks`]).
//!
//! Not copied: mail (downloads again), AI sorting history, each app's own settings (`mobile.*` on
//! the phone, the desktop's look and layout keys), and the computer's local folders, which hold the
//! only copy of their mail and are left alone.
//!
//! The screens poll `setupCopy.status`.

mod exchange;
use exchange::{run, spawn_send_payload};

mod connections;
use connections::{host, join, start_copy};
mod handshake;
use handshake::{
    b64_field, guest_handshake, host_handshake, new_code, normalize_code, read_frame, write_frame,
};
mod restoration;
pub use restoration::{collect_payload, replace_setup};
use restoration::{counts, receive, restore_safety, safety_info};

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use base64::engine::general_purpose::STANDARD;
use ring::rand::{SecureRandom, SystemRandom};
use ring::{aead, hkdf, hmac};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

use crate::backup::BackupData;
use crate::engine::Engine;
use crate::secrets::Secrets;
use crate::store;

/// How long the computer waits for the phone.
const LISTEN_FOR: Duration = Duration::from_secs(5 * 60);
/// Wrong codes before the session ends.
const MAX_ATTEMPTS: u32 = 5;
/// How long the phone tries one address.
const CONNECT_WAIT: Duration = Duration::from_secs(5);
/// The port the computer waits on (falls back to any free one).
const SETUP_COPY_PORT: u16 = 7380;
/// How long the key agreement may take.
const HANDSHAKE_WAIT: Duration = Duration::from_secs(20);
/// Largest handshake message, and largest message at all.
const MAX_HELLO: usize = 4096;
const MAX_FRAME: usize = 64 * 1024 * 1024;
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 8;
const QR_PREFIX: &str = "meron-setup://";
const PAYLOAD_VERSION: u32 = 1;
/// `meta` row holding the locked safety copy; its key is a named secret.
const SAFETY_META_KEY: &str = "setup_copy_safety";
const SAFETY_KEY_NAME: &str = "meron-setup-safety-copy";
/// The second key slot (see `save_safety`).
const SAFETY_KEY_NAME_B: &str = "meron-setup-safety-copy-b";

/// Desktop settings that are about this computer's own app (look, layout,
/// window state): neither copied nor erased. Mirrors the desktop's `DB_KEY`
/// list minus what the core itself reads (proxy, reminders, remote images).
const DESKTOP_ONLY: &[&str] = &[
    "theme_id",
    "theme_follow_system",
    "light_theme_id",
    "dark_theme_id",
    "custom_themes",
    "font_family",
    "message_font_family",
    "font_scale",
    "message_font_scale",
    "show_real_avatars",
    "dark_message_bodies",
    "auto_fit_messages",
    "reader_bottom_actions",
    "show_unread_account_badge",
    "send_shortcut",
    "conversation_layout",
    "chat_full_messages",
    "spell_check",
    "quote_replies",
    "signature",
    "kanban_boards",
    "thread_list_width",
    "kanban_pane_width",
    "kanban_column_width",
    "kanban_lock_scroll",
    "kanban_minimized_columns",
    "hidden_sidenav_accounts",
    "show_unified_inbox_in_sidenav",
    "show_calendar_in_sidenav",
    "tasks_enabled",
    "calendar_enabled",
    "close_to_tray",
    "auto_update_check",
    "dismissed_update_version",
    "language",
    "shortcut_overrides",
    "pinned_contacts",
    "contacts_sort",
];

/// Whether a `settings` key travels with a copy: not one app's own setting.
pub fn is_shared_setting(key: &str) -> bool {
    !key.starts_with("mobile.") && !key.starts_with("session_") && !DESKTOP_ONLY.contains(&key)
}

/// What one device sends the other.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SetupPayload {
    #[serde(default)]
    pub version: u32,
    /// `computer` or `phone`: who sent it.
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub backup: BackupData,
    /// AI sorting keys by provider.
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
    /// Account photos (base64) by account id. The accounts' photo links name
    /// files on the sending device, so the files travel too. Older copies
    /// have none; their links show as missing and get fetched again.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub photos: BTreeMap<String, String>,
}

/// What the screen counts on each side.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Counts {
    pub accounts: u32,
    pub folders: u32,
    pub keys: u32,
}

/// Why a step failed, as the screens name it (`setupCopy.error.<code>`).
#[derive(Debug)]
struct Failure(&'static str);

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Failure {}

fn fail(code: &'static str) -> anyhow::Error {
    anyhow::Error::new(Failure(code))
}

fn failure_code(error: &anyhow::Error) -> &'static str {
    error
        .downcast_ref::<Failure>()
        .map_or("broken", |failure| failure.0)
}

// ---- Session state ------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize)]
struct Status {
    /// `idle`, `waiting` (computer listens), `connecting` (phone), `connected`,
    /// `copying`, `applying` (erased side), `done`, `failed`.
    stage: &'static str,
    /// `host` (listens) or `guest` (connects).
    role: &'static str,
    /// This device: `computer` or `phone`.
    me: String,
    peer_device: String,
    qr: String,
    /// The QR code drawn as SVG (computer only).
    qr_svg: String,
    address: String,
    code: String,
    expires_at: i64,
    /// The device whose setup is kept.
    keep: String,
    mine: Counts,
    peer: Option<Counts>,
    /// Copy pressed here; waiting for the computer to start.
    starting: bool,
    error: String,
    attempts_left: u32,
    /// This device's setup was replaced: reload everything.
    replaced: bool,
    result: Option<Value>,
}

struct Session {
    engine: Arc<Engine>,
    host: bool,
    status: Mutex<Status>,
    out: Mutex<Option<mpsc::UnboundedSender<Value>>>,
    stop: watch::Sender<bool>,
}

static SESSION: Mutex<Option<Arc<Session>>> = Mutex::new(None);
/// The last session's final status, kept for the screen after it ends.
static LAST: Mutex<Option<Status>> = Mutex::new(None);

impl Session {
    fn update(&self, change: impl FnOnce(&mut Status)) {
        change(&mut self.status.lock().unwrap());
    }

    fn snapshot(&self) -> Status {
        self.status.lock().unwrap().clone()
    }

    fn send(&self, message: Value) {
        if let Some(out) = self.out.lock().unwrap().as_ref() {
            let _ = out.send(message);
        }
    }

    fn stop(&self) {
        let _ = self.stop.send(true);
    }

    /// End with `code`, unless it already ended well.
    fn fail(&self, code: &'static str) {
        self.update(|status| {
            if status.stage != "done" {
                status.stage = "failed";
                status.error = code.to_string();
                status.starting = false;
            }
        });
        self.stop();
    }
}

async fn stopped(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}

fn current() -> Option<Arc<Session>> {
    SESSION.lock().unwrap().clone()
}

fn status_json(status: &Status) -> Value {
    let mut value = serde_json::to_value(status).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "active".to_string(),
            json!(!matches!(status.stage, "idle" | "done" | "failed")),
        );
        object.insert("now".to_string(), json!(store::now_unix()));
    }
    value
}

fn idle_status() -> Value {
    status_json(&Status {
        stage: "idle",
        ..Default::default()
    })
}

/// End the running session, if any, telling the other device.
fn end_current() {
    if let Some(session) = SESSION.lock().unwrap().take() {
        session.send(json!({ "t": "bye" }));
        session.stop();
    }
}

/// Keep the final status of `session` for the screen once it is over.
fn retire(session: &Arc<Session>) {
    let mut slot = SESSION.lock().unwrap();
    if slot.as_ref().is_some_and(|live| Arc::ptr_eq(live, session)) {
        *LAST.lock().unwrap() = Some(session.snapshot());
        *slot = None;
    }
}

fn new_session(engine: &Arc<Engine>, host: bool, status: Status) -> Arc<Session> {
    end_current();
    *LAST.lock().unwrap() = None;
    let (stop, _) = watch::channel(false);
    let session = Arc::new(Session {
        engine: engine.clone(),
        host,
        status: Mutex::new(status),
        out: Mutex::new(None),
        stop,
    });
    *SESSION.lock().unwrap() = Some(session.clone());
    session
}

// ---- Commands -------------------------------------------------------------------

/// Dispatches `setupCopy.<command>` requests.
///
/// The caller identifies this device as `computer` (desktop) or `phone`.
///
/// | command         | params                          |
/// |-----------------|---------------------------------|
/// | `host`          | `{}` (computer: start listening)|
/// | `join`          | `{"qr"}` or `{"address","code"}`|
/// | `status`        | `{}`                            |
/// | `setKeep`       | `{"keep":"phone"|"computer"}`   |
/// | `copy`          | `{}`                            |
/// | `cancel`        | `{}`                            |
/// | `safetyInfo`    | `{}`                            |
/// | `restoreSafety` | `{}`                            |
pub async fn dispatch(
    engine: &Arc<Engine>,
    command: &str,
    params: &Value,
    device: &str,
) -> Result<Value> {
    let device = if device == "phone" {
        "phone"
    } else {
        "computer"
    };
    match command {
        "host" => host(engine, device).await,
        "join" => join(engine, params, device).await,
        "status" => Ok(status()),
        "setKeep" => {
            let keep = params
                .get("keep")
                .and_then(Value::as_str)
                .unwrap_or_default();
            set_keep(keep);
            Ok(status())
        }
        "copy" => {
            copy();
            Ok(status())
        }
        "cancel" => {
            // A copy on its way finishes: leaving the screen must not cut it.
            if current()
                .is_some_and(|session| matches!(session.snapshot().stage, "copying" | "applying"))
            {
                return Ok(status());
            }
            end_current();
            *LAST.lock().unwrap() = None;
            Ok(idle_status())
        }
        "safetyInfo" => safety_info(engine),
        "restoreSafety" => restore_safety(engine, device).await,
        other => anyhow::bail!("unknown setupCopy command: {other}"),
    }
}

fn status() -> Value {
    if let Some(session) = current() {
        return status_json(&session.snapshot());
    }
    match LAST.lock().unwrap().as_ref() {
        Some(last) => status_json(last),
        None => idle_status(),
    }
}

fn set_keep(keep: &str) {
    let Some(session) = current() else {
        return;
    };
    if keep != "phone" && keep != "computer" {
        return;
    }
    let status = session.snapshot();
    if status.stage != "connected" || status.starting {
        return;
    }
    if session.host {
        session.update(|status| status.keep = keep.to_string());
        session.send(json!({ "t": "keep", "keep": keep }));
    } else {
        // The computer decides; the screen follows its answer.
        session.send(json!({ "t": "want_keep", "keep": keep }));
    }
}

fn copy() {
    let Some(session) = current() else {
        return;
    };
    let status = session.snapshot();
    if status.stage != "connected" || status.starting {
        return;
    }
    if session.host {
        start_copy(&session);
    } else {
        session.update(|status| status.starting = true);
        session.send(json!({ "t": "go", "keep": status.keep }));
    }
}

// ---- The computer: listening ----------------------------------------------------

// ---- The phone: connecting ------------------------------------------------------

/// Whether this device reaches `host` from a private address in the same
/// /16: a rough "on the same Wi-Fi".
fn same_network(host: Ipv4Addr) -> bool {
    let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") else {
        return false;
    };
    if socket.connect((host, 9)).is_err() {
        return false;
    }
    match socket.local_addr().map(|addr| addr.ip()) {
        Ok(std::net::IpAddr::V4(mine)) => {
            mine.is_private() && mine.octets()[..2] == host.octets()[..2]
        }
        _ => false,
    }
}

// ---- Addresses and codes --------------------------------------------------------

/// This computer's private IPv4 addresses on real network interfaces (not
/// VPN tunnels, containers or loopback), the likeliest Wi-Fi one first.
fn lan_addresses() -> Vec<Ipv4Addr> {
    let interfaces = interface_addresses();
    let mut found: Vec<(String, Ipv4Addr)> = interfaces
        .clone()
        .into_iter()
        .filter(|(name, ip)| {
            let name = name.to_ascii_lowercase();
            ip.is_private()
                && ![
                    "tun",
                    "wg",
                    "mullvad",
                    "tailscale",
                    "docker",
                    "br-",
                    "veth",
                    "virbr",
                    "vmnet",
                    "lo",
                    "utun",
                    "zt",
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
        .collect();
    if found.is_empty() {
        // No interface list here (Windows): the address the default route uses.
        if let Some(ip) = route_address() {
            found.push((String::new(), ip));
        }
    }
    found.sort_by_key(|(name, ip)| {
        let wifi = name.starts_with("wl") || name.starts_with("en") || name.starts_with("eth");
        (!wifi, ip.octets()[0] != 192, ip.octets()[0] == 10)
    });
    let mut out: Vec<Ipv4Addr> = Vec::new();
    for (_, ip) in found {
        if !out.contains(&ip) {
            out.push(ip);
        }
    }
    // A Tailscale address last: it reaches this computer when the two aren't
    // on one Wi-Fi, or a VPN keeps the phone off the local network.
    for (name, ip) in interfaces {
        if name.to_ascii_lowercase().starts_with("tailscale")
            && is_tailscale(ip)
            && !out.contains(&ip)
        {
            out.push(ip);
        }
    }
    out
}

/// Tailscale hands out addresses from the shared 100.64.0.0/10 block.
fn is_tailscale(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (64..128).contains(&b)
}

fn route_address() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.168.1.1:9").ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(ip) if ip.is_private() => Some(ip),
        _ => None,
    }
}

#[cfg(unix)]
fn interface_addresses() -> Vec<(String, Ipv4Addr)> {
    let mut out = Vec::new();
    // SAFETY: getifaddrs hands back a list we only read and then free.
    unsafe {
        let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut list) != 0 {
            return out;
        }
        let mut node = list;
        while !node.is_null() {
            let item = &*node;
            let up = item.ifa_flags & (libc::IFF_UP as u32) != 0;
            let loopback = item.ifa_flags & (libc::IFF_LOOPBACK as u32) != 0;
            if up
                && !loopback
                && !item.ifa_addr.is_null()
                && i32::from((*item.ifa_addr).sa_family) == libc::AF_INET
            {
                let addr = &*(item.ifa_addr as *const libc::sockaddr_in);
                let ip = Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr));
                let name = std::ffi::CStr::from_ptr(item.ifa_name)
                    .to_string_lossy()
                    .into_owned();
                out.push((name, ip));
            }
            node = item.ifa_next;
        }
        libc::freeifaddrs(list);
    }
    out
}

#[cfg(not(unix))]
fn interface_addresses() -> Vec<(String, Ipv4Addr)> {
    Vec::new()
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn qr_svg(text: &str) -> String {
    qrcode::QrCode::new(text.as_bytes())
        .map(|code| {
            code.render::<qrcode::render::svg::Color<'_>>()
                .min_dimensions(200, 200)
                .dark_color(qrcode::render::svg::Color("#000000"))
                .light_color(qrcode::render::svg::Color("#ffffff"))
                .quiet_zone(true)
                .build()
        })
        .unwrap_or_default()
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn qr_svg(_text: &str) -> String {
    String::new()
}

// ---- Key agreement and framing --------------------------------------------------

struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

struct Keys {
    guest_to_host: [u8; 32],
    host_to_guest: [u8; 32],
    confirm: [u8; 32],
}

/// SPAKE2 (a password-authenticated key exchange): both sides mix the code
/// into their messages, so a key comes out the same only when both used the
/// same code. Someone pretending to be either side learns nothing to test
/// code guesses against later: each connection tests one guess at most.
const GUEST_ID: &[u8] = b"meron setup copy phone";
const HOST_ID: &[u8] = b"meron setup copy computer";

/// A connection after the key agreement: one key each way.
struct Channel {
    stream: TcpStream,
    send: [u8; 32],
    recv: [u8; 32],
}

/// AES-256-GCM one way, nonces counting up.
struct Sealer {
    key: aead::LessSafeKey,
    counter: u64,
}

impl Sealer {
    fn new(key: &[u8; 32]) -> Self {
        Self {
            key: aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, key).unwrap()),
            counter: 0,
        }
    }

    fn nonce(&mut self) -> aead::Nonce {
        let mut nonce = [0u8; 12];
        nonce[4..].copy_from_slice(&self.counter.to_be_bytes());
        self.counter += 1;
        aead::Nonce::assume_unique_for_key(nonce)
    }

    fn seal(&mut self, plain: &[u8]) -> Vec<u8> {
        let nonce = self.nonce();
        let mut buffer = plain.to_vec();
        let _ =
            self.key
                .seal_in_place_append_tag(nonce, aead::Aad::from(b"meron-setup"), &mut buffer);
        buffer
    }

    fn open(&mut self, mut sealed: Vec<u8>) -> Result<Vec<u8>> {
        let nonce = self.nonce();
        let plain = self
            .key
            .open_in_place(nonce, aead::Aad::from(b"meron-setup"), &mut sealed)
            .map_err(|_| fail("broken"))?;
        Ok(plain.to_vec())
    }
}

// ---- Talking ----------------------------------------------------------------------

// ---- Reading and replacing a setup ----------------------------------------------

type Applied = (Vec<String>, crate::backup::ImportSummary);

/// Puts back, when dropped before `done`, the secrets of the accounts in
/// `saved` as they were before a copy that didn't go through (one an account
/// didn't have goes). For a keychain; secrets kept in the store come back with
/// its rollback, and saving them again there is harmless.
struct RestoreSecrets<'a> {
    host: &'a dyn crate::engine::EngineHost,
    conn: &'a Connection,
    saved: Vec<String>,
    old: &'a [(String, Secrets)],
    done: bool,
}

impl Drop for RestoreSecrets<'_> {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        for id in &self.saved {
            match self.old.iter().find(|(old_id, _)| old_id == id) {
                Some((_, secret)) if !secret.is_empty() => {
                    if let Err(err) = self.host.store_secret(self.conn, id, secret) {
                        crate::mlog!(
                            crate::log::Level::Warn,
                            "setupCopy",
                            "could not put back a secret: {err:#}"
                        );
                    }
                }
                _ => self.host.delete_account_secret(id),
            }
        }
    }
}

// ---- The safety copy --------------------------------------------------------------
