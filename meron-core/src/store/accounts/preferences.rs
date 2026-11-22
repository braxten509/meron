//! Writes per-account preferences and saved authorization state.
//! Account setup and settings handlers persist values through these shared queries.
//! The account record retains its existing identity during a preference update.

use super::*;

/// Record the fingerprint of a refresh token the provider refused (see
/// [`Creds::rejected_grant`]). Touches only that key, so it cannot undo a save
/// that landed while the refused request was in flight.
pub fn set_account_rejected_grant(conn: &Connection, id: &str, fingerprint: &str) -> Result<()> {
    conn.execute(
        "UPDATE accounts
            SET config = json_set(config, '$.rejected_grant', ?2), updated_at = ?3
          WHERE id = ?1",
        params![id, fingerprint, now_unix()],
    )?;
    Ok(())
}

/// Persist the per-account "load remote images" preference (explicit on/off),
/// overriding the engine default. `json_set` updates just this key in place, so
/// other prefs are untouched.
pub fn set_load_remote_images(conn: &Connection, id: &str, enabled: bool) -> Result<()> {
    set_account_pref(conn, id, "load_remote_images", enabled)
}

/// Set a boolean field in an account's `prefs` JSON column. `key` is a top-level
/// pref name (e.g. "muted", "paused", "included_in_unified"); unset keys resolve
/// to their code default in `AccountPrefs`, so no schema migration is needed.
pub fn set_account_pref(conn: &Connection, id: &str, key: &str, enabled: bool) -> Result<()> {
    let path = format!("$.{key}");
    conn.execute(
        "UPDATE accounts
         SET prefs = json_set(prefs, ?2, json(?3)), updated_at = ?4
         WHERE id = ?1",
        params![id, path, if enabled { "true" } else { "false" }, now_unix()],
    )?;
    Ok(())
}

/// Replace just the proxy entry in an account's connection `config`, leaving
/// the rest of the JSON (hosts, ports, OAuth metadata) untouched.
pub fn set_account_proxy(
    conn: &Connection,
    id: &str,
    choice: &crate::proxy::ProxyChoice,
) -> Result<()> {
    conn.execute(
        "UPDATE accounts
         SET config = json_set(config, '$.proxy', json(?2)), updated_at = ?3
         WHERE id = ?1",
        params![id, choice.to_json().to_string(), now_unix()],
    )?;
    Ok(())
}

/// Replace just the certificate pins in an account's connection `config`. Used
/// when the user accepts a server certificate for an account that already
/// exists, so the pin lands without re-entering the password. `None` clears the
/// pin for that server.
pub fn set_account_cert_pins(
    conn: &Connection,
    id: &str,
    cert_pin: Option<&str>,
    smtp_cert_pin: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE accounts
         SET config = json_set(config, '$.cert_pin', ?2, '$.smtp_cert_pin', ?3), updated_at = ?4
         WHERE id = ?1",
        params![id, cert_pin, smtp_cert_pin, now_unix()],
    )?;
    Ok(())
}

/// Set a numeric field in an account's `prefs` JSON column.
pub fn set_account_pref_u64(conn: &Connection, id: &str, key: &str, value: u64) -> Result<()> {
    let path = format!("$.{key}");
    conn.execute(
        "UPDATE accounts
         SET prefs = json_set(prefs, ?2, json(?3)), updated_at = ?4
         WHERE id = ?1",
        params![id, path, value.to_string(), now_unix()],
    )?;
    Ok(())
}

/// Set or clear a JSON field in an account's `prefs` column. `None` removes the
/// key so callers can return to the code default without storing sentinel data.
pub fn set_account_pref_json(
    conn: &Connection,
    id: &str,
    key: &str,
    value: Option<Value>,
) -> Result<()> {
    let path = format!("$.{key}");
    match value {
        Some(value) => {
            conn.execute(
                "UPDATE accounts
                 SET prefs = json_set(prefs, ?2, json(?3)), updated_at = ?4
                 WHERE id = ?1",
                params![id, path, value.to_string(), now_unix()],
            )?;
        }
        None => {
            conn.execute(
                "UPDATE accounts
                 SET prefs = json_remove(prefs, ?2), updated_at = ?3
                 WHERE id = ?1",
                params![id, path, now_unix()],
            )?;
        }
    }
    Ok(())
}

/// Replace an account's send-as aliases (the whole list). Stored in the `prefs`
/// JSON `aliases` key via `json_set`, leaving other prefs untouched. Entries are
/// normalized by the caller (trimmed, blank emails dropped).
pub fn set_account_aliases(conn: &Connection, id: &str, aliases: &[Alias]) -> Result<()> {
    conn.execute(
        "UPDATE accounts
         SET prefs = json_set(prefs, '$.aliases', json(?2)), updated_at = ?3
         WHERE id = ?1",
        params![id, json!(aliases).to_string(), now_unix()],
    )?;
    Ok(())
}
