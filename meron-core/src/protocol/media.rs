//! Manages files cached on behalf of the mobile reader and account screens.
//! The host writes account images, opens attachments and requests storage totals.
//! Cache clearing and account removal operate on the corresponding media directory.

use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub(crate) fn write_mobile_account_media_file(
    data_directory: &str,
    params: &Value,
    media_kind: &str,
) -> Result<Value, String> {
    let id = req_account_pref_id(params)?;
    let filename = req_str(params, "filename")?;
    let mime = opt_str(params, "mime").to_lowercase();
    let data = req_str(params, "data")?;
    let bytes = STANDARD
        .decode(data.as_bytes())
        .map_err(|error| format!("invalid media data: {error}"))?;
    if bytes.is_empty() {
        return Err("media file is empty".to_string());
    }
    let ext = media_extension(&filename, &mime)?;
    let relative_url = store_mobile_account_media(data_directory, &id, media_kind, ext, &bytes)?;
    Ok(json!({ "url": relative_url }))
}

/// Write media bytes under `media/<kind>/<account>/` and return its `/media/...` URL.
pub(crate) fn store_mobile_account_media(
    data_directory: &str,
    id: &str,
    media_kind: &str,
    ext: &str,
    bytes: &[u8],
) -> Result<String, String> {
    let account_directory = safe_media_segment(id);
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let stored_name = format!("{stamp}.{ext}");
    let relative_url = format!("/media/{media_kind}/{account_directory}/{stored_name}");
    let directory = Path::new(data_directory)
        .join("media")
        .join(media_kind)
        .join(&account_directory);
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    std::fs::write(directory.join(stored_name), bytes).map_err(|error| error.to_string())?;
    Ok(relative_url)
}

pub(crate) fn read_mobile_attachment_file(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let key = req_str(params, "key")?;
    let relative = Path::new(&key);
    if key.trim().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err("invalid attachment key".to_string());
    }
    let root = Path::new(data_directory).join("attachments");
    let path = root.join(relative);
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    if !path.starts_with(&root) {
        return Err("invalid attachment key".to_string());
    }
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    Ok(json!({ "data": STANDARD.encode(data) }))
}

pub(crate) fn mobile_storage_usage(data_directory: &str) -> Result<Value, String> {
    let root = Path::new(data_directory);
    if data_directory.trim().is_empty() {
        return Err("mobile core is not initialized".to_string());
    }
    Ok(json!({
        "cacheBytes": path_size_bytes(&root.join("attachments")).map_err(|err| err.to_string())?,
        "dbBytes": path_size_bytes(&root.join("meron.db")).map_err(|err| err.to_string())?,
    }))
}

pub(crate) fn clear_mobile_storage_cache(data_directory: &str) -> Result<Value, String> {
    let cache_directory = Path::new(data_directory).join("attachments");
    if cache_directory.exists() {
        std::fs::remove_dir_all(&cache_directory).map_err(|error| error.to_string())?;
    }
    std::fs::create_dir_all(&cache_directory).map_err(|error| error.to_string())?;
    mobile_storage_usage(data_directory)
}

pub(crate) fn remove_mobile_account_media(data_directory: &str, account: &str) {
    let root = Path::new(data_directory);
    crate::parse::remove_account_media(&root.join("attachments"), account);
    let account_directory = safe_media_segment(account);
    for media_kind in ["avatars", "wallpapers"] {
        let _ =
            std::fs::remove_dir_all(root.join("media").join(media_kind).join(&account_directory));
    }
}

pub(crate) fn path_size_bytes(path: &Path) -> std::io::Result<u64> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        total += path_size_bytes(&entry?.path())?;
    }
    Ok(total)
}

pub(crate) fn media_extension(filename: &str, mime: &str) -> Result<&'static str, String> {
    let name_ext = filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.trim().to_lowercase())
        .unwrap_or_default();
    match name_ext.as_str() {
        "png" => Ok("png"),
        "jpg" | "jpeg" => Ok("jpg"),
        "webp" => Ok("webp"),
        "gif" => Ok("gif"),
        _ => match mime {
            "image/png" => Ok("png"),
            "image/jpeg" | "image/jpg" => Ok("jpg"),
            "image/webp" => Ok("webp"),
            "image/gif" => Ok("gif"),
            _ => Err("media file must be png, jpeg, webp, or gif".to_string()),
        },
    }
}

pub(crate) fn safe_media_segment(value: &str) -> String {
    let segment: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = segment.trim_matches('_').to_string();
    if cleaned.is_empty() {
        "account".to_string()
    } else {
        cleaned
    }
}
