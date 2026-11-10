//! Account photos (`/media/avatars/...`): whether this device really has the
//! file behind one, carrying them in a setup copy, and getting a missing one
//! back from the provider. A photo link copied from another device names a
//! file only that device has; without this the account showed an empty circle.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::engine::Engine;

/// Bigger photos are not carried in a copy.
pub const COPY_MAX_BYTES: usize = 5 * 1024 * 1024;
/// All photos in one copy together (base64 adds a third; a copy's frame
/// holds 64 MiB). Past it, the rest are fetched again on the other side.
pub const COPY_TOTAL_BYTES: usize = 16 * 1024 * 1024;

/// The file behind a local photo link under `root`, or `None` for a link that
/// is not local (`https://...`, empty) or tries to leave the root.
pub fn local_file(root: &Path, url: &str) -> Option<PathBuf> {
    let relative = url.trim().strip_prefix("/media/")?;
    if relative.is_empty() || relative.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return None;
    }
    Some(root.join(relative))
}

/// Whether `url` is a local photo link this device has no file for.
pub fn is_missing(root: &Path, url: &str) -> bool {
    let url = url.trim();
    url.starts_with("/media/") && local_file(root, url).is_none_or(|file| !file.is_file())
}

/// Blank the photo links in an account list that name files this device
/// doesn't have, so those accounts show their letters.
pub fn hide_missing(root: &Path, accounts: &mut [serde_json::Value]) {
    for account in accounts {
        let missing = account
            .get("avatar_url")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|url| is_missing(root, url));
        if missing && let Some(obj) = account.as_object_mut() {
            obj.insert("avatar_url".to_string(), serde_json::Value::String(String::new()));
        }
    }
}

/// The picture type of `bytes`, from its first bytes; `None` when it is none
/// of the kinds an account photo is saved as.
pub fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Save a photo for `account` under `root` and return its `/media/...` link.
pub fn save(root: &Path, account: &str, ext: &str, bytes: &[u8]) -> std::io::Result<String> {
    let segment: String = account
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' { ch } else { '_' })
        .collect();
    let segment = if segment.is_empty() { "account".to_string() } else { segment };
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
    let dir = root.join("avatars").join(&segment);
    std::fs::create_dir_all(&dir)?;
    let name = format!("{stamp}.{ext}");
    std::fs::write(dir.join(&name), bytes)?;
    Ok(format!("/media/avatars/{segment}/{name}"))
}

/// The photo behind `url` on this device, for a copy: `None` when it is not
/// local, missing, too big, or not a picture.
pub fn read_for_copy(root: &Path, url: &str) -> Option<Vec<u8>> {
    let file = local_file(root, url)?;
    let size = std::fs::metadata(&file).ok()?.len();
    if size == 0 || size > COPY_MAX_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    image_ext(&bytes)?;
    Some(bytes)
}

fn set_avatar(engine: &Engine, account: &str, url: &str) -> rusqlite::Result<usize> {
    engine.db.lock().unwrap().execute(
        "UPDATE accounts SET avatar_url = ?1, updated_at = strftime('%s', 'now') WHERE id = ?2",
        rusqlite::params![url, account],
    )
}

/// Save a photo carried by a copy and point the account at it.
pub fn apply_copied(engine: &Engine, account: &str, bytes: &[u8]) -> anyhow::Result<()> {
    let ext = image_ext(bytes).ok_or_else(|| anyhow::anyhow!("not a picture"))?;
    let url = save(&engine.host.account_media_root(), account, ext, bytes)?;
    set_avatar(engine, account, &url)?;
    Ok(())
}

static TRIED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Let [`fetch_missing`] try every account again (a copy brought new ones).
pub fn forget_tried() {
    *TRIED.lock().unwrap() = None;
}

/// Gmail and Outlook accounts whose photo is missing on this device (none, or
/// a link to a file it doesn't have) get it from the provider again. Each
/// account is tried once per run, so one without a photo costs one request.
/// Answers the accounts that got one; `engine.account_changed` hears each.
pub async fn fetch_missing(engine: &Arc<Engine>) -> Vec<String> {
    let root = engine.host.account_media_root();
    let candidates: Vec<(String, String)> = {
        let accounts = engine.accounts.lock().await;
        let db = engine.db.lock().unwrap();
        accounts
            .iter()
            .filter(|(_, creds)| creds.is_oauth())
            .filter(|(id, _)| {
                let url: String = db
                    .query_row("SELECT avatar_url FROM accounts WHERE id = ?1", [id.as_str()], |row| row.get(0))
                    .unwrap_or_default();
                url.trim().is_empty() || is_missing(&root, &url)
            })
            .map(|(id, _)| {
                let url: String = db
                    .query_row("SELECT avatar_url FROM accounts WHERE id = ?1", [id.as_str()], |row| row.get(0))
                    .unwrap_or_default();
                (id.clone(), url)
            })
            .collect()
    };
    let mut changed = Vec::new();
    for (account, seen) in candidates {
        if !TRIED.lock().unwrap().get_or_insert_with(HashSet::new).insert(account.clone()) {
            continue;
        }
        let creds = match engine.ensure_valid_creds(&account).await {
            Ok(creds) => creds,
            Err(err) => {
                crate::mlog!(crate::log::Level::Info, "avatar", "no sign-in to fetch the photo of {account}: {err:#}");
                continue;
            }
        };
        let outlook = creds.auth_type == "outlook_oauth";
        let app = crate::engine::default_oauth_client(if outlook { "outlook" } else { "gmail" });
        let own = creds.oauth_client_id.trim();
        let client = crate::oauth::Client {
            provider: app.provider.clone(),
            client_id: if own.is_empty() { app.client_id.clone() } else { own.to_string() },
            token_url: creds.oauth_token_url.trim().to_string(),
            ..Default::default()
        };
        let access = creds.access_token.clone().unwrap_or_default();
        let refresh = creds.refresh_token.clone().unwrap_or_default();
        let fetched =
            tokio::task::spawn_blocking(move || crate::oauth::fetch_photo(&client, &access, &refresh, "")).await;
        let photo = match fetched {
            Ok(Ok(photo)) => photo,
            Ok(Err(err)) => {
                crate::mlog!(crate::log::Level::Info, "avatar", "no photo for {account}: {err}");
                continue;
            }
            Err(_) => continue,
        };
        let saved = save(&root, &account, photo.ext, &photo.bytes)
            .map_err(anyhow::Error::from)
            .and_then(|url| {
                // Only over the link seen missing: a copy that landed while
                // this was asking brought its own photo.
                engine
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE accounts SET avatar_url = ?1, updated_at = strftime('%s', 'now')
                         WHERE id = ?2 AND avatar_url = ?3",
                        rusqlite::params![url, account, seen],
                    )
                    .map_err(anyhow::Error::from)
            });
        match saved {
            Ok(0) => {}
            Ok(_) => {
                crate::ffi::emit_event("account.photoChanged", serde_json::json!({ "account": account }));
                if let Some(tell) = engine.account_changed.get() {
                    tell(&account);
                }
                changed.push(account);
            }
            Err(err) => crate::mlog!(crate::log::Level::Warn, "avatar", "could not save the photo of {account}: {err:#}"),
        }
    }
    changed
}

