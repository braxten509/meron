//! UID-validated move and delete operations.

use super::*;

/// MCP must never fall back to folder-wide EXPUNGE: unrelated messages could
/// already carry \Deleted. Validate the mailbox generation before touching flags.
pub async fn expunge_uids_checked(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
    validity: u32,
) -> Result<()> {
    let caps = capabilities_once(session).await.context("CAPABILITY")?;
    anyhow::ensure!(
        caps.has_str("UIDPLUS"),
        "This operation requires IMAP UIDPLUS; no messages were changed"
    );
    let selected = session.select(folder).await.context("SELECT")?;
    anyhow::ensure!(
        validity != 0 && selected.uid_validity == Some(validity),
        "Mailbox changed; request a new approval"
    );
    expunge_selected_uids(session, uids).await
}

async fn expunge_selected_uids(session: &mut Session, uids: &[u32]) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let uid_sets = uid_set_chunks(uids, MAX_UID_SET_LEN);
    store_deleted(session, &uid_sets).await?;
    uid_expunge(session, &uid_sets).await
}

/// Flag every UID in `uid_sets` `\Deleted` in the selected mailbox.
pub(super) async fn store_deleted(session: &mut Session, uid_sets: &[String]) -> Result<()> {
    for uid_set in uid_sets {
        let mut stream = session
            .uid_store(uid_set, "+FLAGS.SILENT (\\Deleted)")
            .await
            .context("UID STORE Deleted")?;
        while let Some(item) = stream.next().await {
            item.context("UID STORE item")?;
        }
    }
    Ok(())
}

/// UID EXPUNGE (RFC 4315) exactly the UIDs in `uid_sets`, leaving any other
/// `\Deleted` message in the mailbox alone.
pub(super) async fn uid_expunge(session: &mut Session, uid_sets: &[String]) -> Result<()> {
    for uid_set in uid_sets {
        let stream = session.uid_expunge(uid_set).await.context("UID EXPUNGE")?;
        futures::pin_mut!(stream);
        while let Some(item) = stream.next().await {
            item.context("UID EXPUNGE item")?;
        }
    }
    Ok(())
}

/// Move-only behavior for MCP organization, including messages in Drafts/Trash.
pub async fn move_to_folder_checked(
    session: &mut Session,
    folder: &str,
    target: &str,
    uids: &[u32],
) -> Result<()> {
    let caps = capabilities_once(session).await.context("CAPABILITY")?;
    let supports_move = caps.has_str("MOVE");
    anyhow::ensure!(
        supports_move || caps.has_str("UIDPLUS"),
        "Moving through MCP requires IMAP MOVE or UIDPLUS; no messages were changed"
    );
    session.select(folder).await.context("SELECT")?;
    for uid_set in uid_set_chunks(uids, MAX_UID_SET_LEN) {
        if supports_move {
            session.uid_mv(&uid_set, target).await.context("UID MOVE")?;
        } else {
            session
                .uid_copy(&uid_set, target)
                .await
                .context("UID COPY")?;
        }
    }
    if !supports_move {
        expunge_selected_uids(session, uids).await?;
    }
    Ok(())
}
