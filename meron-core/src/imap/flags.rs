//! Message flags, keywords and concurrent sorting claims.

use super::*;

/// Read-state changes reconciled via CONDSTORE: which messages flipped \Seen or \Flagged (starred),
/// plus the folder's current HIGHESTMODSEQ to persist for the next incremental
/// fetch.
pub struct FlagSync {
    pub highest_modseq: u64,
    /// `(uid, seen, starred, tags, Meron folder mark)`.
    pub changes: Vec<(u32, bool, bool, Vec<String>, Option<Option<String>>)>,
    /// Every message's tags, when the folder still needed that first full read.
    pub tag_baseline: Option<Vec<(u32, Vec<String>)>>,
    /// `changes` holds every flag change since the given modseq: a cache that
    /// was current at that modseq needs no other flag reads. False when there
    /// was no usable baseline (no CONDSTORE, no prior modseq, new UIDVALIDITY).
    pub complete: bool,
}

/// Reconcile \Seen and \Flagged across an entire folder using CONDSTORE so updates made
/// on other devices are reflected even for messages older than the recent
/// window. Selects with `(CONDSTORE)` to learn HIGHESTMODSEQ, then fetches only
/// the messages whose flags changed since `since_modseq` (CHANGEDSINCE).
///
/// Returns no changes (but still reports HIGHESTMODSEQ) when there's no prior
/// baseline or UIDVALIDITY changed, and is a no-op on servers that do not
/// advertise CONDSTORE — the capability must be checked rather than probed.
/// Proton Mail Bridge answers `SELECT "INBOX" (CONDSTORE)` with an *untagged*
/// `BAD` (" BAD [Error offset=17]: expected CR"), so the client never matches a
/// response to its tag: instead of erroring, every later command on that
/// connection reads the previous command's reply. The desynced session then
/// reports empty mailboxes and an empty folder LIST while still looking
/// healthy, so it goes back into the pool and poisons the account's syncs.
pub async fn sync_flags(
    session: &mut Session,
    folder: &str,
    since_modseq: u64,
    validity_matches: bool,
) -> Result<FlagSync> {
    if !supports_condstore(session).await {
        session.select(folder).await.context("SELECT")?;
        return Ok(FlagSync {
            highest_modseq: 0,
            changes: Vec::new(),
            tag_baseline: None,
            complete: false,
        });
    }
    let mailbox = session
        .select_condstore(folder)
        .await
        .context("SELECT CONDSTORE")?;
    let highest_modseq = mailbox.highest_modseq.unwrap_or(0);

    if since_modseq == 0
        || !validity_matches
        || highest_modseq == 0
        || highest_modseq <= since_modseq
    {
        return Ok(FlagSync {
            highest_modseq,
            changes: Vec::new(),
            tag_baseline: None,
            // Nothing changed since the baseline: still a complete answer.
            complete: since_modseq != 0
                && validity_matches
                && highest_modseq != 0
                && highest_modseq == since_modseq,
        });
    }

    let query = format!("(FLAGS) (CHANGEDSINCE {since_modseq})");
    let mut stream = session
        .uid_fetch("1:*", query)
        .await
        .context("UID FETCH CHANGEDSINCE")?;
    let mut changes = Vec::new();
    while let Some(item) = stream.next().await {
        let fetch = item.context("flag fetch item")?;
        let uid = match fetch.uid {
            Some(uid) => uid,
            None => continue,
        };
        let seen = fetch
            .flags()
            .any(|flag| matches!(flag, async_imap::types::Flag::Seen));
        let starred = fetch
            .flags()
            .any(|flag| matches!(flag, async_imap::types::Flag::Flagged));
        let tags = fetch_tags(&fetch);
        changes.push((uid, seen, starred, tags, fetch_meron_mark(&fetch)));
    }
    drop(stream);
    Ok(FlagSync {
        highest_modseq,
        changes,
        tag_baseline: None,
        complete: true,
    })
}

/// Every message's tags in the selected mailbox. CONDSTORE only reports changes,
/// so mail cached before tags existed has to be read once in full.
pub async fn fetch_all_tags(session: &mut Session) -> Result<Vec<(u32, Vec<String>)>> {
    let mut stream = session
        .uid_fetch("1:*", "(FLAGS)")
        .await
        .context("UID FETCH tag baseline")?;
    let mut tags = Vec::new();
    while let Some(item) = stream.next().await {
        let fetch = item.context("tag baseline item")?;
        if let Some(uid) = fetch.uid {
            tags.push((uid, fetch_tags(&fetch)));
        }
    }
    Ok(tags)
}

