//! What the reader taught AI sorting by moving mail by hand.
//!
//! A *lesson* is one email the reader put somewhere themselves: into Spam, back into the Inbox, or
//! into a Meron folder. AI sorting shows the lessons most like a new email to the AI, which then
//! decides (see [`crate::ai_sort::lessons`]).
//!
//! A lesson lives in `ai_sort_lessons` and, so the reader's other devices learn it too, as a system
//! tag on the email itself: `meron.put.<place>` (see [`LessonPlace::tag`]).
//!
//! Tags travel as IMAP keywords and follow the message through moves, so [`ai_lessons_reconcile`]
//! can bring the table in line with them: a tag seen on mail without a lesson makes one, a lesson
//! whose tag went away (forgotten on another device) goes too.
//!
//! Lessons are only made while AI sorting is on, and never from the reader's own mail.

mod reconciliation;
pub use reconciliation::{
    ai_lesson_before_move, ai_lesson_forget, ai_lesson_from_log, ai_lesson_of_entry_in,
    ai_lesson_tag_unread, ai_lesson_tagged_uids, ai_lessons, ai_lessons_forget_account,
    ai_lessons_forget_keys, ai_lessons_forget_meron_folder, ai_lessons_reconcile,
    ai_lessons_remap_meron_ids,
};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeSet;

use super::tags::{LIVE_STATES, TAG_KEY_SQL, write_tag_changes};

/// Most lessons kept; the oldest go first.
pub const AI_LESSON_MAX: i64 = 1000;
/// Most lessons one action makes (a whole sender row moved at once).
pub const AI_LESSON_PER_ACTION: usize = 20;

/// What every lesson tag starts with.
pub const LESSON_TAG_PREFIX: &str = "meron.put.";

/// Where the reader put an email.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LessonPlace {
    Spam,
    /// Kept in, or moved back to, the Inbox.
    Inbox,
    /// The Meron folder's id.
    Meron(String),
}

impl LessonPlace {
    /// The place as stored and sent to the apps: `spam`, `inbox` or the Meron
    /// folder's id.
    pub fn code(&self) -> String {
        match self {
            LessonPlace::Spam => "spam".to_string(),
            LessonPlace::Inbox => "inbox".to_string(),
            LessonPlace::Meron(id) => id.clone(),
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "spam" => Some(LessonPlace::Spam),
            "inbox" => Some(LessonPlace::Inbox),
            id if id.starts_with(super::MERON_FOLDER_ID_PREFIX)
                && id.len() > super::MERON_FOLDER_ID_PREFIX.len() =>
            {
                Some(LessonPlace::Meron(id.to_string()))
            }
            _ => None,
        }
    }

    /// The system tag that carries this lesson on the email.
    pub fn tag(&self) -> String {
        format!("{LESSON_TAG_PREFIX}{}", self.code())
    }

    pub fn from_tag(tag: &str) -> Option<Self> {
        Self::from_code(tag.strip_prefix(LESSON_TAG_PREFIX)?)
    }
}

/// One lesson.
#[derive(Clone, Debug, PartialEq)]
pub struct AiLesson {
    pub id: i64,
    pub account: String,
    pub tag_key: String,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    /// See [`LessonPlace::code`].
    pub place: String,
    pub at: i64,
}

/// An email a lesson can be made of, read while it is still cached where the
/// reader acts on it.
#[derive(Clone, Debug, PartialEq)]
pub struct LessonSeed {
    pub tag_key: String,
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub date: i64,
}

/// Whether a tag key names the message itself (its Message-ID) rather than a
/// place in a mailbox: only such a key still names it after a move, so only
/// such a lesson is carried by a tag.
fn key_travels(key: &str) -> bool {
    !key.starts_with("uid:")
}

fn subject_words(subject: &str) -> String {
    subject
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Select stored messages that qualify as examples of a filing choice.
///
/// The emails among `uids` of `folder` a lesson can be made of: not the reader's own, one per sender and
/// subject, newest first, at most [`AI_LESSON_PER_ACTION`]. With `one_sender` (a move to Spam), a conversation
/// teaches only when all of it is from one sender, and then through its newest email: when several people wrote
/// in it, a colleague answering a scam say, nothing tells which of them the Spam is.
pub fn ai_lesson_seeds(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
    one_sender: bool,
) -> Result<Vec<LessonSeed>> {
    let own = super::contacts::own_addresses(conn)?;
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {TAG_KEY_SQL}, COALESCE(from_name, ''), COALESCE(from_addr, ''),
                COALESCE(subject, ''), date, lower(COALESCE(thread_key, ''))
         FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3"
    ))?;
    let mut seeds: Vec<(LessonSeed, String)> = Vec::new();
    for uid in uids {
        let seed = stmt
            .query_row(params![account, folder, uid], |row| {
                Ok((
                    LessonSeed {
                        tag_key: row.get(0)?,
                        from_name: row.get(1)?,
                        from_addr: row.get(2)?,
                        subject: row.get(3)?,
                        date: row.get(4)?,
                    },
                    row.get::<_, String>(5)?,
                ))
            })
            .optional()?;
        if let Some((seed, thread)) = seed {
            let addr = seed.from_addr.trim().to_lowercase();
            if !addr.is_empty() && !own.contains(&addr) {
                seeds.push((seed, thread));
            }
        }
    }
    seeds.sort_by(|a, b| b.0.date.cmp(&a.0.date));
    if one_sender {
        let mut senders: std::collections::BTreeMap<String, BTreeSet<String>> = Default::default();
        for (seed, thread) in &seeds {
            senders
                .entry(thread.clone())
                .or_default()
                .insert(seed.from_addr.trim().to_lowercase());
        }
        let mut threads = BTreeSet::new();
        seeds.retain(|(_, thread)| {
            thread.is_empty() || (senders[thread].len() == 1 && threads.insert(thread.clone()))
        });
    }
    let mut seeds: Vec<LessonSeed> = seeds.into_iter().map(|(seed, _)| seed).collect();
    let mut seen = BTreeSet::new();
    seeds.retain(|seed| {
        seen.insert((
            seed.from_addr.trim().to_lowercase(),
            subject_words(&seed.subject),
        ))
    });
    seeds.truncate(AI_LESSON_PER_ACTION);
    Ok(seeds)
}

