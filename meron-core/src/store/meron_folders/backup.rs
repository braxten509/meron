//! Serializes local folders and their entries for backup and restore.
//! The backup service calls these routines inside its store transaction.
//! Restore maps saved folder identities to the entries recreated in the database.

use super::*;

/// Every Meron folder with its entries and aliases.
pub fn meron_backup_collect(conn: &Connection) -> Result<Vec<MeronBackupFolder>> {
    let mut out = Vec::new();
    let mut entries_stmt = conn.prepare(
        "SELECT id, account, added_at FROM meron_folder_entries WHERE folder_id = ?1 ORDER BY id",
    )?;
    let mut aliases_stmt =
        conn.prepare("SELECT key FROM meron_folder_aliases WHERE entry_id = ?1 ORDER BY key")?;
    for folder in list_meron_folders(conn)? {
        let entries = entries_stmt
            .query_map(params![folder.id], |row| {
                Ok(MeronBackupEntry {
                    id: row.get(0)?,
                    account: row.get(1)?,
                    added_at: row.get(2)?,
                    aliases: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut filled = Vec::with_capacity(entries.len());
        for mut entry in entries {
            entry.aliases = aliases_stmt
                .query_map(params![entry.id], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            filled.push(entry);
        }
        out.push(MeronBackupFolder {
            id: folder.id,
            name: folder.name,
            sort_order: folder.sort_order,
            created_at: folder.created_at,
            notify: folder.notify,
            entries: filled,
        });
    }
    Ok(out)
}

/// Restore Meron folders from a backup, never over what is here: a folder whose id or name already
/// exists is kept and the backup's conversations go into it; an alias this device already files
/// elsewhere stays there. Entries of unknown accounts are skipped. Answers with how many folders
/// were added and the backup id → kept id map (for AI sorting's setting).
pub fn meron_backup_restore(
    conn: &Connection,
    folders: &[MeronBackupFolder],
) -> Result<(u32, HashMap<String, String>)> {
    let mut added = 0;
    let mut ids = HashMap::new();
    for folder in folders {
        let id = folder.id.trim();
        let Ok(name) = clean_name(&folder.name) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let kept = if meron_folder_exists(conn, id)? {
            id.to_string()
        } else if let Some(existing) = list_meron_folders(conn)?
            .into_iter()
            .find(|known| known.name.to_lowercase() == name.to_lowercase())
        {
            existing.id
        } else {
            // A folder already here keeps its own notification choice.
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO meron_folders(id, name, sort_order, created_at, notify)
                 VALUES(?1, ?2, ?3, ?4, ?5)",
                params![
                    id,
                    name,
                    folder.sort_order,
                    if folder.created_at > 0 {
                        folder.created_at
                    } else {
                        crate::store::now_unix()
                    },
                    folder.notify as i64
                ],
            )?;
            if inserted == 0 {
                continue;
            }
            added += 1;
            id.to_string()
        };
        ids.insert(id.to_string(), kept.clone());
        for entry in &folder.entries {
            let known: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
                params![entry.account],
                |row| row.get(0),
            )?;
            if !known || entry.id.trim().is_empty() {
                continue;
            }
            let fresh = conn.execute(
                "INSERT OR IGNORE INTO meron_folder_entries(id, account, folder_id, added_at)
                 VALUES(?1, ?2, ?3, ?4)",
                params![entry.id, entry.account, kept, entry.added_at],
            )? > 0;
            if !fresh {
                continue;
            }
            let mut any = false;
            for alias in &entry.aliases {
                if is_alias_key(alias) {
                    any |= conn.execute(
                        "INSERT OR IGNORE INTO meron_folder_aliases(account, key, entry_id)
                         VALUES(?1, ?2, ?3)",
                        params![entry.account, alias, entry.id],
                    )? > 0;
                }
            }
            if !any {
                conn.execute(
                    "DELETE FROM meron_folder_entries WHERE id = ?1",
                    params![entry.id],
                )?;
            }
        }
    }
    Ok((added, ids))
}
