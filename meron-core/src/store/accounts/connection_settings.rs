//! Encodes the connection settings stored alongside an account.
//! Account reads and updates share the same persisted representation.
//! Certificate pins and server settings are restored when loading credentials.

use super::*;

pub(super) fn creds_to_config(creds: &Creds) -> String {
    json!({
        "host": creds.host,
        "port": creds.port,
        "user": creds.user,
        "tls": creds.tls,
        "starttls": creds.starttls,
        "smtp_host": creds.smtp_host,
        "smtp_port": creds.smtp_port,
        "smtp_tls": creds.smtp_tls,
        "smtp_starttls": creds.smtp_starttls,
        "auth_type": creds.auth_type,
        "token_expires_at": creds.token_expires_at,
        "oauth_client_id": creds.oauth_client_id,
        "oauth_client_secret": creds.oauth_client_secret,
        "oauth_token_url": creds.oauth_token_url,
        "oauth_scope": creds.oauth_scope,
        "proxy": creds.proxy.to_json(),
        "cert_pin": creds.cert_pin,
        "smtp_cert_pin": creds.smtp_cert_pin,
        "rejected_grant": creds.rejected_grant,
    })
    .to_string()
}

/// Pins are stored as hex strings; absent, null and empty all mean "no pin"
/// (accounts written before pinning existed have no key at all).
fn config_cert_pin(config: &serde_json::Value, key: &str) -> Option<String> {
    config[key]
        .as_str()
        .map(str::trim)
        .filter(|pin| !pin.is_empty())
        .map(|pin| pin.to_ascii_lowercase())
}

pub(super) fn config_to_creds(json: &str) -> Creds {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_else(|_| json!({}));
    Creds {
        host: v["host"].as_str().unwrap_or("").to_string(),
        port: v["port"].as_u64().unwrap_or(993) as u16,
        user: v["user"].as_str().unwrap_or("").to_string(),
        password: String::new(),
        tls: v["tls"].as_bool().unwrap_or(true),
        starttls: v["starttls"].as_bool().unwrap_or(false),
        smtp_host: v["smtp_host"].as_str().unwrap_or("").to_string(),
        smtp_port: v["smtp_port"].as_u64().unwrap_or(587) as u16,
        smtp_tls: v["smtp_tls"].as_bool().unwrap_or(true),
        smtp_starttls: v["smtp_starttls"].as_bool().unwrap_or(false),
        auth_type: v["auth_type"].as_str().unwrap_or("password").to_string(),
        access_token: None,
        refresh_token: None,
        token_expires_at: v["token_expires_at"].as_i64().unwrap_or(0),
        oauth_client_id: v["oauth_client_id"].as_str().unwrap_or("").to_string(),
        oauth_client_secret: v["oauth_client_secret"].as_str().unwrap_or("").to_string(),
        oauth_token_url: v["oauth_token_url"].as_str().unwrap_or("").to_string(),
        oauth_scope: v["oauth_scope"].as_str().unwrap_or("").to_string(),
        // Accounts written before proxy support have no key here, which parses
        // as "follow the app-wide setting".
        proxy: crate::proxy::ProxyChoice::from_json(&v["proxy"]),
        cert_pin: config_cert_pin(&v, "cert_pin"),
        smtp_cert_pin: config_cert_pin(&v, "smtp_cert_pin"),
        rejected_grant: v["rejected_grant"]
            .as_str()
            .filter(|fingerprint| !fingerprint.is_empty())
            .map(str::to_string),
    }
}
