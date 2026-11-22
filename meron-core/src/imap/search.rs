//! Search criteria and message identifiers.
//!
//! Builds Gmail and standard IMAP queries from the shared search model.
//! Returns mailbox-scoped identifiers for the engine to fetch or filter.

use super::*;

pub async fn search_uids(session: &mut Session, folder: &str, query: &str) -> Result<Vec<u32>> {
    session.select(folder).await.context("SELECT")?;
    let caps = capabilities_once(session).await.ok();
    let has = |name: &str| caps.as_ref().is_some_and(|caps| caps.has_str(name));
    let gmail = has("X-GM-EXT-1");
    // SEARCH keys are US-ASCII unless the command names a charset (RFC 3501
    // §6.4.4), so anything non-ASCII — a CJK or accented query — has to be
    // announced as UTF-8 or the server is entitled to answer BAD. The octets
    // themselves belong in a literal, since a quoted string is 7-bit only; a
    // non-synchronizing one (LITERAL+ / LITERAL-) goes inline in the command.
    // Servers with neither get the quoted form, which most accept anyway.
    let needs_charset = !query.is_ascii();
    let literal = needs_charset && (has("LITERAL+") || has("LITERAL-"));
    let mut result = session
        .uid_search(search_criteria(gmail, query, needs_charset, literal))
        .await;
    if result.is_err() && needs_charset {
        // Servers that reject CHARSET outright (or that advertise UTF8=ACCEPT
        // and take the raw octets) get one retry with the bare criteria before
        // we give up and leave the caller with the cached hits.
        result = session
            .uid_search(search_criteria(gmail, query, false, false))
            .await;
    }
    let set: HashSet<u32> = result.context("UID SEARCH query")?;
    let mut uids: Vec<u32> = set.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    Ok(uids)
}

/// SEARCH has no attachment key and HEADER sees only the top level, so
/// `has:attachment` rules out just what can't hold one: single-part plain text
/// or HTML that no header gives a file name. Everything else is a candidate for
/// the live search to check against its BODYSTRUCTURE.
pub(super) const ATTACHMENT_CANDIDATES: &str = "OR OR HEADER Content-Disposition \"\" \
     HEADER Content-Type \"name\" \
     (NOT HEADER Content-Type \"text/plain\" \
     NOT HEADER Content-Type \"text/html\")";

/// Build the SEARCH criteria for a text `query`.
///
/// On Gmail, defer to its own search engine via X-GM-RAW: it understands the full Gmail query
/// syntax (operators like `from:`, `has:attachment`, `older_than:`, relevance) instead of our crude
/// substring OR. Elsewhere the operators [`SearchQuery`] knows become their SEARCH keys, ANDed by
/// juxtaposition, and the free text falls back to plain SUBJECT/FROM/TEXT matching.
///
/// `literal` sends each string as a non-synchronizing literal instead of a quoted string.
pub(super) fn search_criteria(gmail: bool, query: &str, charset: bool, literal: bool) -> String {
    let string = |value: &str| {
        if literal {
            imap_literal_plus(value)
        } else {
            imap_quote(value)
        }
    };
    let keys = if gmail {
        format!("X-GM-RAW {}", string(&gmail_raw(query)))
    } else {
        let query = SearchQuery::parse(query);
        let mut keys = Vec::new();
        for text in &query.text {
            let query_text = string(text);
            keys.push(format!(
                "OR OR SUBJECT {query_text} FROM {query_text} TEXT {query_text}"
            ));
        }
        // A term's alternatives, ORed: IMAP's OR takes two keys, so nest it.
        let any = |values: &[String], key: &dyn Fn(String) -> String| {
            let mut keys = values.iter().rev().map(|v| key(string(v)));
            let last = keys.next().unwrap_or_default();
            keys.fold(last, |rest, key| format!("OR {key} {rest}"))
        };
        keys.extend(query.from.iter().map(|v| any(v, &|v| format!("FROM {v}"))));
        keys.extend(
            query
                .to
                .iter()
                .map(|v| any(v, &|v| format!("OR TO {v} CC {v}"))),
        );
        keys.extend(
            query
                .subject
                .iter()
                .map(|v| any(v, &|v| format!("SUBJECT {v}"))),
        );
        if query.read {
            keys.push("SEEN".to_string());
        }
        if query.unread {
            keys.push("UNSEEN".to_string());
        }
        if query.starred {
            keys.push("FLAGGED".to_string());
        }
        if query.has_attachment {
            keys.push(ATTACHMENT_CANDIDATES.to_string());
        }
        // The SENT* keys compare the Date header, which is what the cache
        // dates messages by, rather than the internal (arrival) date.
        if let Some(after) = query.after {
            keys.push(format!("SENTSINCE {}", after.format("%d-%b-%Y")));
        }
        if let Some(before) = query.before {
            keys.push(format!("SENTBEFORE {}", before.format("%d-%b-%Y")));
        }
        keys.join(" ")
    };
    if charset {
        format!("CHARSET UTF-8 {keys}")
    } else {
        keys
    }
}

