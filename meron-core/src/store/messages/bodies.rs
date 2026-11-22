//! Reads and writes cached message bodies and attachment metadata.
//! Reader requests reconstruct messages and recipient lists from SQLite rows.
//! Background backfill populates attachment metadata missing from older records.

use super::*;

pub fn has_cached_body(conn: &Connection, account: &str, folder: &str, uid: u32) -> Result<bool> {
    let found = conn
        .query_row(
            "SELECT 1 FROM messages
             WHERE account = ?1 AND folder = ?2 AND uid = ?3
               AND (body IS NOT NULL
                    OR json_extract(json, '$.body_html') IS NOT NULL)",
            params![account, folder, uid],
            |_| Ok(()),
        )
        .ok()
        .is_some();
    Ok(found)
}

pub fn has_message(conn: &Connection, account: &str, folder: &str, uid: u32) -> Result<bool> {
    let found = conn
        .query_row(
            "SELECT 1 FROM messages
             WHERE account = ?1 AND folder = ?2 AND uid = ?3",
            params![account, folder, uid],
            |_| Ok(()),
        )
        .ok()
        .is_some();
    Ok(found)
}

/// Render a stored recipient list (JSON `[{name, addr}]`) as a comma-separated
/// "Name <addr>" / "addr" string. Empty/missing/malformed input yields "".
fn format_recipient_list(json: Option<&str>) -> String {
    let Some(text) = json.filter(|value| !value.is_empty()) else {
        return String::new();
    };
    let Ok(recipients) = serde_json::from_str::<Vec<serde_json::Value>>(text) else {
        return String::new();
    };
    recipients
        .iter()
        .filter_map(|v| {
            let addr = v["addr"].as_str().unwrap_or_default().trim();
            if addr.is_empty() {
                return None;
            }
            let name = v["name"].as_str().unwrap_or_default().trim();
            Some(if name.is_empty() {
                addr.to_string()
            } else {
                format!("{name} <{addr}>")
            })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_recipient_value(value: Option<&Value>) -> String {
    value
        .map(Value::to_string)
        .map(|json| format_recipient_list(Some(&json)))
        .unwrap_or_default()
}

pub fn get_cached_message(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
) -> Result<Option<Message>> {
    let mut stmt = conn.prepare(
        "SELECT subject, from_name, from_addr, date, body, json
         FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
    )?;

    let row = stmt
        .query_row(params![account, folder, uid], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .ok();

    let Some((subject, from_name, from_addr, date, body, json_extra)) = row else {
        return Ok(None);
    };
    let extra: Value = serde_json::from_str(&json_extra).unwrap_or_else(|_| json!({}));

    let to = format_recipient_value(extra.get("to"));

    let reply_to = extra["reply_to"].as_str().unwrap_or_default().to_string();
    let cc = format_recipient_value(extra.get("cc"));
    let bcc = extra["bcc"].as_str().unwrap_or_default().to_string();
    let message_id = extra["message_id"].as_str().unwrap_or_default().to_string();
    let references = extra["references"].as_str().unwrap_or_default().to_string();
    let body_html = extra["body_html"].as_str().map(str::to_string);

    // `body` is the canonical plain-text body. For legacy HTML-only rows where it
    // is missing, fall back to rendering the stored HTML source once.
    let body = match body {
        Some(body) => body,
        None => match &body_html {
            Some(html) => {
                let rendered = crate::parse::render_body(html);
                let _ = conn.execute(
                    "UPDATE messages SET body = ?4 WHERE account = ?1 AND folder = ?2 AND uid = ?3",
                    params![account, folder, uid, rendered],
                );
                rendered
            }
            None => return Ok(None),
        },
    };

    let attachments: Vec<Attachment> = extra
        .get("attachments")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();

    // Keep the HTML source on the returned message so the read handler can build
    // the iframe-ready view (it injects the remote-image CSP per account setting).
    Ok(Some(Message {
        subject,
        from_name,
        from_addr,
        to,
        reply_to,
        cc,
        bcc,
        message_id,
        references,
        delivered: extra["delivered"].as_bool().unwrap_or(false),
        date,
        body,
        body_html,
        body_is_rendered: extra["body_is_rendered"].as_bool().unwrap_or(false),
        invite: extra
            .get("invite")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok()),
        preview: String::new(),
        attachments,
    }))
}

/// The `files` column for `message`: its real attachments as the list cards
/// name them, or NULL when there are none (see the v11 migration).
fn files_json(message: &Message) -> Option<String> {
    let files: Vec<Value> = message
        .file_attachments()
        .map(|attachment| json!({ "filename": attachment.filename, "mime": attachment.mime }))
        .collect();
    (!files.is_empty()).then(|| Value::Array(files).to_string())
}

/// Fill `messages.files` for up to `batch` rows cached before the column existed (see the v11 migration), resuming where the last call stopped.
///
/// Returns true once every row has been visited. Each call is one short transaction, so the caller
/// can release the store between batches.
///
/// Mirrors [`Message::file_attachments`] in SQL: an attachment counts unless the stored HTML shows
/// it inline.
pub fn backfill_files_batch(conn: &Connection, batch: u32) -> Result<bool> {
    let cursor = match crate::store::db::meta_get(conn, FILES_BACKFILL_KEY)?.as_deref() {
        Some("done") => return Ok(true),
        Some(id) => id.parse::<i64>().unwrap_or(0),
        None => 0,
    };
    let tx = conn.unchecked_transaction()?;
    let last: Option<i64> = tx.query_row(
        "SELECT max(id) FROM (SELECT id FROM messages WHERE id > ?1 ORDER BY id LIMIT ?2)",
        params![cursor, batch],
        |row| row.get(0),
    )?;
    let Some(last) = last else {
        crate::store::db::meta_set(&tx, FILES_BACKFILL_KEY, "done")?;
        tx.commit()?;
        return Ok(true);
    };
    tx.execute(
        "UPDATE messages SET files = (
           SELECT CASE WHEN count(*) = 0 THEN NULL ELSE json_group_array(json_object(
             'filename', json_extract(attachment.value, '$.filename'),
             'mime', json_extract(attachment.value, '$.mime'))) END
           FROM json_each(messages.json, '$.attachments') AS attachment
           WHERE json_extract(attachment.value, '$.key') IS NULL
              OR instr(COALESCE(json_extract(messages.json, '$.body_html'), ''),
                       '/media/' || json_extract(attachment.value, '$.key')) = 0
         )
         WHERE id > ?1 AND id <= ?2 AND uid <> 0
           AND json_extract(json, '$.attachments') IS NOT NULL",
        params![cursor, last],
    )?;
    crate::store::db::meta_set(&tx, FILES_BACKFILL_KEY, &last.to_string())?;
    tx.commit()?;
    Ok(false)
}

pub fn save_cached_message(
    conn: &Connection,
    account: &str,
    folder: &str,
    uid: u32,
    message: &Message,
) -> Result<()> {
    let attachments_json = serde_json::to_value(&message.attachments).unwrap_or_else(|_| json!([]));
    // The `json` catch-all column carries header fields that don't have a typed
    // column. Envelope recipients are written by `upsert_messages`; body fetches
    // must not overwrite them.
    let extra_json = json!({
        "reply_to": message.reply_to,
        "bcc": message.bcc,
        "message_id": message.message_id,
        "references": message.references,
        "delivered": message.delivered,
        "body_html": message.body_html,
        "body_is_rendered": message.body_is_rendered,
        "attachments": attachments_json,
        "invite": message.invite,
    })
    .to_string();

    conn.execute(
        "INSERT INTO messages (account, folder, msg_id, uid, subject, from_name, from_addr, date, body, json, files)
         SELECT ?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10
         WHERE NOT EXISTS (
           SELECT 1 FROM removed_message_uids
           WHERE account = ?1 AND folder = ?2 AND uid = ?3
         )
         ON CONFLICT(account, folder, msg_id) DO UPDATE SET
           subject = excluded.subject,
           from_name = excluded.from_name,
           from_addr = excluded.from_addr,
           -- A body whose Date header didn't parse must not undo the header
           -- sync's INTERNALDATE fallback.
           date = CASE WHEN excluded.date = 0 THEN messages.date ELSE excluded.date END,
           body = excluded.body,
           json = json_patch(messages.json, excluded.json),
           files = excluded.files",
        params![
            account,
            folder,
            uid,
            message.subject,
            message.from_name,
            message.from_addr,
            message.date,
            message.body,
            extra_json,
            files_json(message)
        ],
    )?;

    // Every body fetch lands here, so an organizer's update reaches the
    // calendar whichever path read it. A calendar failure must not lose the
    // message.
    if let Some(invite) = &message.invite {
        let _ = crate::calendar::events::apply_invite_update(
            conn,
            account,
            &message.from_addr,
            &message.message_id,
            invite,
        );
    }

    Ok(())
}