/// SELECT `folder` as the preflight of a flag update. Pooled sessions die
/// silently, and the SELECT is the first command that notices, so running it
/// as a separate phase lets a stale connection be replaced before any STORE
/// reaches the server (see `Engine::with_flag_write_session`).
pub async fn prepare_flag_update(session: &mut Session, folder: &str) -> Result<()> {
    session.select(folder).await.context("SELECT")?;
    Ok(())
}

/// Select `folder` for a write of `tag`'s keyword, and say whether it can hold
/// it: it must allow new keywords, or already list this one as permanent.
pub async fn prepare_tag_update(session: &mut Session, folder: &str, tag: &str) -> Result<bool> {
    let mailbox = session.select(folder).await.context("SELECT")?;
    let keyword = crate::tags::tag_to_keyword(tag);
    Ok(mailbox.permanent_flags.iter().any(|flag| match flag {
        async_imap::types::Flag::MayCreate => true,
        async_imap::types::Flag::Custom(known) => known.eq_ignore_ascii_case(&keyword),
        _ => false,
    }))
}

/// The hidden keyword AI sorting leaves on mail once it decided on it (moved or
/// left), so other devices, and mail moved back, aren't sorted again. `$`
/// keywords are never shown as tags.
pub const AI_SORTED_KEYWORD: &str = "$MeronSorted";

pub(super) fn fetch_ai_sorted(fetch: &async_imap::types::Fetch) -> bool {
    fetch.flags().any(|flag| match flag {
        async_imap::types::Flag::Custom(keyword) => keyword.eq_ignore_ascii_case(AI_SORTED_KEYWORD),
        _ => false,
    })
}

/// Put [`AI_SORTED_KEYWORD`] on `uids` in `folder`, and take this device's
/// claim (`claim`: its id, see [`claim_for_ai_sort`]) off them: decided mail
/// needs none. Answers `false`, writing nothing, when the mailbox can't take
/// new keywords.
pub async fn mark_ai_sorted(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
    claim: Option<&str>,
) -> Result<bool> {
    if uids.is_empty() {
        return Ok(true);
    }
    let mailbox = session.select(folder).await.context("SELECT")?;
    let allowed = mailbox.permanent_flags.iter().any(|flag| match flag {
        async_imap::types::Flag::MayCreate => true,
        async_imap::types::Flag::Custom(known) => known.eq_ignore_ascii_case(AI_SORTED_KEYWORD),
        _ => false,
    });
    if !allowed {
        return Ok(false);
    }
    store_flag(
        session,
        uids,
        &format!("+FLAGS.SILENT ({AI_SORTED_KEYWORD})"),
    )
    .await?;
    if let Some(id) = claim {
        store_flag(
            session,
            uids,
            &format!("-FLAGS.SILENT ({AI_CLAIM_PREFIX}{id})"),
        )
        .await?;
    }
    Ok(true)
}

// ---- AI sorting claims -------------------------------------------------------
//
// Two of the reader's devices see the same new email at the same moment. So
// only one of them asks the AI about it (and moves it), a device first puts
// its claim on the email: the keyword `$MeronSorting_<its id>`. The claim goes
// again once the email is decided (it then carries `$MeronSorted`) or the
// device gave up on it.

/// What every claim keyword starts with; the claiming device's id follows.
pub const AI_CLAIM_PREFIX: &str = "$MeronSorting_";
/// Without CONDSTORE two devices can both write a claim: how long a device
/// waits before it reads back who else did, so the other's claim is there.
const AI_CLAIM_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// Who sorts the mail a device asked to claim (see [`claim_for_ai_sort`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiClaims {
    pub states: Vec<AiClaimState>,
    /// The server refused every claim but one (CONDSTORE). Otherwise two
    /// devices can each have read back only their own claim, and the winner
    /// asks once more before it acts on the mail (see `engine::ai_sort`).
    pub exact: bool,
}

/// Whether claims are written "only if unchanged": the server has CONDSTORE.
/// `MERON_AI_CLAIM_PLAIN` turns it off, for checks of the other way on a
/// server that has it.
async fn ai_claims_exact(session: &mut Session) -> bool {
    std::env::var_os("MERON_AI_CLAIM_PLAIN").is_none() && supports_condstore(session).await
}

