//! Bridges mobile backup commands to the core export and restore routines.
//! The caller supplies the file contents or options and the mobile storage location.
//! The response carries the serialized backup or the import summary.

use super::*;

/// Mobile `backup.export`. Mirrors the desktop handler in `main.rs`, differing
/// only in where secrets live: the keyed app-private DB rather than an OS
/// keychain.
pub(crate) fn export_mobile_backup(data_directory: &str, params: &Value) -> Result<Value, String> {
    let include_secrets = params
        .get("include_secrets")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let passphrase = optional_string(params, "passphrase");
    let app_version = optional_string(params, "app_version");
    let platform = optional_string(params, "platform");
    let host = crate::backup::Host {
        platform: &platform,
        app_version: &app_version,
    };
    with_mobile_db(data_directory, |conn| {
        let load = |account: &str| load_mobile_secret(&conn, account);
        let text = crate::backup::export(&conn, include_secrets, Some(&passphrase), host, &load)
            .map_err(|error| format!("{error:#}"))?;
        Ok(json!({ "backup": text }))
    })
}

/// Mobile `backup.import`. An encrypted file opened without a passphrase comes
/// back as `{ "needs_passphrase": true }` rather than an error, so the host can
/// prompt and call again.
pub(crate) fn import_mobile_backup(data_directory: &str, params: &Value) -> Result<Value, String> {
    let text = required_string(params, "backup")?;
    let passphrase = optional_string(params, "passphrase");
    let result = with_mobile_db(data_directory, |conn| {
        let data = match crate::backup::parse(&text, Some(&passphrase)) {
            Ok(data) => data,
            Err(error) if crate::backup::needs_passphrase(&error.to_string()) => {
                return Ok(json!({ "needs_passphrase": true }));
            }
            Err(error) => return Err(format!("{error:#}")),
        };
        let summary = crate::backup::apply(&conn, &data, &|conn, account, secrets| {
            let blob = serde_json::to_string(secrets)?;
            store::upsert_secret(conn, account, &blob)
        })
        .map_err(|error| format!("{error:#}"))?;
        // Restored settings include the app-wide proxy, which socket code reads
        // from a process-global slot rather than the DB.
        crate::proxy::load_global(&conn).map_err(|error| format!("{error:#}"))?;
        Ok(summary.to_json())
    })?;
    if result.get("needs_passphrase").is_none() {
        crate::ffi::emit_event("calendar.changed", json!({ "method": "backup.import" }));
        crate::ffi::emit_event("tasks.changed", json!({ "method": "backup.import" }));
    }
    Ok(result)
}
