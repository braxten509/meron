//! Manages the folder hierarchy in a local mail archive.
//! The desktop local-account commands normalize names and resolve whole subtrees.
//! Creation, renaming and deletion operate on the archive's SQLite connection.

use super::*;

/// Validate and normalize a local folder path: trimmed segments joined by
/// [`LOCAL_DELIMITER`], none of them empty, and "inbox" in any case spelled
/// `INBOX` — the list, thread-id and read paths canonicalize folder names that
/// way for every account, so a local folder must be stored under the same form.
pub fn normalize_local_folder_name(name: &str) -> Result<String> {
    Ok(canon_local_folder(&tidy_folder_path(name)?))
}

/// A local folder path with a top-level "inbox" in any case spelled `INBOX`,
/// so the folder and everything nested under it share one spelling.
pub fn canon_local_folder(path: &str) -> String {
    match path.split_once(LOCAL_DELIMITER) {
        Some((top, rest)) => format!(
            "{}{LOCAL_DELIMITER}{rest}",
            crate::mail_model::canon_folder(top)
        ),
        None => crate::mail_model::canon_folder(path),
    }
}

fn tidy_folder_path(name: &str) -> Result<String> {
    let segments = name
        .split(LOCAL_DELIMITER)
        .map(str::trim)
        .collect::<Vec<_>>();
    if segments.iter().any(|segment| segment.is_empty()) {
        bail!("Folder name is required");
    }
    Ok(segments.join(LOCAL_DELIMITER))
}

/// Create a local folder, and any missing parents so the tree stays whole.
/// Like a server folder, it is addressed by its modified UTF-7 name, so every
/// folder id the app passes around has the same form.
pub fn create_local_folder(conn: &Connection, display_name: &str) -> Result<Folder> {
    let display_name = tidy_folder_path(display_name)?;
    let name = canon_local_folder(&crate::utf7::encode(&display_name));
    if crate::store::folder_exists(conn, LOCAL_ACCOUNT_ID, &name)? {
        bail!("A folder named {display_name} already exists");
    }
    let tx = conn.unchecked_transaction()?;
    let segments = name.split(LOCAL_DELIMITER).collect::<Vec<_>>();
    for depth in 1..=segments.len() {
        let path = segments[..depth].join(LOCAL_DELIMITER);
        tx.execute(
            "INSERT OR IGNORE INTO folders(account, name, delimiter, special_use) VALUES(?1, ?2, ?3, ?4)",
            params![LOCAL_ACCOUNT_ID, path, LOCAL_DELIMITER, LOCAL_SPECIAL_USE],
        )?;
        // A local folder is always "synced": the empty state must not offer
        // to wait for a server.
        tx.execute(
            "INSERT OR IGNORE INTO folder_state(account, folder, uidvalidity, uid_next) VALUES(?1, ?2, 1, 1)",
            params![LOCAL_ACCOUNT_ID, path],
        )?;
    }
    tx.commit()?;
    Ok(Folder {
        display_name,
        name,
        delimiter: Some(LOCAL_DELIMITER.to_string()),
        special_use: Some(LOCAL_SPECIAL_USE.to_string()),
        role: "folder".to_string(),
        ..Default::default()
    })
}

/// The folder and every folder nested under it, deepest first.
pub fn local_folder_subtree(conn: &Connection, name: &str) -> Result<Vec<String>> {
    if !crate::store::folder_exists(conn, LOCAL_ACCOUNT_ID, name)? {
        bail!("Folder not found: {name}");
    }
    let mut targets = crate::store::child_folders(conn, LOCAL_ACCOUNT_ID, name)?;
    targets.push(name.to_string());
    targets.sort_by_key(|folder| std::cmp::Reverse(folder.matches(LOCAL_DELIMITER).count()));
    Ok(targets)
}

