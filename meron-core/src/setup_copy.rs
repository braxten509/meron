//! "Copy setup to another device": one device's whole setup replaces the
//! other's, once, over the local network. Nothing goes through the internet.
//!
//! The computer listens on its network address on a random port for five
//! minutes and shows a QR code (`meron-setup://<addresses>:<port>/<code>`) plus
//! the address and an 8-character code to type. The phone connects; the two
//! agree on keys with SPAKE2 using the code, and each proves it got the same
//! key (HMAC) before anything else is said. Neither side ever sends anything a
//! code could be guessed against offline, not even to a fake computer, so a
//! wrong code learns nothing and a listener on the network sees only
//! ciphertext. Five wrong codes end the session, and pairing must finish
//! within the five minutes. Everything after is AES-256-GCM.
//!
//! Both screens then show the same thing: which device is kept, which erased,
//! with a swap button (the computer decides; the phone asks) and a Copy button
//! on each. The kept device sends a [`SetupPayload`] (the config backup with
//! secrets, plus AI sorting keys); the erased device checks it, saves one
//! locked safety copy of its own setup, wipes its accounts, cached mail,
//! shared settings, Meron folders, tasks, calendar and tags, and applies the
//! copy in one transaction. Ids stay the same (Meron folder ids too, for the
//! `$MeronF_` marks, see [`crate::meron_marks`]).
//!
//! Not copied: mail (downloads again), AI sorting history, each app's own
//! settings (`mobile.*` on the phone, the desktop's look and layout keys), and
//! the computer's local folders, which hold the only copy of their mail and
//! are left alone.
//!
//! The screens poll `setupCopy.status`.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ring::rand::{SecureRandom, SystemRandom};
use ring::{aead, hkdf, hmac};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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

fn failure_code(err: &anyhow::Error) -> &'static str {
    err.downcast_ref::<Failure>().map_or("broken", |failure| failure.0)
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
        object.insert("active".to_string(), json!(!matches!(status.stage, "idle" | "done" | "failed")));
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

