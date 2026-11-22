//! Loads starred message pages from the local database.
//! The folder and unified views select their scope before paging by message identity.
//! Each query returns the headers required to render its next set of cards.

use super::*;

pub fn get_starred(
    conn: &Connection,
    account: &str,
    folder: &str,
    limit: u32,
) -> Result<Vec<MessageHeader>> {
    Ok(get_starred_page(conn, account, folder, limit, None)?.0)
}

/// One newest-first page of a folder's starred mail, keyset paged like
/// [`get_recent_page`] (`date:<date>:<uid>` cursors). Conversations in a Meron
/// folder are left out inside the query, before the LIMIT, so pages stay full.
pub fn get_starred_page(
    conn: &Connection,
    account: &str,
    folder: &str,
    limit: u32,
    before_cursor: Option<(i64, u32)>,
) -> Result<(Vec<MessageHeader>, Option<String>)> {
    get_starred_page_from(conn, account, folder, limit, before_cursor, None)
}

/// [`get_starred_page`] continuing a cursor that carries its folder
/// tie-breaker (see [`format_mail_cursor`]). The Inbox's Starred view also
/// lists starred mail the AI shows in the Inbox from other folders (each row
/// carrying its real folder), which its own folder leaves out.
pub fn get_starred_page_from(
    conn: &Connection,
    account: &str,
    folder: &str,
    limit: u32,
    before_cursor: Option<(i64, u32)>,
    before_folder: Option<&str>,
) -> Result<(Vec<MessageHeader>, Option<String>)> {
    let hide_meron = meron_hides_folder(conn, account, folder)?;
    let mixed = folder.eq_ignore_ascii_case("INBOX")
        && crate::store::ai_sort_inbox_has_rows(conn, account)?;
    // Mail the AI shows in the Inbox from here is left out, as in the folder's list.
    let shown = if crate::store::ai_sort_inbox_uids(conn, account, folder)?.is_empty() {
        String::new()
    } else {
        format!(" AND NOT {}", crate::store::ai_sort_inbox_sql("messages"))
    };
    let columns = "uid, subject, from_name, from_addr, date, seen, starred, thread_key,
                json_extract(json, '$.to'), folder";
    let sql = if mixed {
        // As in `get_recent_page_from`: folder breaks (date, uid) ties.
        let narrow = format!(
            "starred <> 0 AND uid <> 0 AND (?6 = 0 OR NOT {hidden})
               AND (?4 IS NULL OR date < ?4 OR (date = ?4 AND (uid < ?5
                    OR (?7 IS NOT NULL AND uid = ?5 AND folder < ?7))))",
            hidden = meron_hidden_sql("messages"),
        );
        format!(
            "SELECT {columns}, 0 FROM messages WHERE account = ?1 AND folder = ?2 AND {narrow}
             UNION ALL
             SELECT {columns}, 1 FROM messages WHERE id IN {ids} AND {narrow}
             ORDER BY 5 DESC, 1 DESC, 10 DESC LIMIT ?3",
            ids = crate::store::ai_sort_inbox_ids_sql("?1"),
        )
    } else {
        format!(
            "SELECT {columns}, 0 FROM messages
             WHERE account = ?1 AND folder = ?2 AND starred <> 0 AND uid <> 0
               AND (?6 = 0 OR NOT {hidden}){shown}
               AND (?4 IS NULL OR date < ?4 OR (date = ?4 AND uid < ?5))
             ORDER BY date DESC, uid DESC LIMIT ?3",
            hidden = meron_hidden_sql("messages"),
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let map = |row: &rusqlite::Row<'_>| {
        let mut header = message_header_from_row(row)?;
        header.folder = row.get(9)?;
        // Mail shown in the Inbox from another folder: a card of its own.
        header.ai_shown = row.get::<_, i64>(10)? != 0;
        Ok(header)
    };
    let base = params![
        account,
        folder,
        limit.saturating_add(1) as i64,
        before_cursor.map(|(date, _)| date),
        before_cursor.map(|(_, uid)| uid as i64).unwrap_or(0),
        hide_meron as i64
    ];
    let mut out: Vec<MessageHeader> = if mixed {
        let mut all: Vec<&dyn rusqlite::ToSql> = base.to_vec();
        all.push(&before_folder);
        stmt.query_map(all.as_slice(), map)?
            .collect::<rusqlite::Result<_>>()?
    } else {
        stmt.query_map(base, map)?
            .collect::<rusqlite::Result<_>>()?
    };
    let has_more = out.len() > limit as usize;
    if has_more {
        out.truncate(limit as usize);
    }
    let next_cursor = has_more.then(|| out.last()).flatten().map(|header| {
        format_mail_cursor(
            header.date,
            header.uid,
            mixed.then_some(header.folder.as_str()),
        )
    });
    Ok((out, next_cursor))
}

/// What makes two cached rows the same starred conversation.
///
/// A Message-ID or `gmthrid:` thread key names the conversation itself and holds wherever the
/// server files it, so those dedupe across folders.
///
/// `uid:` is the reserved key [`crate::store::card_thread_key`] mints for a message that arrived
/// with no threading headers at all, and an IMAP UID means nothing outside its own mailbox —
/// `uid:1` in Inbox and `uid:1` in Archive are ordinarily two unrelated messages, so those stay
/// scoped to their folder.
///
/// Both the query budget below and the card-level dedupe in `mail_model::starred_thread_cards` key
/// on this, so a row that survives the budget as its own conversation cannot then be swallowed as a
/// copy of another, or vice versa.
pub fn starred_thread_identity(folder: &str, thread_key: &str) -> String {
    if thread_key.starts_with("uid:") {
        format!("{folder}\u{0}{thread_key}")
    } else {
        thread_key.to_string()
    }
}

/// Every starred mail message across all accounts and folders, newest first, for the newest `max_threads` distinct conversations.
///
/// The budget counts conversations rather than rows because the caller renders one card per
/// conversation: an account whose server files each message under several folders (Gmail's All Mail
/// beside the Inbox copy, plus every label) would otherwise spend the whole budget on copies of the
/// same few threads and silently drop older ones off the end of the list.
///
/// A conversation past the budget is skipped rather than ending the scan, since its rows carry no
/// signal about where the remaining copies of the admitted ones are: two folders' copies of one
/// message can hold different cached dates, which puts them arbitrarily far apart in this ordering.
///
/// Stopping at the first over-budget row would drop those stragglers, and the caller picks which
/// folder's copy to show from exactly this set — losing the Inbox copy would put the thread under
/// All Mail instead.
///
/// RSS rows carry `uid = 0` and are excluded; `rss::starred_items` covers them.
pub fn get_starred_all_accounts(
    conn: &Connection,
    max_threads: u32,
) -> Result<Vec<(String, MessageHeader)>> {
    let mut stmt = conn.prepare(
        "SELECT uid, subject, from_name, from_addr, date, seen, starred, thread_key,
                json_extract(json, '$.to'), account, folder FROM messages
         WHERE starred <> 0 AND uid <> 0
         ORDER BY date DESC, uid DESC",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    let mut threads: HashSet<(String, String)> = HashSet::new();
    while let Some(row) = rows.next()? {
        let mut header = message_header_from_row(row)?;
        header.folder = row.get(10)?;
        let account: String = row.get(9)?;
        // The branch-aware key, not the stored root: one root split by subject
        // becomes one card per branch, and a budget counting the root would let
        // a heavily branched conversation return far more cards than it allows.
        let thread = (
            account,
            starred_thread_identity(&header.folder, &card_thread_key(&header)),
        );
        if !threads.contains(&thread) {
            if threads.len() as u32 >= max_threads {
                continue;
            }
            threads.insert(thread.clone());
        }
        out.push((thread.0, header));
    }
    Ok(out)
}
