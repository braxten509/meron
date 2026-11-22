//! Decodes common RPC fields used by desktop feature handlers.
//! Helpers distinguish required values from optional attachment and cursor data.
//! A malformed input is reported to the calling command.

use serde_json::Value;
use std::sync::Arc;

use meron_core::engine::Engine;
use meron_core::{imap, smtp, store};

/// Decode an opaque RSS pagination cursor `"ts:<i64>:<item_key>"`.
pub(crate) fn parse_rss_cursor(raw: &str) -> Option<(i64, String)> {
    let rest = raw.strip_prefix("ts:")?;
    let (ts, key) = rest.split_once(':')?;
    Some((ts.parse().ok()?, key.to_string()))
}

/// Whether an account is RSS-backed (vs mail), per its row in the unified DB.
pub(crate) fn is_rss(engine: &Arc<Engine>, account: &str) -> anyhow::Result<bool> {
    Ok(store::account_engine(&engine.db.lock().unwrap(), account)?.as_deref() == Some("rss"))
}

/// Parse the optional `attachments` array. An entry that fails to deserialize
/// is a hard error: skipping it would send/save the message without its file
/// while reporting success.
pub(crate) fn opt_attachments(params: &Value) -> anyhow::Result<Vec<smtp::AttachmentInput>> {
    match params.get("attachments") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|val| {
                serde_json::from_value::<smtp::AttachmentInput>(val.clone())
                    .map_err(|error| anyhow::anyhow!("invalid attachment: {error}"))
            })
            .collect(),
        Some(Value::Null) | None => Ok(Vec::new()),
        Some(_) => Err(anyhow::anyhow!("attachments must be an array")),
    }
}

pub(crate) fn required_string(params: &Value, key: &str) -> anyhow::Result<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing string param: {key}"))
}

pub(crate) fn required_boolean(params: &Value, key: &str) -> anyhow::Result<bool> {
    params
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("missing bool param: {key}"))
}

pub(crate) fn required_u16(params: &Value, key: &str) -> anyhow::Result<u16> {
    params
        .get(key)
        .and_then(Value::as_u64)
        // Clamp rather than wrap: a 70 000-row limit must not read as 4 464.
        .map(|n| u16::try_from(n).unwrap_or(u16::MAX))
        .ok_or_else(|| anyhow::anyhow!("missing number param: {key}"))
}

pub(crate) fn required_u32(params: &Value, key: &str) -> anyhow::Result<u32> {
    let number = params
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("missing number param: {key}"))?;
    u32::try_from(number).map_err(|_| anyhow::anyhow!("{key} exceeds the supported range"))
}

/// A certificate pin parameter: hex, normalized, with blank treated as absent.
pub(crate) fn cert_pin_param(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|pin| !pin.is_empty())
        .map(|pin| pin.to_ascii_lowercase())
}

pub(crate) fn required_strings(params: &Value, key: &str) -> anyhow::Result<Vec<String>> {
    let items = params
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("missing array param: {key}"))?;
    let mut out = Vec::new();
    for v in items {
        if let Some(s) = v.as_str() {
            out.push(s.to_string());
        } else {
            return Err(anyhow::anyhow!("array element is not a string"));
        }
    }
    Ok(out)
}

/// IMAP APPEND a freshly-sent message to the account's Sent folder, with `\Seen`.
///
/// Best-effort: callers log and ignore errors so SMTP success doesn't surface as "send failed" when
/// the server's Sent folder is unusual.
///
/// After the APPEND succeeds we also fetch the most recent envelopes from the Sent folder and
/// upsert them into the local store. Without this, the just- sent message would only land in the DB
/// at the next periodic sync — meaning it wouldn't appear in the thread view until the user
/// reconnects or refreshes.
///
/// Resolve the outgoing From address + display name for a send/draft, deferring to the shared store
/// rule so desktop and mobile accept the same identities. An unowned address is an error, surfaced
/// to the composer rather than sent under a substituted sender.
pub(crate) fn resolve_send_from(
    engine: &Arc<Engine>,
    account: &str,
    creds: &imap::Creds,
    requested_from: &str,
) -> anyhow::Result<(String, String)> {
    let db = engine.db.lock().unwrap();
    store::resolve_send_from(&db, account, &creds.user, requested_from)
}

/// A present UID must identify the same message after conversion to an IMAP integer.
fn message_uid(value: &Value) -> anyhow::Result<u32> {
    value
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
        .ok_or_else(|| anyhow::anyhow!("invalid message UID: {value}"))
}

pub(crate) fn optional_message_uid(params: &Value) -> anyhow::Result<Option<u32>> {
    params
        .get("uid")
        .filter(|value| !value.is_null())
        .map(message_uid)
        .transpose()
}

pub(crate) fn explicit_message_uids(params: &Value) -> anyhow::Result<Vec<u32>> {
    let Some(value) = params.get("uids").filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("uids must be an array"))?;
    values.iter().map(message_uid).collect()
}

/// The explicit UIDs of a tag change, or nothing when it names a whole thread.
pub(crate) fn tag_uids(params: &Value) -> anyhow::Result<Vec<u32>> {
    if params.get("uids").is_some_and(|value| !value.is_null()) {
        explicit_message_uids(params)
    } else {
        Ok(optional_message_uid(params)?.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn uid_parameters_preserve_boundaries_and_reject_a_mixed_invalid_selection() {
        let maximum = u64::from(u32::MAX);
        assert_eq!(
            optional_message_uid(&json!({"uid": maximum})).unwrap(),
            Some(u32::MAX)
        );
        assert_eq!(optional_message_uid(&json!({"uid": null})).unwrap(), None);
        assert!(optional_message_uid(&json!({"uid": maximum + 2})).is_err());
        assert!(explicit_message_uids(&json!({"uids": [1, maximum + 2]})).is_err());
        assert!(tag_uids(&json!({"uid": 1, "uids": ["bad"]})).is_err());
        assert_eq!(tag_uids(&json!({"uid": 1})).unwrap(), [1]);
    }
}