/// `query` for Gmail's own search. Queries joined on separate lines (a saved
/// search narrowed by another, the later lines led by `AND`) are each grouped in
/// parentheses, with any quote or group they leave open closed, so Gmail keeps
/// one from running into the next as well.
pub(super) fn gmail_raw(query: &str) -> String {
    let lines: Vec<&str> = query
        .lines()
        .map(|line| {
            let line = line.trim();
            match line.strip_prefix("AND") {
                Some(rest) if rest.is_empty() || rest.starts_with(char::is_whitespace) => {
                    rest.trim_start()
                }
                _ => line,
            }
        })
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() < 2 {
        return query.to_string();
    }
    lines
        .iter()
        .map(|line| format!("({})", close_open_terms(line)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `value` as a non-synchronizing literal (RFC 7888), normalized like
/// [`imap_quote`]. LITERAL- caps these at 4096 octets, far above any query.
fn imap_literal_plus(value: &str) -> String {
    let value = value.trim().replace(['\r', '\n'], " ");
    format!("{{{}+}}\r\n{value}", value.len())
}

/// Every UID in `folder` at or above `floor`. `UID SEARCH UID n:*` names the
/// highest UID even when it is below `n`, so the result is filtered again.
pub async fn uids_from(session: &mut Session, folder: &str, floor: u32) -> Result<Vec<u32>> {
    session.select(folder).await.context("SELECT")?;
    let set: HashSet<u32> = session
        .uid_search(format!("UID {floor}:*"))
        .await
        .context("UID SEARCH UID range")?;
    let mut uids: Vec<u32> = set.into_iter().filter(|uid| *uid >= floor).collect();
    uids.sort_unstable();
    Ok(uids)
}

/// Return every UID currently in the folder. Used to prune locally cached
/// messages that have been moved or deleted by another client.
pub async fn list_all_uids(session: &mut Session, folder: &str) -> Result<HashSet<u32>> {
    session.select(folder).await.context("SELECT")?;
    let set: HashSet<u32> = session.uid_search("ALL").await.context("UID SEARCH ALL")?;
    Ok(set)
}

/// Every unseen UID in `folder`. Sync keeps the ones outside the cached window
/// so a folder's unread total matches the server, not just what is cached.
pub async fn list_unseen_uids(session: &mut Session, folder: &str) -> Result<HashSet<u32>> {
    session.select(folder).await.context("SELECT")?;
    let set: HashSet<u32> = session
        .uid_search("UNSEEN")
        .await
        .context("UID SEARCH UNSEEN")?;
    Ok(set)
}

/// Every `\Flagged` UID in `folder`, newest (highest) first.
pub async fn search_starred_uids(session: &mut Session, folder: &str) -> Result<Vec<u32>> {
    session.select(folder).await.context("SELECT")?;
    let set: HashSet<u32> = session
        .uid_search("FLAGGED")
        .await
        .context("UID SEARCH FLAGGED")?;
    let mut uids: Vec<u32> = set.into_iter().collect();
    uids.sort_unstable_by(|a, b| b.cmp(a));
    Ok(uids)
}