/// The `setupCopy.<command>` commands. `device` is this device: `computer`
/// (desktop) or `phone`.
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
pub async fn dispatch(engine: &Arc<Engine>, command: &str, params: &Value, device: &str) -> Result<Value> {
    let device = if device == "phone" { "phone" } else { "computer" };
    match command {
        "host" => host(engine, device).await,
        "join" => join(engine, params, device).await,
        "status" => Ok(status()),
        "setKeep" => {
            let keep = params.get("keep").and_then(Value::as_str).unwrap_or_default();
            set_keep(keep);
            Ok(status())
        }
        "copy" => {
            copy();
            Ok(status())
        }
        "cancel" => {
            // A copy on its way finishes: leaving the screen must not cut it.
            if current().is_some_and(|session| matches!(session.snapshot().stage, "copying" | "applying")) {
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

/// The computer starts the copy, both screens show it, the kept side sends.
fn start_copy(session: &Arc<Session>) {
    let status = session.snapshot();
    session.update(|status| {
        status.stage = "copying";
        status.starting = false;
    });
    session.send(json!({ "t": "start", "keep": status.keep }));
    if status.keep == status.me {
        spawn_send_payload(session.clone());
    }
}

// ---- The computer: listening ----------------------------------------------------

async fn host(engine: &Arc<Engine>, device: &str) -> Result<Value> {
    let addresses = lan_addresses();
    if addresses.is_empty() {
        let session = new_session(engine, true, Status { me: device.to_string(), ..Default::default() });
        session.fail("no_network");
        retire(&session);
        return Ok(status());
    }
    // A fixed port, so a firewall can let it in with one rule; any free port
    // when something else already holds it.
    let listener = match TcpListener::bind(("0.0.0.0", SETUP_COPY_PORT)).await {
        Ok(listener) => listener,
        Err(_) => TcpListener::bind(("0.0.0.0", 0)).await?,
    };
    let port = listener.local_addr()?.port();
    let code = new_code();
    let mine = counts(engine).await;
    let list = addresses.iter().map(Ipv4Addr::to_string).collect::<Vec<_>>().join(",");
    let qr = format!("{QR_PREFIX}{list}:{port}/{code}");
    let session = new_session(
        engine,
        true,
        Status {
            stage: "waiting",
            role: "host",
            me: device.to_string(),
            qr_svg: qr_svg(&qr),
            qr,
            address: format!("{}:{port}", addresses[0]),
            code: format!("{}-{}", &code[..4], &code[4..]),
            expires_at: store::now_unix() + LISTEN_FOR.as_secs() as i64,
            keep: device.to_string(),
            mine,
            attempts_left: MAX_ATTEMPTS,
            ..Default::default()
        },
    );
    tokio::spawn(accept_loop(session.clone(), listener, code));
    Ok(status())
}

async fn accept_loop(session: Arc<Session>, listener: TcpListener, code: String) {
    let mut stop = session.stop.subscribe();
    let deadline = tokio::time::Instant::now() + LISTEN_FOR;
    loop {
        let accepted = tokio::select! {
            _ = stopped(&mut stop) => {
                retire(&session);
                return;
            }
            _ = tokio::time::sleep_until(deadline) => {
                session.fail("expired");
                retire(&session);
                return;
            }
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, _)) = accepted else {
            continue;
        };
        let left = session.snapshot().attempts_left;
        // The five minutes hold for the pairing too: a phone that connected
        // just before the end can't finish after it.
        let limit = deadline.min(tokio::time::Instant::now() + HANDSHAKE_WAIT);
        let shaken = tokio::select! {
            _ = stopped(&mut stop) => {
                retire(&session);
                return;
            }
            shaken = tokio::time::timeout_at(limit, host_handshake(stream, &code, left)) => shaken,
        };
        if tokio::time::Instant::now() >= deadline {
            session.fail("expired");
            retire(&session);
            return;
        }
        match shaken {
            Ok(Ok(Some(channel))) => {
                // One use: nobody else gets in.
                drop(listener);
                run(session.clone(), channel).await;
                retire(&session);
                return;
            }
            Ok(Ok(None)) => {
                let left = left.saturating_sub(1);
                session.update(|status| status.attempts_left = left);
                if left == 0 {
                    session.fail("too_many_attempts");
                    retire(&session);
                    return;
                }
            }
            // Something that isn't Meron, or a phone that gave up: keep waiting.
            _ => {}
        }
    }
}

// ---- The phone: connecting ------------------------------------------------------

/// Where to connect, from a scanned QR code or the typed address and code.
fn parse_target(params: &Value) -> Result<(Vec<Ipv4Addr>, u16, String)> {
    let qr = params.get("qr").and_then(Value::as_str).unwrap_or_default().trim();
    let (hosts, port, code) = if !qr.is_empty() {
        let rest = qr.strip_prefix(QR_PREFIX).ok_or_else(|| fail("bad_qr"))?;
        let (address, code) = rest.split_once('/').ok_or_else(|| fail("bad_qr"))?;
        let (hosts, port) = address.rsplit_once(':').ok_or_else(|| fail("bad_qr"))?;
        (hosts.to_string(), port.to_string(), code.to_string())
    } else {
        let address = params.get("address").and_then(Value::as_str).unwrap_or_default().trim();
        let code = params.get("code").and_then(Value::as_str).unwrap_or_default();
        let (hosts, port) = address.rsplit_once(':').ok_or_else(|| fail("bad_address"))?;
        (hosts.to_string(), port.to_string(), code.to_string())
    };
    let port: u16 = port.trim().parse().map_err(|_| fail("bad_address"))?;
    let hosts: Vec<Ipv4Addr> = hosts
        .split(',')
        .filter_map(|host| host.trim().parse().ok())
        .collect();
    if hosts.is_empty() || port == 0 {
        return Err(fail("bad_address"));
    }
    let code = normalize_code(&code);
    if code.len() != CODE_LEN {
        return Err(fail("bad_code"));
    }
    Ok((hosts, port, code))
}

async fn join(engine: &Arc<Engine>, params: &Value, device: &str) -> Result<Value> {
    let (hosts, port, code) = match parse_target(params) {
        Ok(target) => target,
        Err(err) => {
            let session = new_session(engine, false, Status { me: device.to_string(), role: "guest", ..Default::default() });
            session.fail(failure_code(&err));
            retire(&session);
            return Ok(status());
        }
    };
    let mine = counts(engine).await;
    let session = new_session(
        engine,
        false,
        Status {
            stage: "connecting",
            role: "guest",
            me: device.to_string(),
            mine,
            ..Default::default()
        },
    );
    tokio::spawn(async move {
        let mut stop = session.stop.subscribe();
        let connected = tokio::select! {
            _ = stopped(&mut stop) => {
                retire(&session);
                return;
            }
            connected = connect_any(&hosts, port) => connected,
        };
        let stream = match connected {
            Ok(stream) => stream,
            Err(err) => {
                session.fail(failure_code(&err));
                retire(&session);
                return;
            }
        };
        match tokio::time::timeout(HANDSHAKE_WAIT, guest_handshake(stream, &code)).await {
            Ok(Ok(channel)) => run(session.clone(), channel).await,
            Ok(Err(err)) => session.fail(failure_code(&err)),
            Err(_) => session.fail("unreachable"),
        }
        retire(&session);
    });
    Ok(status())
}

/// Connect to the first address that answers, those on this phone's own
/// network first. Fails with `refused` (nothing listens: the computer stopped
/// waiting, or a firewall said no), `other_network` (this phone isn't on a
/// local network the computer could be on) or `unreachable` (no answer: most
/// likely the computer's firewall).
async fn connect_any(hosts: &[Ipv4Addr], port: u16) -> Result<TcpStream> {
    let mut ordered: Vec<Ipv4Addr> = hosts.to_vec();
    ordered.sort_by_key(|host| !same_network(*host));
    let mut refused = false;
    for host in &ordered {
        let addr = SocketAddr::from((*host, port));
        match tokio::time::timeout(CONNECT_WAIT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(err)) if err.kind() == std::io::ErrorKind::ConnectionRefused => refused = true,
            _ => {}
        }
    }
    if refused {
        return Err(fail("refused"));
    }
    if !ordered.iter().any(|host| same_network(*host)) {
        return Err(fail("other_network"));
    }
    Err(fail("unreachable"))
}

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
                && !["tun", "wg", "mullvad", "tailscale", "docker", "br-", "veth", "virbr", "vmnet", "lo", "utun", "zt"]
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
        if name.to_ascii_lowercase().starts_with("tailscale") && is_tailscale(ip) && !out.contains(&ip) {
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
                let name = std::ffi::CStr::from_ptr(item.ifa_name).to_string_lossy().into_owned();
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

fn new_code() -> String {
    let rng = SystemRandom::new();
    let mut bytes = [0u8; CODE_LEN];
    let _ = rng.fill(&mut bytes);
    bytes
        .iter()
        .map(|byte| CODE_ALPHABET[(*byte as usize) % CODE_ALPHABET.len()] as char)
        .collect()
}

/// A typed code as the computer made it: capitals, no dashes or spaces.
fn normalize_code(code: &str) -> String {
    code.chars()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| CODE_ALPHABET.contains(&(*c as u8)))
        .collect()
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

fn spake_start(code: &str, host: bool) -> (Spake2<Ed25519Group>, Vec<u8>) {
    let password = Password::new(code.as_bytes());
    let (guest, host_id) = (Identity::new(GUEST_ID), Identity::new(HOST_ID));
    if host {
        Spake2::<Ed25519Group>::start_b(&password, &guest, &host_id)
    } else {
        Spake2::<Ed25519Group>::start_a(&password, &guest, &host_id)
    }
}

/// The channel's keys from SPAKE2's shared key.
fn derive(shared: &[u8], transcript: &[u8]) -> Result<Keys> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, transcript).extract(shared);
    let mut out = [0u8; 96];
    prk.expand(&[b"meron setup copy v2"], Len(96))
        .and_then(|okm| okm.fill(&mut out))
        .map_err(|_| anyhow!("key derivation failed"))?;
    let mut keys = Keys {
        guest_to_host: [0; 32],
        host_to_guest: [0; 32],
        confirm: [0; 32],
    };
    keys.guest_to_host.copy_from_slice(&out[..32]);
    keys.host_to_guest.copy_from_slice(&out[32..64]);
    keys.confirm.copy_from_slice(&out[64..]);
    Ok(keys)
}

fn proof(keys: &Keys, who: &[u8], transcript: &[u8]) -> Vec<u8> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, &keys.confirm);
    let mut message = who.to_vec();
    message.extend_from_slice(transcript);
    hmac::sign(&key, &message).as_ref().to_vec()
}