/// What a message says about who sorts it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiClaimState {
    pub uid: u32,
    /// Some device's AI decided on it already ([`AI_SORTED_KEYWORD`]).
    pub sorted: bool,
    /// The ids of the devices whose claim it carries, lowercase, smallest first.
    pub claims: Vec<String>,
}

impl AiClaimState {
    /// Whether the device `id` is the one to sort this email: it isn't
    /// decided, the device's claim is on it, and of several claims (two
    /// devices wrote at once on a server that can't refuse the second) the
    /// smallest id's counts, so every device names the same winner.
    pub fn won_by(&self, id: &str) -> bool {
        !self.sorted
            && self
                .claims
                .first()
                .is_some_and(|first| first.eq_ignore_ascii_case(id))
    }
}

fn fetch_ai_claims(fetch: &async_imap::types::Fetch) -> Vec<String> {
    let mut claims: Vec<String> = fetch
        .flags()
        .filter_map(|flag| match flag {
            async_imap::types::Flag::Custom(keyword) => {
                let keyword = keyword.to_lowercase();
                keyword
                    .strip_prefix(&AI_CLAIM_PREFIX.to_lowercase())
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
            }
            _ => None,
        })
        .collect();
    claims.sort();
    claims.dedup();
    claims
}

/// [`ai_claim_states`] in the already selected mailbox.
async fn read_ai_claims(session: &mut Session, uids: &[u32]) -> Result<Vec<AiClaimState>> {
    let mut out = Vec::with_capacity(uids.len());
    for uid_set in uid_set_chunks(uids, MAX_UID_SET_LEN) {
        let mut stream = session
            .uid_fetch(uid_set, "(UID FLAGS)")
            .await
            .context("UID FETCH claims")?;
        while let Some(item) = stream.next().await {
            let fetch = item.context("UID FETCH claims item")?;
            let Some(uid) = fetch.uid else {
                continue;
            };
            if !fetch_has_flags(&fetch) {
                continue;
            }
            out.push(AiClaimState {
                uid,
                sorted: fetch_ai_sorted(&fetch),
                claims: fetch_ai_claims(&fetch),
            });
        }
    }
    Ok(out)
}

/// Who sorts each of `uids` in `folder`, as the server has it now. Mail no
/// longer in the folder is left out.
pub async fn ai_claim_states(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
) -> Result<Vec<AiClaimState>> {
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    session.examine(folder).await.context("EXAMINE")?;
    read_ai_claims(session, uids).await
}

/// Claim `uids` of `folder` for the device `id`, and answer who sorts each
/// now (see [`AiClaimState::won_by`]). `None`, writing nothing, when the
/// mailbox can't take new keywords: there is no claiming there.
///
/// Mail that is decided or carries a claim already is left alone. On a
/// server with CONDSTORE the claim is written "only if nothing changed since
/// I looked", so of two devices at the same instant the server refuses one.
/// Elsewhere both may write; each then reads back, the smallest id wins and
/// the other takes its own claim off again.
pub async fn claim_for_ai_sort(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
    id: &str,
) -> Result<Option<AiClaims>> {
    if uids.is_empty() {
        return Ok(Some(AiClaims::default()));
    }
    let keyword = format!("{AI_CLAIM_PREFIX}{id}");
    let condstore = ai_claims_exact(session).await;
    let mut exact = condstore;
    // Twice with CONDSTORE: another change to the mail (read elsewhere, say)
    // between the look and the write refuses the claim too, and nobody has it.
    let rounds = if condstore { 2 } else { 1 };
    for _ in 0..rounds {
        let mailbox = if condstore {
            session
                .select_condstore(folder)
                .await
                .context("SELECT CONDSTORE")?
        } else {
            session.select(folder).await.context("SELECT")?
        };
        let allowed = mailbox.permanent_flags.iter().any(|flag| match flag {
            async_imap::types::Flag::MayCreate => true,
            async_imap::types::Flag::Custom(known) => known.eq_ignore_ascii_case(&keyword),
            _ => false,
        });
        if !allowed {
            return Ok(None);
        }
        let free: Vec<u32> = read_ai_claims(session, uids)
            .await?
            .into_iter()
            .filter(|state| !state.sorted && state.claims.is_empty())
            .map(|state| state.uid)
            .collect();
        if free.is_empty() {
            break;
        }
        let op = match mailbox.highest_modseq {
            Some(seen) if condstore && seen > 0 => {
                format!("(UNCHANGEDSINCE {seen}) +FLAGS.SILENT ({keyword})")
            }
            _ => format!("+FLAGS.SILENT ({keyword})"),
        };
        // A refusal (the mail changed meanwhile) is no failure: who has the
        // claim is read back below either way.
        if let Err(err) = store_flag(session, &free, &op).await {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.aiSort",
                "folder={folder}: claim not written: {err:#}"
            );
        }
        if !condstore || mailbox.highest_modseq.unwrap_or(0) == 0 {
            exact = false;
            tokio::time::sleep(AI_CLAIM_SETTLE).await;
            break;
        }
    }
    let mut states = read_ai_claims(session, uids).await?;
    // Claimed together with a device that wins: this one's claim goes again.
    let lost: Vec<u32> = states
        .iter()
        .filter(|state| {
            state
                .claims
                .iter()
                .any(|claim| claim.eq_ignore_ascii_case(id))
                && !state.won_by(id)
        })
        .map(|state| state.uid)
        .collect();
    if !lost.is_empty() {
        store_flag(session, &lost, &format!("-FLAGS.SILENT ({keyword})")).await?;
        for state in states.iter_mut().filter(|state| lost.contains(&state.uid)) {
            state.claims.retain(|claim| !claim.eq_ignore_ascii_case(id));
        }
    }
    Ok(Some(AiClaims { states, exact }))
}

