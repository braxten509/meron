//! What the reader taught AI sorting by moving mail by hand.
//!
//! A *lesson* is one email the reader put somewhere themselves: into Spam,
//! back into the Inbox, or into a Meron folder. AI sorting shows the lessons
//! most like a new email to the AI, which then decides (see
//! [`crate::ai_sort::lessons`]).
//!
//! A lesson lives in `ai_sort_lessons` and, so the reader's other devices
//! learn it too, as a system tag on the email itself: `meron.put.<place>`
//! (see [`LessonPlace::tag`]). Tags travel as IMAP keywords and follow the
//! message through moves, so [`ai_lessons_reconcile`] can bring the table in
//! line with them: a tag seen on mail without a lesson makes one, a lesson
//! whose tag went away (forgotten on another device) goes too.
//!
//! Lessons are only made while AI sorting is on, and never from the reader's
//! own mail.

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

/// The emails among `uids` of `folder` a lesson can be made of: not the
/// reader's own, one per sender and subject, newest first, at most
/// [`AI_LESSON_PER_ACTION`]. With `one_sender` (a move to Spam), a
/// conversation teaches only when all of it is from one sender, and then
/// through its newest email: when several people wrote in it, a colleague
/// answering a scam say, nothing tells which of them the Spam is.
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
        seen.insert((seed.from_addr.trim().to_lowercase(), subject_words(&seed.subject)))
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
        .query_map(params![account, key, format!("{LESSON_TAG_PREFIX}%")], |row| {
            row.get::<_, String>(0)
        })?
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
    std::mem::take(&mut *UNPUSHED.lock().unwrap()).into_iter().collect()
}