fn proof_ok(keys: &Keys, who: &[u8], transcript: &[u8], tag: &[u8]) -> bool {
    let key = hmac::Key::new(hmac::HMAC_SHA256, &keys.confirm);
    let mut message = who.to_vec();
    message.extend_from_slice(transcript);
    hmac::verify(&key, &message, tag).is_ok()
}

fn b64_field(value: &Value, key: &str) -> Result<Vec<u8>> {
    let text = value.get(key).and_then(Value::as_str).ok_or_else(|| fail("broken"))?;
    STANDARD.decode(text).map_err(|_| fail("broken"))
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    writer.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
    writer.write_all(bytes).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R, max: usize) -> Result<Vec<u8>> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > max {
        return Err(fail("broken"));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_json<W: AsyncWrite + Unpin>(writer: &mut W, value: &Value) -> Result<()> {
    write_frame(writer, value.to_string().as_bytes()).await
}

async fn read_json<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Value> {
    let bytes = read_frame(reader, MAX_HELLO).await?;
    serde_json::from_slice(&bytes).map_err(|_| fail("broken"))
}

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
        let mut buf = plain.to_vec();
        let _ = self
            .key
            .seal_in_place_append_tag(nonce, aead::Aad::from(b"meron-setup"), &mut buf);
        buf
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

/// The computer's side. `None`: the phone's code was wrong.
async fn host_handshake(mut stream: TcpStream, code: &str, left: u32) -> Result<Option<Channel>> {
    let hello = read_json(&mut stream).await?;
    if hello.get("meron_setup").and_then(Value::as_u64) != Some(2) {
        return Err(fail("broken"));
    }
    let guest_message = b64_field(&hello, "spake")?;
    let (state, message) = spake_start(code, true);
    write_json(&mut stream, &json!({ "spake": STANDARD.encode(&message) })).await?;
    let transcript = [guest_message.as_slice(), message.as_slice()].concat();
    // A message that isn't a valid point can't come from a real phone.
    let shared = state.finish(&guest_message).map_err(|_| fail("broken"))?;
    let keys = derive(&shared, &transcript)?;
    let reply = read_json(&mut stream).await?;
    let tag = b64_field(&reply, "proof")?;
    if !proof_ok(&keys, b"guest", &transcript, &tag) {
        let _ = write_json(
            &mut stream,
            &json!({ "error": "wrong_code", "left": left.saturating_sub(1) }),
        )
        .await;
        return Ok(None);
    }
    write_json(
        &mut stream,
        &json!({ "proof": STANDARD.encode(proof(&keys, b"host", &transcript)) }),
    )
    .await?;
    Ok(Some(Channel {
        stream,
        send: keys.host_to_guest,
        recv: keys.guest_to_host,
    }))
}

/// The phone's side.
async fn guest_handshake(mut stream: TcpStream, code: &str) -> Result<Channel> {
    let (state, message) = spake_start(code, false);
    write_json(&mut stream, &json!({ "meron_setup": 2, "spake": STANDARD.encode(&message) })).await?;
    let answer = read_json(&mut stream).await?;
    let host_message = b64_field(&answer, "spake")?;
    let transcript = [message.as_slice(), host_message.as_slice()].concat();
    let shared = state.finish(&host_message).map_err(|_| fail("broken"))?;
    let keys = derive(&shared, &transcript)?;
    write_json(
        &mut stream,
        &json!({ "proof": STANDARD.encode(proof(&keys, b"guest", &transcript)) }),
    )
    .await?;
    let reply = read_json(&mut stream).await.map_err(|_| fail("wrong_code"))?;
    if reply.get("error").and_then(Value::as_str) == Some("wrong_code") {
        return Err(fail(if reply.get("left").and_then(Value::as_u64) == Some(0) {
            "too_many_attempts"
        } else {
            "wrong_code"
        }));
    }
    let tag = b64_field(&reply, "proof")?;
    if !proof_ok(&keys, b"host", &transcript, &tag) {
        return Err(fail("wrong_code"));
    }
    Ok(Channel {
        stream,
        send: keys.guest_to_host,
        recv: keys.host_to_guest,
    })
}

// ---- Talking ----------------------------------------------------------------------

async fn run(session: Arc<Session>, channel: Channel) {
    let Channel { stream, send, recv } = channel;
    let (mut reader, mut writer) = stream.into_split();
    let (out, mut queue) = mpsc::unbounded_channel::<Value>();
    *session.out.lock().unwrap() = Some(out);
    let mut sealer = Sealer::new(&send);
    let mut opener = Sealer::new(&recv);
    let writing = tokio::spawn(async move {
        while let Some(message) = queue.recv().await {
            let sealed = sealer.seal(message.to_string().as_bytes());
            if write_frame(&mut writer, &sealed).await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });
    session.update(|status| {
        status.stage = "connected";
        status.error.clear();
    });
    let hello = session.snapshot();
    session.send(json!({ "t": "hello", "device": hello.me, "counts": hello.mine }));
    let mut stop = session.stop.subscribe();
    loop {
        let frame = tokio::select! {
            _ = stopped(&mut stop) => break,
            frame = read_frame(&mut reader, MAX_FRAME) => frame,
        };
        let message = frame.and_then(|frame| opener.open(frame)).and_then(|plain| {
            serde_json::from_slice::<Value>(&plain).map_err(|_| fail("broken"))
        });
        match message {
            Ok(message) => handle(&session, message),
            Err(_) => {
                let stage = session.snapshot().stage;
                // After its answer the other side hangs up: not a failure.
                if !matches!(stage, "done" | "applying" | "failed") {
                    session.fail("peer_left");
                }
                break;
            }
        }
    }
    // Let the writer send what is queued (a goodbye, an answer), then close.
    *session.out.lock().unwrap() = None;
    let _ = tokio::time::timeout(Duration::from_secs(5), writing).await;
}

fn handle(session: &Arc<Session>, message: Value) {
    let kind = message.get("t").and_then(Value::as_str).unwrap_or_default();
    let text = |key: &str| message.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    match kind {
        "hello" => {
            let device = text("device");
            let counts: Counts = message
                .get("counts")
                .and_then(|counts| serde_json::from_value(counts.clone()).ok())
                .unwrap_or_default();
            session.update(|status| {
                status.peer_device = device.clone();
                status.peer = Some(counts);
            });
            if session.host {
                // Keep the side with more accounts; the computer on a tie.
                let status = session.snapshot();
                let keep = if counts.accounts > status.mine.accounts {
                    device
                } else if counts.accounts < status.mine.accounts {
                    status.me.clone()
                } else {
                    "computer".to_string()
                };
                session.update(|status| status.keep = keep.clone());
                session.send(json!({ "t": "keep", "keep": keep }));
            }
        }
        "keep" if !session.host => {
            let keep = text("keep");
            session.update(|status| status.keep = keep);
        }
        "want_keep" if session.host => set_keep(&text("keep")),
        "go" if session.host => {
            let status = session.snapshot();
            if status.stage == "connected" && text("keep") == status.keep {
                start_copy(session);
            } else {
                // The direction changed meanwhile: show it, ask again.
                session.send(json!({ "t": "keep", "keep": status.keep }));
                session.send(json!({ "t": "not_started" }));
            }
        }
        "not_started" => session.update(|status| status.starting = false),
        "start" if !session.host => {
            let keep = text("keep");
            session.update(|status| {
                status.keep = keep.clone();
                status.stage = "copying";
                status.starting = false;
            });
            if keep == session.snapshot().me {
                spawn_send_payload(session.clone());
            }
        }
        "data" => {
            let status = session.snapshot();
            if status.stage != "copying" || status.keep == status.me {
                return;
            }
            session.update(|status| status.stage = "applying");
            let payload = message.get("payload").cloned().unwrap_or(Value::Null);
            let session = session.clone();
            tokio::spawn(async move {
                let me = session.snapshot().me;
                match receive(&session.engine, payload, &me).await {
                    Ok(result) => {
                        session.update(|status| {
                            status.stage = "done";
                            status.replaced = true;
                            status.result = Some(result.clone());
                        });
                        session.send(json!({ "t": "applied", "result": result }));
                    }
                    Err(err) => {
                        let code = failure_code(&err);
                        crate::mlog!(crate::log::Level::Warn, "setupCopy", "the copy was not applied: {err:#}");
                        session.send(json!({ "t": "failed", "error": code }));
                        session.fail(code);
                    }
                }
                session.stop();
            });
        }
        "applied" => {
            let result = message.get("result").cloned();
            session.update(|status| {
                status.stage = "done";
                status.result = result;
            });
            session.stop();
        }
        "failed" => {
            let code = match message.get("error").and_then(Value::as_str) {
                Some("bad_copy") => "peer_bad_copy",
                _ => "peer_failed",
            };
            session.fail(code);
        }
        "bye" => {
            if !matches!(session.snapshot().stage, "done" | "applying") {
                session.fail("peer_left");
            }
        }
        _ => {}
    }
}

fn spawn_send_payload(session: Arc<Session>) {
    tokio::spawn(async move {
        let me = session.snapshot().me;
        match collect_payload(&session.engine, &me).await {
            Ok(payload) => match serde_json::to_value(&payload) {
                Ok(value) => session.send(json!({ "t": "data", "payload": value })),
                Err(_) => {
                    session.send(json!({ "t": "failed", "error": "broken" }));
                    session.fail("collect_failed");
                }
            },
            Err(err) => {
                crate::mlog!(crate::log::Level::Warn, "setupCopy", "could not read this setup: {err:#}");
                session.send(json!({ "t": "failed", "error": "broken" }));
                session.fail("collect_failed");
            }
        }
    });
}

// ---- Reading and replacing a setup ----------------------------------------------

async fn counts(engine: &Arc<Engine>) -> Counts {
    let (accounts, folders) = {
        let db = engine.db.lock().unwrap();
        let accounts: u32 = db
            .query_row("SELECT COUNT(*) FROM accounts WHERE engine <> 'local'", [], |row| row.get(0))
            .unwrap_or(0);
        let folders = store::list_meron_folders(&db).map(|folders| folders.len() as u32).unwrap_or(0);
        (accounts, folders)
    };
    let keys = crate::engine::ai_sort::loaded_keys(engine).await.len() as u32;
    Counts {
        accounts,
        folders,
        keys,
    }
}

/// This device's setup as a copy carries it: accounts with their secrets,
/// shared settings, Meron folders, tasks, calendar, tags, and the AI keys.
pub async fn collect_payload(engine: &Arc<Engine>, device: &str) -> Result<SetupPayload> {
    let ids: Vec<String> = {
        let db = engine.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT id FROM accounts WHERE engine <> 'local'")?;
        stmt.query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    let keys = crate::engine::ai_sort::loaded_keys(engine).await;
    let engine = engine.clone();
    let device = device.to_string();
    tokio::task::spawn_blocking(move || {
        // Secrets first, without the store's lock (a keychain can be slow).
        // One that can't be read stops it: a copy (or safety copy) without
        // that password would lose it.
        let secrets: HashMap<String, Secrets> = ids
            .into_iter()
            .map(|id| {
                let secret = engine.host.load_account_secret(&engine.db, &id)?;
                anyhow::Ok((id, secret))
            })
            .collect::<Result<_>>()?;
        let mut backup = crate::backup::collect(&engine.db.lock().unwrap(), true, &|id| {
            secrets.get(id).cloned().unwrap_or_default()
        })?;
        backup.settings.retain(|key, _| is_shared_setting(key));
        let photo_root = engine.host.account_media_root();
        let mut photo_bytes = 0usize;
        let photos = backup
            .accounts
            .iter()
            .filter_map(|account| {
                let bytes = crate::account_photos::read_for_copy(&photo_root, &account.avatar_url)?;
                photo_bytes += bytes.len();
                (photo_bytes <= crate::account_photos::COPY_TOTAL_BYTES)
                    .then(|| (account.id.clone(), STANDARD.encode(bytes)))
            })
            .collect();
        Ok(SetupPayload {
            version: PAYLOAD_VERSION,
            device,
            backup,
            keys: keys.into_iter().collect(),
            photos,
        })
    })
    .await?
}

/// The erased side: check the copy, then replace this setup with it.
async fn receive(engine: &Arc<Engine>, payload: Value, device: &str) -> Result<Value> {
    let payload: SetupPayload = serde_json::from_value(payload).map_err(|_| fail("bad_copy"))?;
    if payload.version != PAYLOAD_VERSION || payload.device == device {
        return Err(fail("bad_copy"));
    }
    if payload.backup.accounts.iter().any(|account| account.id.trim().is_empty()) {
        return Err(fail("bad_copy"));
    }
    replace_setup(engine, payload, device).await
}

/// Save a safety copy of this setup, wipe it and apply `payload`. Nothing is
/// erased when the safety copy can't be saved; the wipe and the apply are one
/// transaction.
pub async fn replace_setup(engine: &Arc<Engine>, payload: SetupPayload, device: &str) -> Result<Value> {
    save_safety(engine, device).await.map_err(|err| {
        crate::mlog!(crate::log::Level::Warn, "setupCopy", "no safety copy, nothing erased: {err:#}");
        fail("safety_failed")
    })?;
    // Today's account secrets, read before anything changes, to put back if
    // the new ones can't all be saved. One that can't be read stops the copy
    // (nothing erased): putting "none" back would delete that password.
    let old_secrets: Vec<(String, Secrets)> = {
        let engine = engine.clone();
        tokio::task::spawn_blocking(move || {
            let ids: Vec<String> = {
                let db = engine.db.lock().unwrap();
                let mut statement = db.prepare("SELECT id FROM accounts WHERE engine <> 'local'")?;
                let ids = statement
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                ids
            };
            ids.into_iter()
                .map(|id| {
                    let secret = engine.host.load_account_secret(&engine.db, &id)?;
                    anyhow::Ok((id, secret))
                })
                .collect::<Result<Vec<_>>>()
        })
        .await
        .map_err(anyhow::Error::from)
        .and_then(|read| read)
        .map_err(|err| {
            crate::mlog!(crate::log::Level::Warn, "setupCopy", "could not read this device's passwords, nothing erased: {err:#}");
            fail("apply_failed")
        })?
    };
    let applied = {
        let engine = engine.clone();
        let backup = payload.backup.clone();
        tokio::task::spawn_blocking(move || {
            wipe_and_apply(&engine.db.lock().unwrap(), engine.host.as_ref(), &backup, &old_secrets)
        })
        .await
    };
    // A crashed apply counts as failed: the transaction never committed.
    let (old_ids, summary) = applied.map_err(anyhow::Error::from).and_then(|applied| applied).map_err(|err| {
        crate::mlog!(crate::log::Level::Warn, "setupCopy", "applying the copy failed, nothing changed: {err:#}");
        fail("apply_failed")
    })?;
    // The old accounts' warm state goes; same ids load their new settings.
    for id in &old_ids {
        engine.accounts.lock().await.remove(id);
        engine.clear_pool(id);
        crate::parse::remove_account_media(&engine.host.media_root(), id);
    }
    // The photos came along: save them here and point the accounts at them.
    for (id, data) in &payload.photos {
        if !payload.backup.accounts.iter().any(|account| &account.id == id) {
            continue;
        }
        let saved = STANDARD
            .decode(data.as_bytes())
            .map_err(anyhow::Error::from)
            .and_then(|bytes| crate::account_photos::apply_copied(engine, id, &bytes));
        if let Err(err) = saved {
            crate::mlog!(crate::log::Level::Warn, "setupCopy", "could not save the photo of {id}: {err:#}");
        }
    }
    // Load the copied accounts with their sign-ins now: the account list is
    // read right after the copy, and an account not loaded yet shows as
    // "needs reconnect" until the next restart.
    {
        let loaded = store::load_accounts(&engine.db.lock().unwrap()).unwrap_or_default();
        let mut accounts = engine.accounts.lock().await;
        for (id, mut creds) in loaded {
            engine.host.apply_secret(&engine.db.lock().unwrap(), &id, &mut creds);
            accounts.insert(id, creds);
        }
    }
    let mut secret_failures = 0;
    for provider in crate::ai_sort::PROVIDERS {
        let key = payload.keys.get(provider).cloned();
        if let Err(err) = crate::engine::ai_sort::store_key(engine, provider, key).await {
            secret_failures += 1;
            crate::mlog!(crate::log::Level::Warn, "setupCopy", "could not save the {provider} key: {err:#}");
        }
    }
    {
        let db = engine.db.lock().unwrap();
        let _ = crate::proxy::load_global(&db);
    }
    // Photos that didn't come along (an older device sent the copy): ask
    // Google or Microsoft for them, behind the copy.
    {
        let engine = engine.clone();
        crate::account_photos::forget_tried();
        tokio::spawn(async move {
            crate::account_photos::fetch_missing(&engine).await;
        });
    }
    // The phone's screens listen for these (a no-op on the desktop).
    crate::ffi::emit_event("calendar.changed", json!({ "method": "setupCopy" }));
    crate::ffi::emit_event("tasks.changed", json!({ "method": "setupCopy" }));
    Ok(json!({
        "accounts": summary.accounts,
        "folders": summary.meron_folders,
        "keys": payload.keys.len(),
        "tasks": summary.tasks,
        "events": summary.events,
        "secret_failures": secret_failures,
    }))
}

type Applied = (Vec<String>, crate::backup::ImportSummary);

/// Wipe this device's setup and apply `data`, in one transaction, with every
/// account secret saved before it commits (in the same transaction where the
/// store keeps them, else in the keychain). When a secret can't be saved, or
/// the commit fails, the old secrets go back and nothing changes. Answers
/// with the old account ids.
fn wipe_and_apply(
    conn: &Connection,
    host: &dyn crate::engine::EngineHost,
    data: &BackupData,
    old_secrets: &[(String, Secrets)],
) -> Result<Applied> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let old_ids: Vec<String> = tx
        .prepare("SELECT id FROM accounts WHERE engine <> 'local'")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for id in &old_ids {
        store::delete_account_in(&tx, id)?;
    }
    let keys: Vec<String> = tx
        .prepare("SELECT key FROM settings")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for key in keys.iter().filter(|key| is_shared_setting(key)) {
        tx.execute("DELETE FROM settings WHERE key = ?1", params![key])?;
    }
    for table in [
        "meron_folder_aliases",
        "meron_folder_uids",
        "meron_folder_entries",
        "meron_folders",
        "meron_marks",
        "tasks",
        "task_lists",
        "events",
    ] {
        tx.execute(&format!("DELETE FROM {table}"), [])?;
    }
    let mut data = data.clone();
    data.settings.retain(|key, _| is_shared_setting(key));
    let secrets: RefCell<Vec<(String, Secrets)>> = RefCell::new(Vec::new());
    let summary = crate::backup::apply_in(&tx, &data, &|_, id, secret| {
        secrets.borrow_mut().push((id.to_string(), secret.clone()));
        Ok(())
    })?;
    let secrets = secrets.into_inner();
    // From here on, every way out but a commit (an error, a panic) puts the
    // old secrets back; the transaction rolls back by itself.
    let mut restore = RestoreSecrets {
        host,
        conn,
        saved: Vec::new(),
        old: old_secrets,
        done: false,
    };
    for (id, secret) in &secrets {
        restore.saved.push(id.clone());
        host.store_secret(&tx, id, secret)
            .map_err(|err| err.context("a password or sign-in token could not be saved"))?;
    }
    tx.commit()?;
    restore.done = true;
    // Old accounts the copy doesn't have: their secrets go.
    for id in old_ids.iter().filter(|id| !secrets.iter().any(|(new, _)| new == *id)) {
        host.delete_account_secret(id);
    }
    Ok((old_ids, summary))
}

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
                        crate::mlog!(crate::log::Level::Warn, "setupCopy", "could not put back a secret: {err:#}");
                    }
                }
                _ => self.host.delete_account_secret(id),
            }
        }
    }
}