/// Take the claims of the devices `ids` off `uids` in `folder`: this device
/// gave up on the mail, or takes it over from a device that never finished.
pub async fn unclaim_for_ai_sort(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
    ids: &[String],
) -> Result<()> {
    if uids.is_empty() || ids.is_empty() {
        return Ok(());
    }
    session.select(folder).await.context("SELECT")?;
    let keywords: Vec<String> = ids
        .iter()
        .map(|id| format!("{AI_CLAIM_PREFIX}{id}"))
        .collect();
    store_flag(
        session,
        uids,
        &format!("-FLAGS.SILENT ({})", keywords.join(" ")),
    )
    .await
}

/// The UIDs of `folder` that carry `keyword`, ascending: the folder opened
/// read-only and one SEARCH, nothing fetched.
pub async fn uids_with_keyword(
    session: &mut Session,
    folder: &str,
    keyword: &str,
) -> Result<Vec<u32>> {
    session.examine(folder).await.context("EXAMINE")?;
    let found: HashSet<u32> = session
        .uid_search(format!("KEYWORD {keyword}"))
        .await
        .context("UID SEARCH KEYWORD")?;
    let mut uids: Vec<u32> = found.into_iter().collect();
    uids.sort_unstable();
    Ok(uids)
}

/// The Meron folder id a FETCH response's `$MeronF_<id>` keyword names
/// (`Some(None)`: no mark). Two devices writing at once can leave two: the
/// smallest id wins on every device, so they agree. `None` when the response
/// carries no FLAGS at all (a server's own note of a change, say): that says
/// nothing about marks and must not read as one taken away.
pub(super) fn fetch_meron_mark(fetch: &async_imap::types::Fetch) -> Option<Option<String>> {
    if !fetch_has_flags(fetch) {
        return None;
    }
    // Any normal folder's mark beats the built-in Archive's: a conversation
    // moved from Archive into another Meron folder can carry both a moment.
    Some(
        fetch
            .flags()
            .filter_map(|flag| match flag {
                async_imap::types::Flag::Custom(keyword) => {
                    crate::meron_marks::folder_of_keyword(&keyword)
                }
                _ => None,
            })
            .min_by(|a, b| {
                crate::store::meron_is_builtin(a)
                    .cmp(&crate::store::meron_is_builtin(b))
                    .then_with(|| a.cmp(b))
            }),
    )
}

/// Whether a FETCH response has a FLAGS item. `Fetch` only lists flags, so an
/// empty list and no FLAGS look alike; a response with a header came from
/// this app's own full FETCH, which always asks for FLAGS, and the small ones
/// left are told apart by their parsed form.
fn fetch_has_flags(fetch: &async_imap::types::Fetch) -> bool {
    fetch.flags().next().is_some()
        || fetch.header().is_some()
        || format!("{fetch:?}").contains("Flags(")
}