/// Permanently delete a local folder, its subfolders and all the mail in them.
/// Returns the folders removed and how many messages went with them.
pub fn delete_local_folder(conn: &Connection, name: &str) -> Result<(Vec<String>, usize)> {
    let targets = local_folder_subtree(conn, name)?;
    let mut deleted = 0;
    for folder in &targets {
        conn.execute("DELETE FROM archive_raw WHERE folder = ?1", params![folder])?;
        deleted += crate::store::delete_folder(conn, LOCAL_ACCOUNT_ID, folder)?;
    }
    reclaim_space(conn);
    Ok((targets, deleted))
}

/// Rename a local folder, moving its subfolders and their mail with it, and
/// create any missing parents of the new name. Returns each folder's old and
/// new name; none when the name is unchanged.
pub fn rename_local_folder(
    conn: &Connection,
    name: &str,
    display_name: &str,
    media_root: &Path,
) -> Result<Vec<(String, String)>> {
    let display_name = tidy_folder_path(display_name)?;
    let target = canon_local_folder(&crate::utf7::encode(&display_name));
    let sources = local_folder_subtree(conn, name)?;
    if target == name {
        return Ok(Vec::new());
    }
    if target.starts_with(&format!("{name}{LOCAL_DELIMITER}")) {
        bail!("A folder cannot be moved inside itself");
    }
    let pairs = sources
        .iter()
        .map(|from| (from.clone(), format!("{target}{}", &from[name.len()..])))
        .collect::<Vec<_>>();
    for (_, to) in &pairs {
        if crate::store::folder_exists(conn, LOCAL_ACCOUNT_ID, to)? {
            bail!("A folder named {} already exists", crate::utf7::decode(to));
        }
    }

    // Every row moves in one transaction, so an interruption leaves the folder
    // whole under one name or the other. Attachment files and index rows are
    // written again afterwards; opening the archive redoes any that are missing
    // (`repair_local_index`).
    let mut moved = Vec::with_capacity(pairs.len());
    let tx = conn.unchecked_transaction()?;
    // The parents of the new name, as creating it would add them.
    let segments = target.split(LOCAL_DELIMITER).collect::<Vec<_>>();
    for depth in 1..segments.len() {
        let path = segments[..depth].join(LOCAL_DELIMITER);
        tx.execute(
            "INSERT OR IGNORE INTO folders(account, name, delimiter, special_use) VALUES(?1, ?2, ?3, ?4)",
            params![LOCAL_ACCOUNT_ID, path, LOCAL_DELIMITER, LOCAL_SPECIAL_USE],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO folder_state(account, folder, uidvalidity, uid_next) VALUES(?1, ?2, 1, 1)",
            params![LOCAL_ACCOUNT_ID, path],
        )?;
    }
    for (from, to) in &pairs {
        moved.push(local_folder_uids(&tx, from)?);
        tx.execute(
            "INSERT INTO folders(account, name, delimiter, special_use) VALUES(?1, ?2, ?3, ?4)",
            params![LOCAL_ACCOUNT_ID, to, LOCAL_DELIMITER, LOCAL_SPECIAL_USE],
        )?;
        // Keeping the UID counter, so new mail does not reuse a moved UID.
        tx.execute(
            "INSERT INTO folder_state(account, folder, uidvalidity, uid_next)
             SELECT account, ?3, uidvalidity, uid_next FROM folder_state WHERE account = ?1 AND folder = ?2",
            params![LOCAL_ACCOUNT_ID, from, to],
        )?;
        tx.execute(
            "UPDATE archive_raw SET folder = ?2 WHERE folder = ?1",
            params![from, to],
        )?;
        crate::store::delete_folder_rows(&tx, LOCAL_ACCOUNT_ID, from)?;
    }
    tx.commit()?;

    // Attachment files are keyed by folder, so each message's are written again
    // under the new name.
    for ((from, to), uids) in pairs.iter().zip(&moved) {
        for &uid in uids {
            crate::parse::remove_message_media(media_root, LOCAL_ACCOUNT_ID, from, uid);
            index_raw(conn, to, uid, media_root)?;
        }
    }
    Ok(pairs)
}