// ---- The safety copy --------------------------------------------------------------

/// Save this setup, locked with a fresh key kept with the other secrets. One
/// copy: the next replaces it.
async fn save_safety(engine: &Arc<Engine>, device: &str) -> Result<()> {
    let payload = collect_payload(engine, device).await?;
    let mine = counts(engine).await;
    let plain = serde_json::to_vec(&payload)?;
    let rng = SystemRandom::new();
    let mut key = [0u8; 32];
    let mut nonce = [0u8; 12];
    rng.fill(&mut key).map_err(|_| anyhow!("no randomness"))?;
    rng.fill(&mut nonce).map_err(|_| anyhow!("no randomness"))?;
    let sealing = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_256_GCM, &key).unwrap());
    let mut sealed = plain;
    sealing
        .seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"meron-safety"),
            &mut sealed,
        )
        .map_err(|_| anyhow!("could not lock the safety copy"))?;
    // Two key slots, used in turn: the new key goes in the slot the saved copy
    // doesn't use, and the copy points at its slot. Until the new copy is
    // written (one row, one step) the old one still opens with its own key.
    let old_slot = safety_row(&engine.db.lock().unwrap())?.map(|row| safety_slot(&row));
    let slot = if old_slot.as_deref() == Some(SAFETY_KEY_NAME) {
        SAFETY_KEY_NAME_B
    } else {
        SAFETY_KEY_NAME
    };
    let engine2 = engine.clone();
    let encoded_key = STANDARD.encode(key);
    tokio::task::spawn_blocking(move || engine2.host.store_named_secret(&engine2.db, slot, Some(&encoded_key)))
        .await??;
    let row = json!({
        "created_at": store::now_unix(),
        "counts": mine,
        "slot": slot,
        "nonce": STANDARD.encode(nonce),
        "data": STANDARD.encode(&sealed),
    });
    engine.db.lock().unwrap().execute(
        "INSERT INTO meta(key, value) VALUES(?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SAFETY_META_KEY, row.to_string()],
    )?;
    // The old copy is gone: so is its key. Best effort.
    if let Some(old) = old_slot.filter(|old| old != slot) {
        let engine2 = engine.clone();
        let _ = tokio::task::spawn_blocking(move || engine2.host.store_named_secret(&engine2.db, &old, None)).await;
    }
    Ok(())
}

