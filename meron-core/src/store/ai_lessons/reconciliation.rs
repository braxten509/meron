//! Maintains learned filing examples when mail, tags or folder IDs change.
//! The sorting engine reads matching examples and reconciles them with current entries.
//! Cleanup follows account removal, folder deletion and backup restoration.

use super::*;

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
    conn.execute(
        "DELETE FROM ai_sort_lessons WHERE account = ?1",
        params![account],
    )?;
    Ok(())
}

/// A deleted Meron folder: the lessons that point into it go, with their
/// tags. Inside the delete's transaction.
pub fn ai_lessons_forget_meron_folder(conn: &Connection, id: &str) -> Result<()> {
    let rows: Vec<(i64, String, String)> = conn
        .prepare("SELECT id, account, tag_key FROM ai_sort_lessons WHERE place = ?1")?
        .query_map(params![id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
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
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    for (id, account, key, place) in rows {
        if !key_travels(&key) || crate::store::is_local_account(&account) {
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
                    params![id, now.code(), crate::store::now_unix()],
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
        .query_map(params![pattern], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let own = crate::store::contacts::own_addresses(&tx)?;
    // Only while there is room: what doesn't fit waits for a lesson to go,
    // rather than pushing out one that is then made again next time.
    let kept: i64 = tx.query_row("SELECT COUNT(*) FROM ai_sort_lessons", [], |row| row.get(0))?;
    let mut room = AI_LESSON_MAX - kept;
    let now = crate::store::now_unix();
    for (account, key, tag) in loose {
        if room <= 0 {
            break;
        }
        if crate::store::is_local_account(&account) {
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

/// The lesson in moving `uids` from `folder` to `target` on one account: Spam for a move into the
/// account's Spam folder, Inbox for a move into its Inbox, nothing for any other folder. An Undo
/// teaches nothing and forgets the lesson the undone move made (the one for the folder the mail now
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
        if crate::store::is_local_account(account) {
            return Ok(MoveLesson::default());
        }
        let place_of = |folder: &str| -> Result<Option<LessonPlace>> {
            Ok(
                match crate::store::folder_role(conn, account, folder)?.as_str() {
                    "junk" => Some(LessonPlace::Spam),
                    "inbox" => Some(LessonPlace::Inbox),
                    _ => None,
                },
            )
        };
        if undo {
            // The undone move put the mail in `folder`: what that taught goes.
            let Some(taught) = place_of(folder)? else {
                return Ok(MoveLesson::default());
            };
            let targets = std::collections::BTreeMap::from([(folder.to_string(), uids.to_vec())]);
            return Ok(MoveLesson {
                account: account.to_string(),
                keys: crate::store::message_tag_keys(conn, account, &targets)?,
                place: Some(taught),
                undo: true,
                ..MoveLesson::default()
            });
        }
        // Nothing is learned while AI sorting is off (an Undo above still
        // forgets), and drafts and the reader's sent mail teach nothing.
        if !teaching(conn, account)?
            || matches!(
                crate::store::folder_role(conn, account, folder)?.as_str(),
                "sent" | "drafts"
            )
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
    read().unwrap_or_else(|error| {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.aiSort",
            "could not read a move's lesson: {error:#}"
        );
        MoveLesson::default()
    })
}

/// The lesson in filing a whole conversation (Meron folder entry `entry_id`) by hand, which leaves
/// it in `place`: its newest email not the reader's own teaches `place`. An Undo (`undo`) teaches
/// nothing: it forgets what the undone filing taught, the lessons of its mail for any other place.
/// Inside the caller's transaction, and before the entry is released when it is.
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
    if crate::store::is_local_account(&account) || (!undo && !teaching(conn, &account)?) {
        return Ok(());
    }
    let own = crate::store::contacts::own_addresses(conn)?;
    let seeds: Vec<LessonSeed> = conn
        .prepare_cached(&format!(
            "SELECT {key}, COALESCE(m.from_name, ''), COALESCE(m.from_addr, ''),
                    COALESCE(m.subject, ''), m.date
             FROM meron_folder_uids u
             JOIN messages m ON m.account = u.account AND m.folder = u.folder AND m.uid = u.uid
             WHERE u.entry_id = ?1 ORDER BY m.date DESC, m.uid DESC",
            key = crate::store::tags::qualified_tag_key("m"),
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
pub fn ai_lesson_from_log(conn: &Connection, entry: &crate::store::AiSortLogEntry) {
    let tag_key = entry
        .message_id
        .trim()
        .trim_matches(|c| c == ' ' || c == '<' || c == '>')
        .to_lowercase();
    let sender = entry.sender.trim();
    let (from_name, from_addr) = match sender.rsplit_once('<') {
        Some((name, addr)) if addr.ends_with('>') => (
            name.trim().to_string(),
            addr.trim_end_matches('>').trim().to_string(),
        ),
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
    if let Err(error) = ai_lessons_teach(conn, &entry.account, &[seed], &LessonPlace::Inbox) {
        crate::mlog!(
            crate::log::Level::Warn,
            "mail.aiSort",
            "could not record a move back's lesson: {error:#}"
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