/// The lesson tags a message carries now.
fn live_lesson_tags(conn: &Connection, account: &str, key: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT tag FROM message_tags
         WHERE account = ?1 AND tag_key = ?2 AND tag LIKE ?3 AND state IN {LIVE_STATES}
         ORDER BY updated_at DESC, tag"
    ))?;
    let tags = stmt
        .query_map(
            params![account, key, format!("{LESSON_TAG_PREFIX}%")],
            |row| row.get::<_, String>(0),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(tags)
}

/// Give the message `place`'s tag and take its other lesson tags (`None`:
/// take them all).
fn set_lesson_tag(
    conn: &Connection,
    account: &str,
    key: &str,
    place: Option<&LessonPlace>,
) -> Result<()> {
    if !key_travels(key) || super::is_local_account(account) {
        return Ok(());
    }
    let add: Vec<String> = place.map(LessonPlace::tag).into_iter().collect();
    let remove: Vec<String> = live_lesson_tags(conn, account, key)?
        .into_iter()
        .filter(|tag| !add.contains(tag))
        .collect();
    write_tag_changes(conn, account, &[key.to_string()], &add, &remove, true)?;
    UNPUSHED.lock().unwrap().insert(account.to_string());
    Ok(())
}

/// Accounts whose lesson tags changed here and wait to be pushed.
static UNPUSHED: std::sync::Mutex<BTreeSet<String>> = std::sync::Mutex::new(BTreeSet::new());

/// The accounts whose lesson tags changed since the last call, for the
/// caller to push (see `engine::push_lesson_tags`). A push never asked for
/// still happens with the account's next sync, as for every tag.
pub fn ai_lessons_take_unpushed() -> Vec<String> {
    std::mem::take(&mut *UNPUSHED.lock().unwrap())
        .into_iter()
        .collect()
}

/// Drop the lessons past [`AI_LESSON_MAX`], oldest first, with their tags.
fn trim_lessons(conn: &Connection) -> Result<()> {
    let old: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT id, account, tag_key FROM ai_sort_lessons
             ORDER BY at DESC, id DESC LIMIT -1 OFFSET ?1",
        )?
        .query_map(params![AI_LESSON_MAX], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (id, account, key) in old {
        conn.execute("DELETE FROM ai_sort_lessons WHERE id = ?1", params![id])?;
        set_lesson_tag(conn, &account, &key, None)?;
    }
    Ok(())
}

/// [`ai_lessons_teach`], inside the caller's transaction and whatever the
/// setting says.
fn teach_in(
    conn: &Connection,
    account: &str,
    seeds: &[LessonSeed],
    place: &LessonPlace,
) -> Result<()> {
    let now = super::now_unix();
    for seed in seeds {
        conn.execute(
            "INSERT INTO ai_sort_lessons(account, tag_key, from_name, from_addr, subject, place, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(account, tag_key) DO UPDATE SET
               from_name = excluded.from_name, from_addr = excluded.from_addr,
               subject = excluded.subject, place = excluded.place, at = excluded.at",
            params![
                account,
                seed.tag_key,
                seed.from_name.trim(),
                seed.from_addr.trim(),
                seed.subject.trim(),
                place.code(),
                now
            ],
        )?;
        set_lesson_tag(conn, account, &seed.tag_key, Some(place))?;
    }
    trim_lessons(conn)
}

/// Whether the reader's moves teach right now: AI sorting is on, and the mail
/// is on a real mail account.
fn teaching(conn: &Connection, account: &str) -> Result<bool> {
    Ok(!super::is_local_account(account) && super::ai_sort_config(conn)?.enabled)
}

/// Remember that the reader put these emails in `place`. Answers whether
/// anything was learned (the caller then pushes tags to the server). Nothing
/// is learned while AI sorting is off.
pub fn ai_lessons_teach(
    conn: &Connection,
    account: &str,
    seeds: &[LessonSeed],
    place: &LessonPlace,
) -> Result<bool> {
    if seeds.is_empty() || !teaching(conn, account)? {
        return Ok(false);
    }
    let tx = conn.unchecked_transaction()?;
    teach_in(&tx, account, seeds, place)?;
    tx.commit()?;
    Ok(true)
}

/// What a move by hand teaches, read before the move while the mail is still
/// cached in its folder, and recorded with [`MoveLesson::learn`] once the
/// move succeeded.
#[derive(Clone, Debug, Default)]
pub struct MoveLesson {
    account: String,
    seeds: Vec<LessonSeed>,
    keys: Vec<String>,
    place: Option<LessonPlace>,
    undo: bool,
}

impl MoveLesson {
    /// Record the lesson after the move succeeded. Answers whether tags
    /// changed, for the caller to push them.
    pub fn learn(&self, conn: &Connection) -> bool {
        let Some(place) = &self.place else {
            return false;
        };
        let done = if self.undo {
            ai_lessons_forget_keys(conn, &self.account, &self.keys, place)
        } else {
            ai_lessons_teach(conn, &self.account, &self.seeds, place)
        };
        done.unwrap_or_else(|operation_error| {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.aiSort",
                "could not record a move's lesson: {operation_error:#}"
            );
            false
        })
    }
}

#[cfg(test)]
mod tests;
