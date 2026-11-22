//! Resolves the sender identities attached to each configured account.
//! Compose uses these queries to choose a From address and identify outgoing mail.
//! Card rendering shares the same display-name and alias rules.

use super::*;

/// An account's configured send-as aliases, for validating/labelling an outgoing
/// From against addresses the user actually owns.
pub fn account_aliases(conn: &Connection, id: &str) -> Result<Vec<Alias>> {
    let prefs: Option<String> = conn
        .query_row(
            "SELECT prefs FROM accounts WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match prefs {
        Some(prefs) => AccountPrefs::parse(&prefs).aliases(),
        None => Vec::new(),
    })
}

/// Resolve the `From` address and display name for an outgoing message, shared by desktop and mobile so both honor the same send-as rules.
///
/// An account may send as its own address, as the mailbox login, or as any configured alias; an
/// empty request means "the account's own address". The account address and the login are separate
/// fields (a custom IMAP account carries both, and they differ whenever the server authenticates by
/// user name), so both count as identities the user owns.
///
/// An address that is none of those is an error rather than a silent substitution: quietly
/// rewriting the sender is how a message goes out as the wrong identity without anyone noticing.
///
/// Note that a provider can still override the `From` we transmit — Gmail rewrites it to the
/// authenticated address unless the alias is verified in its own settings — which is outside what
/// this can enforce.
pub fn resolve_send_from(
    conn: &Connection,
    id: &str,
    login_user: &str,
    requested_from: &str,
) -> Result<(String, String)> {
    let (email, sender_name) = conn
        .query_row(
            "SELECT email, sender_name FROM accounts WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                    row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            },
        )
        .optional()?
        .unwrap_or_default();

    let login = login_user.trim();
    // Mirrors `account.list`, which falls back to the login when the account
    // was stored without an explicit address.
    let primary = if email.trim().is_empty() {
        login
    } else {
        email.trim()
    };

    let requested = requested_from.trim().to_lowercase();
    if requested.is_empty() || requested == primary.to_lowercase() {
        return Ok((primary.to_string(), sender_name));
    }
    if requested == login.to_lowercase() {
        return Ok((login.to_string(), sender_name));
    }
    match account_aliases(conn, id)?
        .into_iter()
        .find(|alias| alias.email.trim().to_lowercase() == requested)
    {
        Some(alias) => {
            let name = if alias.name.trim().is_empty() {
                sender_name
            } else {
                alias.name
            };
            Ok((alias.email.trim().to_string(), name))
        }
        None => Err(anyhow::anyhow!(
            "{} is not an address this account can send from",
            requested_from.trim()
        )),
    }
}