/// The key slot a saved safety copy's key is in (older copies: the first).
fn safety_slot(row: &Value) -> String {
    match row.get("slot").and_then(Value::as_str) {
        Some(SAFETY_KEY_NAME_B) => SAFETY_KEY_NAME_B.to_string(),
        _ => SAFETY_KEY_NAME.to_string(),
    }
}

fn safety_row(conn: &Connection) -> Result<Option<Value>> {
    let text: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", params![SAFETY_META_KEY], |row| row.get(0))
        .optional()?;
    Ok(text.and_then(|text| serde_json::from_str(&text).ok()))
}

fn safety_info(engine: &Arc<Engine>) -> Result<Value> {
    let row = safety_row(&engine.db.lock().unwrap())?;
    Ok(match row {
        Some(row) => json!({
            "exists": true,
            "created_at": row.get("created_at").cloned().unwrap_or(json!(0)),
            "counts": row.get("counts").cloned().unwrap_or(json!({})),
        }),
        None => json!({ "exists": false }),
    })
}

/// Put the safety copy back. The setup it replaces becomes the safety copy,
/// so this can be undone the same way.
async fn restore_safety(engine: &Arc<Engine>, device: &str) -> Result<Value> {
    if current().is_some() {
        anyhow::bail!("Finish or cancel the copy first.");
    }
    let row = safety_row(&engine.db.lock().unwrap())?.ok_or_else(|| anyhow!("There is no safety copy."))?;
    let engine2 = engine.clone();
    let slot = safety_slot(&row);
    let key = tokio::task::spawn_blocking(move || engine2.host.load_named_secret(&engine2.db, &slot))
        .await??
        .ok_or_else(|| anyhow!("The safety copy's key is missing."))?;
    let key = STANDARD.decode(key)?;
    let nonce: [u8; 12] = b64_field(&row, "nonce")?
        .try_into()
        .map_err(|_| anyhow!("The safety copy is damaged."))?;
    let mut sealed = b64_field(&row, "data")?;
    let opening = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &key).map_err(|_| anyhow!("The safety copy is damaged."))?,
    );
    let plain = opening
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"meron-safety"),
            &mut sealed,
        )
        .map_err(|_| anyhow!("The safety copy is damaged."))?;
    let payload: SetupPayload = serde_json::from_slice(plain)?;
    let result = replace_setup(engine, payload, device).await?;
    Ok(json!({ "ok": true, "replaced": true, "result": result }))
}
