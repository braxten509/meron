//! Handles account setup and preferences requested by the mobile hosts.
//! Discovery resolves server settings and account commands persist the selected configuration.
//! Results are encoded for the same protocol used by Android and iOS.

mod discovery;
pub(crate) use discovery::autodiscover_mobile_account;
#[cfg(test)]
use discovery::parse_mobile_autoconfig;

use super::*;
use hickory_resolver::Resolver;
use hickory_resolver::proto::rr::RData;
use quick_xml::Reader;
use quick_xml::events::Event;
use std::time::Duration;
use url::form_urlencoded;

#[derive(Clone, Debug, Default)]
struct DiscoveredMailSettings {
    imap_host: String,
    imap_port: u16,
    smtp_host: String,
    smtp_port: u16,
    username: String,
    provider_name: String,
    source: String,
}

#[derive(Default)]
struct AutoconfigServer {
    server_type: String,
    hostname: String,
    port: u16,
    socket_type: String,
    username: String,
}

pub(crate) fn list_mobile_accounts(data_directory: &str) -> Result<Value, String> {
    with_mobile_db(data_directory, |conn| {
        let mut accounts = store::list_accounts(&conn).map_err(|error| error.to_string())?;
        crate::account_photos::hide_missing(
            &std::path::Path::new(data_directory.trim()).join("media"),
            &mut accounts,
        );
        let creds_by_id = store::load_accounts(&conn)
            .map_err(|error| error.to_string())?
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        for account in &mut accounts {
            if account
                .get("auth_type")
                .and_then(Value::as_str)
                .is_some_and(|auth_type| auth_type == "rss")
            {
                continue;
            }
            let needs_reconnect = account
                .get("id")
                .and_then(Value::as_str)
                .and_then(|id| {
                    let mut creds = creds_by_id.get(id)?.clone();
                    load_mobile_secret(&conn, id).apply_to(&mut creds);
                    Some(account_needs_reconnect(&creds))
                })
                .unwrap_or(true);
            if let Some(object) = account.as_object_mut() {
                object.insert("needs_reconnect".to_string(), json!(needs_reconnect));
            }
        }
        Ok(json!({ "accounts": accounts }))
    })
}

pub(crate) fn account_preference_id(params: &Value) -> Result<String, String> {
    required_string(params, "account")
        .or_else(|_| required_string(params, "account_id"))
        .or_else(|_| required_string(params, "id"))
}

pub(crate) fn remove_mobile_account(data_directory: &str, params: &Value) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let result = with_mobile_db(data_directory, |conn| {
        store::delete_account(&conn, &id).map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })?;
    remove_mobile_account_media(data_directory, &id);
    crate::ffi::evict_engine_account(&id);
    Ok(result)
}