/// Drop the lessons past [`AI_LESSON_MAX`], oldest first, with their tags.
fn trim_lessons(conn: &Connection) -> Result<()> {
    let old: Vec<(i64, String, String)> = conn
        .prepare(
            "SELECT id, account, tag_key FROM ai_sort_lessons
             ORDER BY at DESC, id DESC LIMIT -1 OFFSET ?1",
        )?
        .query_map(params![AI_LESSON_MAX], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
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

/// Forget the lessons that put these messages in `place` (an Undo of the
/// move that taught them), with their tags. A lesson for another place was
/// taught by another action and stays. Answers whether there were any.
pub fn ai_lessons_forget_keys(
    conn: &Connection,
    account: &str,
    keys: &[String],
    place: &LessonPlace,
) -> Result<bool> {
    let tx = conn.unchecked_transaction()?;
    let mut any = false;
    for key in keys {
        let gone = tx.execute(
            "DELETE FROM ai_sort_lessons WHERE account = ?1 AND tag_key = ?2 AND place = ?3",
            params![account, key, place.code()],
        )?;
        if gone > 0 {
            any = true;
            set_lesson_tag(&tx, account, key, None)?;
        }
    }
    tx.commit()?;
    Ok(any)
}

/// Forget one lesson, with its tag. Answers with its account, for the tag
/// push, or `None` when there is no such lesson.
pub fn ai_lesson_forget(conn: &Connection, id: i64) -> Result<Option<String>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT account, tag_key FROM ai_sort_lessons WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((account, key)) = row else {
        return Ok(None);
    };
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM ai_sort_lessons WHERE id = ?1", params![id])?;
    set_lesson_tag(&tx, &account, &key, None)?;
    tx.commit()?;
    Ok(Some(account))
}

/// A removed account: its lessons go (its tags go with the account's).
pub fn ai_lessons_forget_account(conn: &Connection, account: &str) -> Result<()> {
    conn.execute("DELETE FROM ai_sort_lessons WHERE account = ?1", params![account])?;
    Ok(())
}

/// A deleted Meron folder: the lessons that point into it go, with their
/// tags. Inside the delete's transaction.
pub fn ai_lessons_forget_meron_folder(conn: &Connection, id: &str) -> Result<()> {
    let rows: Vec<(i64, String, String)> = conn
        .prepare("SELECT id, account, tag_key FROM ai_sort_lessons WHERE place = ?1")?
        .query_map(params![id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (lesson, account, key) in rows {
        conn.execute("DELETE FROM ai_sort_lessons WHERE id = ?1", params![lesson])?;
        set_lesson_tag(conn, &account, &key, None)?;
    }
    Ok(())
}

/// Meron folder ids that changed (a setup copy or restore kept another id for
/// the same folder): lessons and their tags follow.
pub fn ai_lessons_remap_meron_ids(
    conn: &Connection,
    map: &std::collections::HashMap<String, String>,
) -> Result<()> {
    for (old, kept) in map.iter().filter(|(old, kept)| old != kept) {
        let rows: Vec<(String, String)> = conn
            .prepare("SELECT account, tag_key FROM ai_sort_lessons WHERE place = ?1")?
            .query_map(params![old], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        conn.execute(
            "UPDATE ai_sort_lessons SET place = ?2 WHERE place = ?1",
            params![old, kept],
        )?;
        let place = LessonPlace::Meron(kept.clone());
        for (account, key) in rows {
            set_lesson_tag(conn, &account, &key, Some(&place))?;
        }
    }
    Ok(())
}

/// Bring the lessons in line with the lesson tags on mail (see the module
/// docs): what another device taught or forgot shows here once a sync has
/// read the tags. Cheap; run before the lessons are read.
pub fn ai_lessons_reconcile(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let pattern = format!("{LESSON_TAG_PREFIX}%");
    // Lessons whose tag changed or went away.
    let rows: Vec<(i64, String, String, String)> = tx
        .prepare("SELECT id, account, tag_key, place FROM ai_sort_lessons")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, account, key, place) in rows {
        if !key_travels(&key) || super::is_local_account(&account) {
            continue;
        }
        let tagged = live_lesson_tags(&tx, &account, &key)?
            .iter()
            .find_map(|tag| LessonPlace::from_tag(tag));
        match tagged {
            None => {
                tx.execute("DELETE FROM ai_sort_lessons WHERE id = ?1", params![id])?;
            }
            // Changed on another device, at a time this one can't know.
            // Spam must never win on a guess (see below): the lesson keeps
            // its time when it turns into a Spam one.
            Some(LessonPlace::Spam) if place != "spam" => {
                tx.execute(
                    "UPDATE ai_sort_lessons SET place = 'spam' WHERE id = ?1",
                    params![id],
                )?;
            }
            Some(now) if now.code() != place => {
                tx.execute(
                    "UPDATE ai_sort_lessons SET place = ?2, at = ?3 WHERE id = ?1",
                    params![id, now.code(), super::now_unix()],
                )?;
            }
            Some(_) => {}
        }
    }
    // Tags on mail without a lesson: taught on another device.
    let loose: Vec<(String, String, String)> = tx
        .prepare(&format!(
            "SELECT t.account, t.tag_key, t.tag FROM message_tags t
             WHERE t.tag LIKE ?1 AND t.state IN {LIVE_STATES}
               AND NOT EXISTS (SELECT 1 FROM ai_sort_lessons l
                               WHERE l.account = t.account AND l.tag_key = t.tag_key)
             ORDER BY t.updated_at DESC"
        ))?
        .query_map(params![pattern], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let own = super::contacts::own_addresses(&tx)?;
    // Only while there is room: what doesn't fit waits for a lesson to go,
    // rather than pushing out one that is then made again next time.
    let kept: i64 = tx.query_row("SELECT COUNT(*) FROM ai_sort_lessons", [], |row| row.get(0))?;
    let mut room = AI_LESSON_MAX - kept;
    let now = super::now_unix();
    for (account, key, tag) in loose {
        if room <= 0 {
            break;
        }
        if super::is_local_account(&account) {
            continue;
        }
        let Some(place) = LessonPlace::from_tag(&tag) else {
            continue;
        };
        let header: Option<(String, String, String, i64)> = tx
            .prepare_cached(&format!(
                "SELECT COALESCE(from_name, ''), COALESCE(from_addr, ''), COALESCE(subject, ''), date
                 FROM messages WHERE account = ?1 AND {TAG_KEY_SQL} = ?2 AND uid <> 0 LIMIT 1"
            ))?
            .query_row(params![account, key], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .optional()?;
        // Not cached here (yet): looked at again next time.
        let Some((from_name, from_addr, subject, date)) = header else {
            continue;
        };
        // When the other device taught it, this one can't know, and the
        // newest lesson decides whether Spam is open for such mail (see
        // `ai_sort::lessons::alike`). Spam must never win on a guess: a Spam
        // lesson counts as old as its email (it can't be older), any other
        // as new as now (it can't be newer).
        let at = match place {
            LessonPlace::Spam if date > 0 => date.min(now),
            _ => now,
        };
        let addr = from_addr.trim().to_lowercase();
        if addr.is_empty() || own.contains(&addr) {
            continue;
        }
        tx.execute(
            "INSERT OR IGNORE INTO ai_sort_lessons
               (account, tag_key, from_name, from_addr, subject, place, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                account,
                key,
                from_name.trim(),
                from_addr.trim(),
                subject.trim(),
                place.code(),
                at
            ],
        )?;
        room -= 1;
    }
    tx.commit()?;
    Ok(())
}

/// Every lesson, newest first, brought in line with the tags first.
pub fn ai_lessons(conn: &Connection) -> Result<Vec<AiLesson>> {
    ai_lessons_reconcile(conn)?;
    let lessons = conn
        .prepare(
            "SELECT id, account, tag_key, from_name, from_addr, subject, place, at
             FROM ai_sort_lessons ORDER BY at DESC, id DESC",
        )?
        .query_map([], |row| {
            Ok(AiLesson {
                id: row.get(0)?,
                account: row.get(1)?,
                tag_key: row.get(2)?,
                from_name: row.get(3)?,
                from_addr: row.get(4)?,
                subject: row.get(5)?,
                place: row.get(6)?,
                at: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(lessons)
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

/// The lesson in moving `uids` from `folder` to `target` on one account: Spam
/// for a move into the account's Spam folder, Inbox for a move into its
/// Inbox, nothing for any other folder. An Undo teaches nothing and forgets
/// the lesson the undone move made (the one for the folder the mail now
/// leaves). Never fails: a move goes on without its lesson.
pub fn ai_lesson_before_move(
    conn: &Connection,
    account: &str,
    folder: &str,
    target: &str,
    uids: &[u32],
    undo: bool,
) -> MoveLesson {
    let read = || -> Result<MoveLesson> {
        if super::is_local_account(account) {
            return Ok(MoveLesson::default());
        }
        let place_of = |folder: &str| -> Result<Option<LessonPlace>> {
            Ok(match super::folder_role(conn, account, folder)?.as_str() {
                "junk" => Some(LessonPlace::Spam),
                "inbox" => Some(LessonPlace::Inbox),
                _ => None,
            })
        };
        if undo {
            // The undone move put the mail in `folder`: what that taught goes.
            let Some(taught) = place_of(folder)? else {
                return Ok(MoveLesson::default());
            };
            let targets = std::collections::BTreeMap::from([(folder.to_string(), uids.to_vec())]);
            return Ok(MoveLesson {
                account: account.to_string(),
                keys: super::message_tag_keys(conn, account, &targets)?,
                place: Some(taught),
                undo: true,
                ..MoveLesson::default()
            });
        }
        // Nothing is learned while AI sorting is off (an Undo above still
        // forgets), and drafts and the reader's sent mail teach nothing.
        if !teaching(conn, account)?
            || matches!(super::folder_role(conn, account, folder)?.as_str(), "sent" | "drafts")
        {
            return Ok(MoveLesson::default());
        }
        let Some(place) = place_of(target)? else {
            return Ok(MoveLesson::default());
        };
        Ok(MoveLesson {
            account: account.to_string(),
            seeds: ai_lesson_seeds(conn, account, folder, uids, place == LessonPlace::Spam)?,
            place: Some(place),
            ..MoveLesson::default()
        })
    };
    read().unwrap_or_else(|err| {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.aiSort",
            "could not read a move's lesson: {err:#}"
        );
        MoveLesson::default()
    })
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
        done.unwrap_or_else(|err| {
            crate::mlog!(
                crate::log::Level::Warn,
                "mail.aiSort",
                "could not record a move's lesson: {err:#}"
            );
            false
        })
    }
}

/// The lesson in filing a whole conversation (Meron folder entry `entry_id`)
/// by hand, which leaves it in `place`: its newest email not the reader's own
/// teaches `place`. An Undo (`undo`) teaches nothing: it forgets what the
/// undone filing taught, the lessons of its mail for any other place. Inside
/// the caller's transaction, and before the entry is released when it is.
pub fn ai_lesson_of_entry_in(
    conn: &Connection,
    entry_id: &str,
    place: &LessonPlace,
    undo: bool,
) -> Result<()> {
    let account: Option<String> = conn
        .query_row(
            "SELECT account FROM meron_folder_entries WHERE id = ?1",
            params![entry_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(account) = account else {
        return Ok(());
    };
    // An Undo forgets also while AI sorting is off.
    if super::is_local_account(&account) || (!undo && !teaching(conn, &account)?) {
        return Ok(());
    }
    let own = super::contacts::own_addresses(conn)?;
    let seeds: Vec<LessonSeed> = conn
        .prepare_cached(&format!(
            "SELECT {key}, COALESCE(m.from_name, ''), COALESCE(m.from_addr, ''),
                    COALESCE(m.subject, ''), m.date
             FROM meron_folder_uids u
             JOIN messages m ON m.account = u.account AND m.folder = u.folder AND m.uid = u.uid
             WHERE u.entry_id = ?1 ORDER BY m.date DESC, m.uid DESC",
            key = super::tags::qualified_tag_key("m"),
        ))?
        .query_map(params![entry_id], |row| {
            Ok(LessonSeed {
                tag_key: row.get(0)?,
                from_name: row.get(1)?,
                from_addr: row.get(2)?,
                subject: row.get(3)?,
                date: row.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    if undo {
        for seed in &seeds {
            let gone = conn.execute(
                "DELETE FROM ai_sort_lessons WHERE account = ?1 AND tag_key = ?2 AND place <> ?3",
                params![account, seed.tag_key, place.code()],
            )?;
            if gone > 0 {
                set_lesson_tag(conn, &account, &seed.tag_key, None)?;
            }
        }
        return Ok(());
    }
    let newest = seeds.into_iter().find(|seed| {
        let addr = seed.from_addr.trim().to_lowercase();
        !addr.is_empty() && !own.contains(&addr)
    });
    match newest {
        Some(seed) => teach_in(conn, &account, &[seed], place),
        None => Ok(()),
    }
}

/// The lesson in "Move back" on a row of the AI's log: the AI was wrong about
/// this email, which belongs in the Inbox. Made of what the row kept (sender,
/// subject, Message-ID); a row without a Message-ID or sender address teaches
/// nothing. Never fails the move back.
pub fn ai_lesson_from_log(conn: &Connection, entry: &super::AiSortLogEntry) {
    let tag_key = entry
        .message_id
        .trim()
        .trim_matches(|c| c == ' ' || c == '<' || c == '>')
        .to_lowercase();
    let sender = entry.sender.trim();
    let (from_name, from_addr) = match sender.rsplit_once('<') {
        Some((name, addr)) if addr.ends_with('>') => {
            (name.trim().to_string(), addr.trim_end_matches('>').trim().to_string())
        }
        _ if sender.contains('@') => (String::new(), sender.to_string()),
        _ => (sender.to_string(), String::new()),
    };
    if tag_key.is_empty() || from_addr.is_empty() {
        return;
    }
    let seed = LessonSeed {
        tag_key,
        from_name,
        from_addr,
        subject: entry.subject.clone(),
        date: entry.at,
    };
    if let Err(err) = ai_lessons_teach(conn, &entry.account, &[seed], &LessonPlace::Inbox) {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.aiSort",
            "could not record a move back's lesson: {err:#}"
        );
    }
}

/// Whether any of `uids` in `folder` is mail this device hasn't read `tag` on
/// yet: not cached, or cached without it. Such mail carries a lesson another
/// device taught, which a sync of the folder brings here.
pub fn ai_lesson_tag_unread(
    conn: &Connection,
    account: &str,
    folder: &str,
    uids: &[u32],
    tag: &str,
) -> Result<bool> {
    let mut stmt = conn.prepare_cached(
        "SELECT server_tags FROM messages WHERE account = ?1 AND folder = ?2 AND uid = ?3",
    )?;
    for uid in uids {
        let read: Option<Option<String>> = stmt
            .query_row(params![account, folder, uid], |row| row.get(0))
            .optional()?;
        let known = read
            .flatten()
            .is_some_and(|tags| tags.split(' ').any(|known| known == tag));
        if !known {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The UIDs of `folder` this device last read `tag` on.
pub fn ai_lesson_tagged_uids(
    conn: &Connection,
    account: &str,
    folder: &str,
    tag: &str,
) -> Result<Vec<u32>> {
    let rows: Vec<(u32, String)> = conn
        .prepare_cached(
            "SELECT uid, server_tags FROM messages
             WHERE account = ?1 AND folder = ?2 AND server_tags LIKE ?3",
        )?
        .query_map(params![account, folder, format!("%{tag}%")], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .into_iter()
        .filter(|(_, tags)| tags.split(' ').any(|known| known == tag))
        .map(|(uid, _)| uid)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn(sorting_on: bool) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        super::super::run_migrations(&conn).unwrap();
        super::super::setting_set(
            &conn,
            super::super::AI_SORT_KEY,
            &serde_json::json!({ "enabled": sorting_on, "model": "m" }),
        )
        .unwrap();
        conn
    }

    fn insert(conn: &Connection, folder: &str, uid: u32, addr: &str, subject: &str, id: &str) {
        conn.execute(
            "INSERT INTO messages(account, folder, msg_id, uid, subject, from_name, from_addr, date, json)
             VALUES('acct', ?1, ?2, ?2, ?3, 'Shop', ?4, 100, ?5)",
            params![folder, uid, subject, addr, serde_json::json!({ "message_id": id }).to_string()],
        )
        .unwrap();
    }

    fn places(conn: &Connection) -> Vec<(String, String)> {
        ai_lessons(conn)
            .unwrap()
            .into_iter()
            .map(|lesson| (lesson.from_addr, lesson.place))
            .collect()
    }

    fn tagged(conn: &Connection, place: &LessonPlace) -> Vec<String> {
        let mut keys: Vec<String> = super::super::keys_with_tag(conn, "acct", &place.tag())
            .unwrap()
            .into_iter()
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn a_move_to_spam_teaches_and_its_undo_forgets() {
        // While AI sorting is off, moves teach nothing.
        let off = test_conn(false);
        insert(&off, "INBOX", 1, "promo@shop.example", "Big sale", "<m1@shop>");
        assert!(!ai_lesson_before_move(&off, "acct", "INBOX", "Junk", &[1], false).learn(&off));
        assert!(places(&off).is_empty());

        let conn = test_conn(true);
        insert(&conn, "INBOX", 1, "promo@shop.example", "Big sale", "<m1@shop>");
        // A move to a plain folder is no lesson; one to Spam is.
        assert!(!ai_lesson_before_move(&conn, "acct", "INBOX", "Receipts", &[1], false).learn(&conn));
        let lesson = ai_lesson_before_move(&conn, "acct", "INBOX", "Junk", &[1], false);
        conn.execute("UPDATE messages SET folder = 'Junk', uid = 9, msg_id = 9", []).unwrap();
        assert!(lesson.learn(&conn));
        assert_eq!(places(&conn), [("promo@shop.example".to_string(), "spam".to_string())]);
        // The email carries the lesson for the reader's other devices.
        assert_eq!(tagged(&conn, &LessonPlace::Spam), ["m1@shop"]);

        // Undo moves it back out of Spam: the lesson and its tag go.
        let undo = ai_lesson_before_move(&conn, "acct", "Junk", "INBOX", &[9], true);
        assert!(undo.learn(&conn));
        assert!(places(&conn).is_empty());
        assert!(tagged(&conn, &LessonPlace::Spam).is_empty());

        // Moved back to the Inbox by hand (not an Undo): keep such mail there.
        let back = ai_lesson_before_move(&conn, "acct", "Junk", "INBOX", &[9], false);
        assert!(back.learn(&conn));
        assert_eq!(places(&conn), [("promo@shop.example".to_string(), "inbox".to_string())]);
        assert_eq!(tagged(&conn, &LessonPlace::Inbox), ["m1@shop"]);

        // A conversation with a colleague's answer in it, sent to Spam whole:
        // nothing tells whose mail the Spam is, so nobody is taught as Spam.
        insert(&conn, "INBOX", 2, "phish@bad.example", "Your account", "<m3@bad>");
        insert(&conn, "INBOX", 3, "colleague@work.example", "Re: Your account", "<m4@work>");
        insert(&conn, "INBOX", 4, "bulk@ads.example", "Deal one", "<m5@ads>");
        insert(&conn, "INBOX", 5, "bulk@ads.example", "Deal two", "<m6@ads>");
        conn.execute("UPDATE messages SET thread_key = 't1' WHERE uid IN (2, 3)", []).unwrap();
        conn.execute("UPDATE messages SET thread_key = 't2', date = uid WHERE uid IN (4, 5)", [])
            .unwrap();
        let spam: Vec<String> = ai_lesson_seeds(&conn, "acct", "INBOX", &[2, 3, 4, 5], true)
            .unwrap()
            .into_iter()
            .map(|seed| seed.subject)
            .collect();
        assert_eq!(spam, ["Deal two"]);
    }

    #[test]
    fn another_devices_tag_becomes_a_lesson_and_its_removal_forgets_it() {
        let conn = test_conn(true);
        insert(&conn, "Junk", 5, "spam@bulk.example", "You won", "<m2@bulk>");
        // A sync reads the lesson tag another device put on the email.
        let seen = [LessonPlace::Spam.tag()];
        super::super::observe_server_tags(&conn, "acct", "Junk", 5, &seen).unwrap();
        assert_eq!(places(&conn), [("spam@bulk.example".to_string(), "spam".to_string())]);
        // When it was taught isn't known: a Spam lesson counts as old as its
        // email, so it never beats a newer "keep in the Inbox" on a guess.
        assert_eq!(ai_lessons(&conn).unwrap()[0].at, 100);

        // The other device forgot the lesson: the tag is gone at the next sync.
        super::super::observe_server_tags(&conn, "acct", "Junk", 5, &[]).unwrap();
        assert!(places(&conn).is_empty());
    }
}
