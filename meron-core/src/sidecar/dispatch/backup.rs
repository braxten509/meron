//! Dispatches desktop backup operations to the shared backup implementation.
//! The host selects export or restore and supplies its file or encryption options.
//! The sidecar passes the result back over the desktop RPC connection.

use serde_json::{Value, json};
use std::sync::Arc;

use meron_core::engine::Engine;
use meron_core::protocol::Request;
use meron_core::{backup, proxy, secrets, store};

use crate::Writer;
use crate::sidecar::params::*;

/// Handle `backup.*`: export and import of the whole store.
pub(crate) async fn dispatch(
    engine: &Arc<Engine>,
    command_request: &Request,
    _out: &Writer,
) -> anyhow::Result<Value> {
    let parameters = &command_request.params;
    match command_request.method.as_str() {
        // Serialize accounts, prefs, feeds and settings to a backup document
        // (see `backup`). The passphrase-based key derivation is deliberately
        // slow, so this runs on the blocking pool rather than the reactor.
        "backup.export" => {
            let include_secrets = parameters
                .get("include_secrets")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let passphrase = parameters
                .get("passphrase")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            // The Go host owns the product version; core only knows its crate's.
            // The platform goes with it because desktop and mobile version
            // independently, so the number alone doesn't identify a build.
            let app_version = parameters
                .get("app_version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let platform = parameters
                .get("platform")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let engine = engine.clone();
            let text = tokio::task::spawn_blocking(move || {
                // Secrets are read up front, with the DB lock released, because
                // they come from the OS keychain — the slowest and most
                // failure-prone dependency here (a Flatpak Secret portal with no
                // backend can hang outright). Holding the store lock across that
                // would stall every other request behind a backup.
                let secrets: std::collections::HashMap<String, secrets::Secrets> =
                    if include_secrets {
                        let ids = {
                            let conn = engine.db.lock().unwrap();
                            store::list_accounts(&conn)?
                                .iter()
                                .filter_map(|account| account.get("id").and_then(Value::as_str))
                                .map(str::to_string)
                                .collect::<Vec<_>>()
                        };
                        ids.into_iter()
                            // An account whose entry is missing or unreadable exports
                            // without one rather than failing the whole backup.
                            .map(|id| {
                                let loaded = secrets::load(&id).unwrap_or_default();
                                (id, loaded)
                            })
                            .collect()
                    } else {
                        std::collections::HashMap::new()
                    };
                backup::export(
                    &engine.db.lock().unwrap(),
                    include_secrets,
                    Some(passphrase.as_str()),
                    backup::Host {
                        platform: platform.as_str(),
                        app_version: app_version.as_str(),
                    },
                    &|account| secrets.get(account).cloned().unwrap_or_default(),
                )
            })
            .await??;
            Ok(json!({ "backup": text }))
        }

        // Restore a backup document. An encrypted file opened without a
        // passphrase comes back as `needs_passphrase` (not an error) so the UI
        // can prompt and call again instead of showing a failure.
        "backup.import" => {
            let text = required_string(parameters, "backup")?;
            let passphrase = parameters
                .get("passphrase")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let engine = engine.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                let data = match backup::parse(&text, Some(passphrase.as_str())) {
                    Ok(data) => data,
                    Err(err) if backup::needs_passphrase(&err.to_string()) => return Ok(None),
                    Err(err) => return Err(err),
                };
                let conn = engine.db.lock().unwrap();
                let summary = backup::apply(&conn, &data, &|conn, account, secrets| {
                    // Mirror DesktopHost::store_secret: the keychain owns the
                    // secret, and the legacy SQLite column stays empty.
                    let _ = conn;
                    secrets::store(account, secrets)
                })?;
                // Restored settings include the app-wide proxy, which socket
                // code reads from a process-global slot.
                proxy::load_global(&conn)?;
                Ok(Some(summary))
            })
            .await??;
            match outcome {
                Some(summary) => Ok(summary.to_json()),
                None => Ok(json!({ "needs_passphrase": true })),
            }
        }

        other => Err(anyhow::anyhow!("unknown method: {other}")),
    }
}