pub(crate) fn reorder_mobile_accounts(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let ids = required_strings(params, "accounts")?;
    with_mobile_db(data_directory, |conn| {
        store::reorder_accounts(&conn, &ids).map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

#[cfg(test)]
mod tests;

pub(crate) fn suggest_mobile_contacts(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let mut account = optional_string(params, "account");
    if account.is_empty() {
        account = optional_string(params, "account_id");
    }
    if account.is_empty() {
        account = optional_string(params, "id");
    }
    let query = optional_string(params, "query");
    let limit = optional_u32(params, "limit").unwrap_or(8);
    let sort = store::ContactSort::parse(&optional_string(params, "sort"));
    let directory = params
        .get("directory")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    with_mobile_db(data_directory, |conn| {
        let contacts = store::list_contacts(&conn, &account, &query, limit, sort, directory)
            .map_err(|error| error.to_string())?;
        Ok(json!({ "contacts": contacts }))
    })
}

pub(crate) fn set_mobile_account_name(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let name = optional_string(params, "name");
    with_mobile_db(data_directory, |conn| {
        conn.execute(
            "UPDATE accounts SET display_name = ?1, updated_at = strftime('%s', 'now') WHERE id = ?2",
            rusqlite::params![name, id],
        )
        .map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

pub(crate) fn set_mobile_account_sender_name(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let name = optional_string(params, "name");
    with_mobile_db(data_directory, |conn| {
        conn.execute(
            "UPDATE accounts SET sender_name = ?1, updated_at = strftime('%s', 'now') WHERE id = ?2",
            rusqlite::params![name, id],
        )
        .map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

pub(crate) fn set_mobile_account_avatar(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let avatar_url = optional_string(params, "avatar_url");
    with_mobile_db(data_directory, |conn| {
        conn.execute(
            "UPDATE accounts SET avatar_url = ?1, updated_at = strftime('%s', 'now') WHERE id = ?2",
            rusqlite::params![avatar_url, id],
        )
        .map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

pub(crate) fn set_mobile_account_chat_wallpaper(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let wallpaper = match params.get("wallpaper") {
        Some(Value::Null) | None => None,
        Some(value) => {
            let fields = value
                .as_object()
                .ok_or_else(|| "wallpaper must be an object".to_string())?;
            match fields
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default()
            {
                "preset" => {
                    let preset_id = fields
                        .get("presetId")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim();
                    if preset_id.is_empty() {
                        return Err("preset wallpaper requires presetId".to_string());
                    }
                    Some(json!({ "kind": "preset", "presetId": preset_id }))
                }
                "custom" => {
                    let url = fields
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim();
                    if !url.starts_with("/media/wallpapers/") {
                        return Err("custom wallpaper URL must be a Meron wallpaper".to_string());
                    }
                    Some(json!({ "kind": "custom", "url": url }))
                }
                _ => return Err("unknown wallpaper kind".to_string()),
            }
        }
    };
    with_mobile_db(data_directory, |conn| {
        store::set_account_pref_json(&conn, &id, "chat_wallpaper", wallpaper)
            .map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

/// Mobile `account.setSignature`. A null `signature` clears the override so the
/// account follows the app-wide signature again.
pub(crate) fn set_mobile_account_signature(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let signature = store::AccountSignature::from_param(params.get("signature"))?;
    with_mobile_db(data_directory, |conn| {
        store::set_account_pref_json(&conn, &id, "signature", signature.map(|sig| json!(sig)))
            .map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

pub(crate) fn set_mobile_account_images(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let enabled = required_boolean(params, "enabled")?;
    with_mobile_db(data_directory, |conn| {
        store::set_load_remote_images(&conn, &id, enabled).map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

pub(crate) fn set_mobile_account_bool_pref(
    data_directory: &str,
    params: &Value,
    key: &str,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let enabled = required_boolean(params, "enabled")?;
    with_mobile_db(data_directory, |conn| {
        store::set_account_pref(&conn, &id, key, enabled).map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

pub(crate) fn set_mobile_account_save_sent_copy(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let value = match params.get("value") {
        Some(Value::Bool(enabled)) => Some(json!(enabled)),
        Some(Value::Null) | None => None,
        _ => return Err("value must be true, false, or null".to_string()),
    };
    with_mobile_db(data_directory, |conn| {
        store::set_account_pref_json(&conn, &id, "save_sent_copy", value)
            .map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    })
}

pub(crate) fn set_mobile_account_rss_sync_interval(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let minutes = params
        .get("minutes")
        .and_then(Value::as_u64)
        .unwrap_or(60)
        .clamp(5, 1440);
    with_mobile_db(data_directory, |conn| {
        store::set_account_pref_u64(&conn, &id, "rss_sync_interval_minutes", minutes)
            .map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true, "minutes": minutes }))
    })
}

pub(crate) fn set_mobile_account_aliases(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let mut aliases: Vec<store::Alias> = params
        .get("aliases")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| "aliases must be an array".to_string())?
        .unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    aliases.retain_mut(|alias| {
        alias.email = alias.email.trim().to_string();
        alias.name = alias.name.trim().to_string();
        !alias.email.is_empty() && seen.insert(alias.email.to_lowercase())
    });
    with_mobile_db(data_directory, |conn| {
        store::set_account_aliases(&conn, &id, &aliases).map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true, "aliases": aliases.len() }))
    })
}

pub(crate) fn set_mobile_account_proxy(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let choice = crate::proxy::ProxyChoice::from_json(params.get("proxy").unwrap_or(&Value::Null));
    let result = with_mobile_db(data_directory, |conn| {
        store::set_account_proxy(&conn, &id, &choice).map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true, "proxy": choice.to_json() }))
    });
    if result.is_ok() {
        crate::ffi::evict_engine_account(&id);
    }
    result
}

/// Store certificate pins the user accepted for an account that already exists.
/// The desktop sidecar has the same call; on mobile it is the only way a pin is
/// ever set, since the failure surfaces on a sync rather than at save time.
/// Omitting a key leaves that server's pin alone; an explicit null clears it.
pub(crate) fn set_mobile_account_cert_pin(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let id = account_preference_id(params)?;
    let result = with_mobile_db(data_directory, |conn| {
        let stored = store::load_accounts(&conn)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|(account, _)| account == &id)
            .map(|(_, creds)| creds);
        let cert_pin = match params.get("cert_pin") {
            Some(_) => pin_param(params, "cert_pin"),
            None => stored.as_ref().and_then(|creds| creds.cert_pin.clone()),
        };
        let smtp_cert_pin = match params.get("smtp_cert_pin") {
            Some(_) => pin_param(params, "smtp_cert_pin"),
            None => stored
                .as_ref()
                .and_then(|creds| creds.smtp_cert_pin.clone()),
        };
        store::set_account_cert_pins(&conn, &id, cert_pin.as_deref(), smtp_cert_pin.as_deref())
            .map_err(|error| error.to_string())?;
        Ok(json!({ "ok": true }))
    });
    if result.is_ok() {
        crate::ffi::evict_engine_account(&id);
    }
    result
}

/// The app-wide proxy. Mobile keeps most preferences on the platform side, but
/// this one has to live in the core store: the socket layer reads it, including
/// from background syncs that never touch the UI process state.
pub(crate) fn get_mobile_proxy(data_directory: &str) -> Result<Value, String> {
    with_mobile_db(data_directory, |conn| {
        let stored = store::settings_get(&conn, &[crate::proxy::SETTING_KEY.to_string()])
            .map_err(|error| error.to_string())?;
        Ok(json!({ "proxy": stored[crate::proxy::SETTING_KEY] }))
    })
}

pub(crate) fn set_mobile_proxy(data_directory: &str, params: &Value) -> Result<Value, String> {
    let value = params.get("proxy").cloned().unwrap_or(Value::Null);
    with_mobile_db(data_directory, |conn| {
        store::setting_set(&conn, crate::proxy::SETTING_KEY, &value)
            .map_err(|error| error.to_string())?;
        // Republish immediately: a live Engine (foreground session) would
        // otherwise keep using the proxy it loaded when it was built.
        crate::proxy::set_global(crate::proxy::parse_global(&value));
        Ok(json!({ "ok": true }))
    })
}

/// Fetch the certificate a mail server presents so the account UI can show it
/// and offer to pin it. The desktop sidecar has the same call in `main.rs`;
/// both exist because a self-signed local bridge is unreachable without a pin.
/// A certificate pin parameter: hex, normalized, with blank treated as absent.
fn pin_param(params: &Value, key: &str) -> Option<String> {
    Some(optional_string(params, key).to_ascii_lowercase()).filter(|pin| !pin.is_empty())
}

pub(crate) fn probe_mobile_cert(params: &Value) -> Result<Value, String> {
    let host = required_string(params, "host")?;
    let port = optional_u16(params, "port").unwrap_or(993);
    let protocol = optional_string(params, "protocol");
    let starttls = params
        .get("starttls")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let proxy = crate::proxy::ProxyChoice::from_json(params.get("proxy").unwrap_or(&Value::Null));
    let info = crate::ffi::engine_block_on(async move {
        crate::tls::probe(&host, port, &protocol, starttls, proxy.resolve().as_ref()).await
    })?;
    Ok(json!({ "certificate": info }))
}

pub(crate) fn add_mobile_password_account(
    data_directory: &str,
    params: &Value,
) -> Result<Value, String> {
    let email = required_string(params, "email")?;
    if !email.contains('@') {
        return Err("invalid email".to_string());
    }
    let imap_host = required_string(params, "imap_host")?;
    let smtp_host = required_string(params, "smtp_host")?;
    if imap_host.trim().is_empty() || smtp_host.trim().is_empty() {
        return Err("server required".to_string());
    }
    let username = required_string(params, "username")?;
    if username.trim().is_empty() {
        return Err("username required".to_string());
    }

    let imap_port = optional_u16(params, "imap_port").unwrap_or(993);
    let smtp_port = optional_u16(params, "smtp_port").unwrap_or(465);
    // Older clients send one `tls` switch and rely on the port to distinguish
    // implicit TLS from STARTTLS. Newer clients can choose each transport mode
    // explicitly; keep the inferred values as a backwards-compatible fallback.
    let tls = params.get("tls").and_then(Value::as_bool).unwrap_or(true);
    let (imap_tls, imap_starttls) =
        explicit_or_inferred_tls_mode(params, "imap_tls", "starttls", Some("tls"), tls, imap_port);
    let (smtp_tls, smtp_starttls) =
        explicit_or_inferred_tls_mode(params, "smtp_tls", "smtp_starttls", None, tls, smtp_port);
    let id = account_id(&email);
    let password = optional_string(params, "password");
    let creds = Creds {
        host: imap_host,
        port: imap_port,
        user: username,
        password,
        tls: imap_tls,
        starttls: imap_starttls,
        smtp_host,
        smtp_port,
        smtp_tls,
        smtp_starttls,
        auth_type: "password".to_string(),
        access_token: None,
        refresh_token: None,
        token_expires_at: 0,
        oauth_client_id: String::new(),
        oauth_client_secret: String::new(),
        oauth_token_url: String::new(),
        oauth_scope: String::new(),
        proxy: crate::proxy::ProxyChoice::from_json(params.get("proxy").unwrap_or(&Value::Null)),
        cert_pin: pin_param(params, "cert_pin"),
        smtp_cert_pin: pin_param(params, "smtp_cert_pin"),
        rejected_grant: None,
    };
    let meta = AccountMeta {
        engine: "mail".to_string(),
        provider: "custom".to_string(),
        email: email.clone(),
        display_name: optional_string(params, "display_name"),
        avatar_url: String::new(),
        sender_name: optional_string(params, "sender_name"),
    };
    // Reconnecting re-runs this call with the setup form's fields; anything the
    // form does not carry (the account's proxy, accepted certificates) is kept
    // from the stored account rather than reset. See [`OmittedSettings`].
    let omitted = crate::imap::OmittedSettings {
        proxy: params.get("proxy").is_none(),
        cert_pin: params.get("cert_pin").is_none(),
        smtp_cert_pin: params.get("smtp_cert_pin").is_none(),
        password: params.get("password").is_none(),
    };

    let result = with_mobile_db(data_directory, |conn| {
        let mut creds = creds.clone();
        if omitted.any()
            && let Some(mut stored) =
                store::load_account(&conn, &id).map_err(|error| error.to_string())?
        {
            // The password is held in the secret store, not the account row.
            if omitted.password {
                load_mobile_secret(&conn, &id).apply_to(&mut stored);
            }
            creds.carry_over(&stored, omitted);
        }
        store::upsert_account(&conn, &id, &meta, &creds).map_err(|error| error.to_string())?;
        store_mobile_secret(&conn, &id, &creds)?;
        let mut account = store::list_accounts(&conn)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|account| account.get("id").and_then(Value::as_str) == Some(id.as_str()))
            .ok_or_else(|| "account not found after save".to_string())?;
        if let Some(object) = account.as_object_mut() {
            object.insert(
                "needs_reconnect".to_string(),
                json!(account_needs_reconnect(&creds)),
            );
        }
        Ok(json!({ "account": account }))
    })?;
    crate::ffi::evict_engine_account(&id);
    Ok(result)
}

fn explicit_or_inferred_tls_mode(
    params: &Value,
    tls_key: &str,
    starttls_key: &str,
    tls_alias: Option<&str>,
    legacy_tls: bool,
    port: u16,
) -> (bool, bool) {
    let primary_tls = params.get(tls_key).and_then(Value::as_bool);
    let alias_tls = tls_alias
        .and_then(|key| params.get(key))
        .and_then(Value::as_bool);
    let explicit_starttls = params.get(starttls_key).and_then(Value::as_bool);
    // The legacy alias becomes an explicit partner only when the STARTTLS key
    // is present too; by itself it must retain the old port-inference behavior.
    let explicit_tls = primary_tls.or_else(|| explicit_starttls.and(alias_tls));
    // A lone false STARTTLS flag does not identify the intended alternative;
    // retain legacy port inference unless its paired TLS flag is also present.
    if explicit_tls.is_none() && explicit_starttls != Some(true) {
        return tls_mode(legacy_tls, port);
    }
    let starttls = explicit_starttls.unwrap_or(false);
    let tls = explicit_tls.unwrap_or(legacy_tls);
    (tls && !starttls, starttls)
}