/// The account's own email addresses (primary + send-as aliases), lowercased, for
/// detecting which cached messages were sent *by* this account.
pub fn self_addrs(conn: &Connection, id: &str) -> std::collections::HashSet<String> {
    let mut addrs = std::collections::HashSet::new();
    if let Ok(Some(email)) = conn
        .query_row(
            "SELECT email FROM accounts WHERE id = ?1",
            params![id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
    {
        let email = email.trim().to_lowercase();
        if !email.is_empty() {
            addrs.insert(email);
        }
    }
    for alias in account_aliases(conn, id).unwrap_or_default() {
        let email = alias.email.trim().to_lowercase();
        if !email.is_empty() {
            addrs.insert(email);
        }
    }
    addrs
}

/// Every address the user owns, across all accounts (primary + send-as aliases),
/// lowercased. A reply excludes all of them rather than only the replying
/// account's: mail addressed to two of the user's own addresses should not come
/// back to either of them.
pub fn all_self_addrs(conn: &Connection) -> std::collections::HashSet<String> {
    let mut addrs = std::collections::HashSet::new();
    let Ok(mut stmt) = conn.prepare("SELECT email, prefs FROM accounts") else {
        return addrs;
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    }) else {
        return addrs;
    };
    for row in rows.flatten() {
        let (email, prefs) = row;
        let mut candidates = vec![email.unwrap_or_default()];
        if let Some(prefs) = prefs {
            candidates.extend(
                AccountPrefs::parse(&prefs)
                    .aliases()
                    .into_iter()
                    .map(|alias| alias.email),
            );
        }
        for candidate in candidates {
            let email = candidate.trim().to_lowercase();
            if !email.is_empty() {
                addrs.insert(email);
            }
        }
    }
    addrs
}

/// Determine whether this account sent a cached message.
///
/// A copy in Sent is outgoing even if the user sent it from an alias unknown to Meron.
/// In other folders, match the From address against the addresses from [`self_addrs`].
///
/// Delivery headers distinguish incoming mail from a colleague using a shared alias.
/// Pass [`crate::parse::Message::delivered`] when the body is cached, or false otherwise.
pub fn is_outgoing(
    mine: &std::collections::HashSet<String>,
    folder: &str,
    from_addr: &str,
    delivered: bool,
) -> bool {
    if crate::imap::looks_like_sent(folder) {
        return true;
    }
    is_outgoing_by_sender(mine, from_addr, delivered)
}

/// [`is_outgoing`] for a message of `account`. A local folder's name is the
/// user's own choice, not a server's Sent mailbox, so there only the sender
/// and delivery headers decide.
pub fn account_message_is_outgoing(
    account: &str,
    mine: &std::collections::HashSet<String>,
    folder: &str,
    from_addr: &str,
    delivered: bool,
) -> bool {
    if crate::store::is_local_account(account) {
        return is_outgoing_by_sender(mine, from_addr, delivered);
    }
    is_outgoing(mine, folder, from_addr, delivered)
}

fn is_outgoing_by_sender(
    mine: &std::collections::HashSet<String>,
    from_addr: &str,
    delivered: bool,
) -> bool {
    if delivered {
        return false;
    }
    let from = from_addr.trim().to_lowercase();
    !from.is_empty() && mine.contains(&from)
}

/// Show the correspondent on a thread card.
///
/// For outgoing mail with a usable recipient, show the first recipient in the From fields and
/// record the number of others in `recipient_overflow`. Use `folder` for rows without a source.
/// The transformation affects the displayed card. The stored envelope retains its original values.
///
/// For empty names and names containing the address itself, look up the best name already
/// cached for that address. The same correspondent can then be recognized across folders.
pub fn apply_card_identity(
    conn: &Connection,
    account: &str,
    folder: &str,
    headers: &mut [MessageHeader],
) {
    let mine = self_addrs(conn, account);
    let mut name_cache: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for header in headers {
        let source_folder = if header.folder.is_empty() {
            folder
        } else {
            header.folder.as_str()
        };
        // Envelope headers carry no delivery headers, so the thread card keeps
        // the address match; the reader and reply paths read the cached body.
        if account_message_is_outgoing(account, &mine, source_folder, &header.from_addr, false)
            && let Some(first) = header.to.first().cloned()
        {
            header.recipient_overflow = (header.to.len() - 1) as u32;
            header.from_name = first.name;
            header.from_addr = first.addr;
        }
        let addr = header.from_addr.trim().to_lowercase();
        if addr.is_empty() || !is_junk_display_name(&header.from_name, &addr) {
            continue;
        }
        let resolved = name_cache
            .entry(addr.clone())
            .or_insert_with(|| known_display_name(conn, account, &addr));
        // No better name anywhere: clear the junk so the UI falls back to the
        // address local part rather than showing "addr addr".
        header.from_name = resolved.clone().unwrap_or_default();
    }
}

/// A display name that adds no information over the address: empty, or one that
/// embeds the address itself (`"addr"`, `"addr addr"`).
fn is_junk_display_name(name: &str, addr_lower: &str) -> bool {
    let name = name.trim().to_lowercase();
    name.is_empty() || name.contains(addr_lower)
}

/// Best human display name seen for `addr` across the account's cached messages:
/// the most recent non-empty sender name that isn't itself junk.
fn known_display_name(conn: &Connection, account: &str, addr_lower: &str) -> Option<String> {
    conn.query_row(
        "SELECT from_name FROM messages
         WHERE account = ?1 AND lower(from_addr) = ?2
           AND TRIM(COALESCE(from_name, '')) <> ''
           AND instr(lower(from_name), ?2) = 0
         ORDER BY id DESC LIMIT 1",
        params![account, addr_lower],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
}