/// Write Meron-folder marks on `uids` of `folder`: take away the marks
/// `remove` lists, then add `add`. Answers `false`, writing nothing, when an
/// `add` is asked for and the mailbox can't take new keywords.
pub async fn store_meron_marks(
    session: &mut Session,
    folder: &str,
    uids: &[u32],
    remove: &[String],
    add: Option<&str>,
) -> Result<bool> {
    if uids.is_empty() {
        return Ok(true);
    }
    let mailbox = session.select(folder).await.context("SELECT")?;
    if let Some(add) = add {
        let allowed = mailbox.permanent_flags.iter().any(|flag| match flag {
            async_imap::types::Flag::MayCreate => true,
            async_imap::types::Flag::Custom(known) => known.eq_ignore_ascii_case(add),
            _ => false,
        });
        if !allowed {
            return Ok(false);
        }
    }
    let remove: Vec<&String> = remove
        .iter()
        .filter(|keyword| add.is_none_or(|add| !keyword.eq_ignore_ascii_case(add)))
        .collect();
    if !remove.is_empty() {
        let list = remove
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        store_flag(session, uids, &format!("-FLAGS.SILENT ({list})")).await?;
    }
    if let Some(add) = add {
        store_flag(session, uids, &format!("+FLAGS.SILENT ({add})")).await?;
    }
    Ok(true)
}

/// The tags among a FETCH response's keywords, sorted.
pub(super) fn fetch_tags(fetch: &async_imap::types::Fetch) -> Vec<String> {
    let mut tags: Vec<String> = fetch
        .flags()
        .filter_map(|flag| match flag {
            async_imap::types::Flag::Custom(keyword) => crate::tags::keyword_to_tag(&keyword),
            _ => None,
        })
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

/// Longest UID set sent in one command. RFC 7162 asks clients to keep command
/// lines under 8192 octets; this leaves room for the tag and the flag list.
pub(super) const MAX_UID_SET_LEN: usize = 7000;

/// Compact `uids` into IMAP sequence sets (`1:5,9,12:20`), split so no set is
/// longer than `max_len`. A folder-wide "mark all read" can cover tens of
/// thousands of UIDs, which as a flat list overruns server line limits.
pub(super) fn uid_set_chunks(uids: &[u32], max_len: usize) -> Vec<String> {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        i += 1;
        let range = if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        };
        if !current.is_empty() && current.len() + 1 + range.len() > max_len {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(',');
        }
        current.push_str(&range);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

async fn store_flag(session: &mut Session, uids: &[u32], op: &str) -> Result<()> {
    for uid_set in uid_set_chunks(uids, MAX_UID_SET_LEN) {
        let mut stream = session
            .uid_store(uid_set, op)
            .await
            .context("UID STORE FLAGS.SILENT")?;
        while let Some(item) = stream.next().await {
            item.context("UID STORE item")?;
        }
    }
    Ok(())
}

/// Add or remove a tag's keyword on a set of UIDs already in the selected
/// mailbox. The caller must first run [`prepare_tag_update`] on the session.
pub async fn store_tag(session: &mut Session, uids: &[u32], tag: &str, add: bool) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let sign = if add { '+' } else { '-' };
    let keyword = crate::tags::tag_to_keyword(tag);
    store_flag(session, uids, &format!("{sign}FLAGS.SILENT ({keyword})")).await
}

/// Add or remove the `\Seen` flag on a set of UIDs already in the selected
/// mailbox. The caller must first run [`prepare_flag_update`] on the session.
pub async fn store_seen(session: &mut Session, uids: &[u32], seen: bool) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let op = if seen {
        "+FLAGS.SILENT (\\Seen)"
    } else {
        "-FLAGS.SILENT (\\Seen)"
    };
    store_flag(session, uids, op).await
}

/// Add or remove the `\Flagged` flag on a set of UIDs already in the selected
/// mailbox. The caller must first run [`prepare_flag_update`] on the session.
pub async fn store_starred(session: &mut Session, uids: &[u32], starred: bool) -> Result<()> {
    if uids.is_empty() {
        return Ok(());
    }
    let op = if starred {
        "+FLAGS.SILENT (\\Flagged)"
    } else {
        "-FLAGS.SILENT (\\Flagged)"
    };
    store_flag(session, uids, op).await
}
