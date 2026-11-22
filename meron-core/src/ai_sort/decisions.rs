//! Interprets model decisions for a message and the folders available to it.
//! Single-message and batch responses share validation for confident and uncertain choices.
//! The sorting engine records each accepted answer in the local store.

use super::*;

/// The label a decision model sees for the folder at `index`: `f1`, `f2`, ….
fn decision_label(index: usize) -> String {
    format!("f{}", index + 1)
}

/// One email as a decision model reads it: the same sender, subject and start
/// the chat request carries.
pub fn decision_state(email: &PromptEmail) -> String {
    let mut text = format!("From: {}\nSubject: {}", email.from, email.subject);
    if let Some(reason) = &email.automated {
        text.push_str(&format!("\nBulk or automated mail: {reason}"));
    }
    if let Some(start) = email.start.as_deref().filter(|start| !start.is_empty()) {
        text.push_str("\n\n");
        text.push_str(start);
    }
    if !email.earlier.is_empty() {
        text.push_str("\n\nEmails the owner moved by hand before (newest first):");
        for lesson in &email.earlier {
            text.push_str(&format!(
                "\n- From: {} | Subject: {} | Moved to: {}",
                lesson.from, lesson.subject, lesson.moved_to
            ));
        }
    }
    text
}

/// The labels and what each means: one per folder (its note, else its name)
/// and "leave".
pub fn decision_criteria(folders: &[PromptFolder]) -> serde_json::Map<String, Value> {
    decision_criteria_taught(folders, None)
}

/// [`decision_criteria`], with the Spam label when the email is backed for it.
pub fn decision_criteria_taught(
    folders: &[PromptFolder],
    spam: Option<&SpamChoice>,
) -> serde_json::Map<String, Value> {
    let mut criteria = serde_json::Map::new();
    if let Some(spam) = spam {
        criteria.insert(
            DECISION_SPAM.to_string(),
            json!(format!(
                "{}: only when this is the same kind of email from the same sender as an \
                 earlier email the owner moved to {}.",
                spam.name, spam.name
            )),
        );
    }
    for (index, folder) in folders.iter().take(254).enumerate() {
        let note = folder.note.trim();
        let meaning = if note.is_empty() {
            folder.name.clone()
        } else {
            format!("{}: {note}", folder.name)
        };
        criteria.insert(decision_label(index), json!(meaning));
    }
    criteria.insert(
        DECISION_LEAVE.to_string(),
        json!("None of these fit; leave it in the Inbox."),
    );
    criteria
}

/// The `/systemone` request for one email.
pub fn decision_body(model: &str, folders: &[PromptFolder], email: &PromptEmail) -> Value {
    decision_body_taught(model, folders, email, None)
}

/// [`decision_body`] with the reader's lessons: `spam` is a label only when
/// the email is backed for it.
pub fn decision_body_taught(
    model: &str,
    folders: &[PromptFolder],
    email: &PromptEmail,
    spam: Option<&SpamChoice>,
) -> Value {
    let spam = spam.filter(|_| email.spam_backed);
    let mut instructions = DECISION_INSTRUCTIONS.to_string();
    if !email.earlier.is_empty() {
        instructions.push_str(DECISION_EARLIER);
    }
    json!({
        "model": model,
        "state": decision_state(email),
        "questions": {
            DECISION_QUESTION: {
                "type": "choice",
                "instructions": instructions,
                "criteria": decision_criteria_taught(folders, spam),
            }
        }
    })
}

/// About how many characters a decision request carries besides the email.
pub fn decision_overhead(folders: &[PromptFolder]) -> usize {
    DECISION_INSTRUCTIONS.len() + Value::Object(decision_criteria(folders)).to_string().len()
}

/// Read a decision answer: the chosen label and its probability. "leave", or a
/// folder below [`DECISION_SURE`], stays in the Inbox; a refusal or an answer
/// that can't be read leaves it too, as an unreadable chat answer does.
pub fn parse_decision(value: &Value, folders: &[PromptFolder]) -> (Option<Pick>, String) {
    parse_decision_for(value, folders, false)
}

/// [`parse_decision`]; for bulk or automated mail (`automated`), the label with
/// the top probability is used even below [`DECISION_SURE`], unless that label
/// is "leave".
pub fn parse_decision_for(
    value: &Value,
    folders: &[PromptFolder],
    automated: bool,
) -> (Option<Pick>, String) {
    parse_decision_taught(value, folders, automated, None)
}

/// [`parse_decision_for`], with the Spam choice when the email was backed for
/// it: Spam needs [`DECISION_SPAM_SURE`], bulk or automated mail included.
pub fn parse_decision_taught(
    value: &Value,
    folders: &[PromptFolder],
    automated: bool,
    spam: Option<&SpamChoice>,
) -> (Option<Pick>, String) {
    let Some(answer) = value.pointer(&format!("/answers/{DECISION_QUESTION}")) else {
        return (None, String::new());
    };
    if answer
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "choice")
    {
        return (None, "No answer".to_string());
    }
    let Some(mut choice) = answer.get("choice").and_then(Value::as_str).map(str::trim) else {
        return (None, String::new());
    };
    let probabilities = answer.get("probabilities").and_then(Value::as_object);
    if automated {
        let top = probabilities.and_then(|all| {
            all.iter()
                .filter_map(|(label, p)| Some((label.as_str(), p.as_f64()?)))
                .max_by(|a, b| a.1.total_cmp(&b.1))
        });
        if let Some((label, _)) = top {
            choice = label.trim();
        }
    }
    let probability = probabilities
        .and_then(|all| all.get(choice))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let percent = (probability * 100.0).round().clamp(0.0, 100.0) as u32;
    if choice == DECISION_LEAVE {
        return (None, format!("None fit ({percent}%)"));
    }
    if choice == DECISION_SPAM {
        let why = format!("{percent}% sure");
        let pick = spam
            .filter(|_| probability >= DECISION_SPAM_SURE)
            .map(|spam| Pick {
                target: spam.target(),
                why: why.clone(),
            });
        return (pick, why);
    }
    let Some(folder) = (0..folders.len().min(254))
        .find(|index| decision_label(*index) == choice)
        .map(|index| &folders[index])
    else {
        return (None, String::new());
    };
    let why = format!("{percent}% sure");
    if automated {
        let why = automated_why(&why);
        return (
            Some(Pick {
                target: folder.target.clone(),
                why: why.clone(),
            }),
            why,
        );
    }
    if probability < DECISION_SURE {
        return (None, why);
    }
    (
        Some(Pick {
            target: folder.target.clone(),
            why: why.clone(),
        }),
        why,
    )
}

/// A 1-token request, to check a key.
pub fn probe_body(model: &str) -> Value {
    json!({
        "model": model,
        "messages": [{ "role": "user", "content": "Say OK." }],
        "max_tokens": 1,
        "temperature": 0
    })
}

/// Read an answer, tolerantly (fences, text around the JSON), and apply the "not sure" rule: an
/// email moves only when its id is known and answered once, `sure` is true, and its folder is one
/// of the list (exactly, else ignoring case). Answers by id: `Some` to move, `None` to leave in the
/// Inbox (also for every id the answer left out). Unreadable answers leave everything.
pub fn parse_answer(
    text: &str,
    ids: &[String],
    folders: &[PromptFolder],
) -> BTreeMap<String, (Option<Pick>, String)> {
    parse_answer_flagged(text, ids, folders, &HashSet::new())
}

/// [`parse_answer`], except that for the ids in `automated` (bulk or automated
/// mail) a folder from the list is used even when `sure` is not true; only
/// "Inbox" or no usable folder keeps those in the Inbox.
pub fn parse_answer_flagged(
    text: &str,
    ids: &[String],
    folders: &[PromptFolder],
    automated: &HashSet<String>,
) -> BTreeMap<String, (Option<Pick>, String)> {
    parse_answer_taught(text, ids, folders, automated, None)
}

/// [`parse_answer_flagged`], with the Spam choice of the request and the ids it is backed for (see
/// [`PromptEmail::spam_backed`]). Spam is the careful answer: it only counts for a backed id and
/// with `sure` true, bulk or automated mail included; any other Spam answer keeps the email in the
/// Inbox.
pub fn parse_answer_taught(
    text: &str,
    ids: &[String],
    folders: &[PromptFolder],
    automated: &HashSet<String>,
    spam: Option<(&SpamChoice, &HashSet<String>)>,
) -> BTreeMap<String, (Option<Pick>, String)> {
    let mut out: BTreeMap<String, (Option<Pick>, String)> = ids
        .iter()
        .map(|id| (id.clone(), (None, String::new())))
        .collect();
    let Some(value) = answer_json(text) else {
        return out;
    };
    let entries = value
        .get("emails")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let id_of = |entry: &Value| match entry.get("id") {
        Some(Value::String(id)) => Some(id.trim().to_string()),
        Some(Value::Number(id)) => Some(id.to_string()),
        _ => None,
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    for entry in &entries {
        if let Some(id) = id_of(entry) {
            *counts.entry(id).or_default() += 1;
        }
    }
    for entry in &entries {
        let Some(id) = id_of(entry) else {
            continue;
        };
        let Some(slot) = out.get_mut(&id) else {
            continue;
        };
        let why: String = entry
            .get("why")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(80)
            .collect();
        slot.1 = why.clone();
        if counts.get(&id).copied() != Some(1) {
            continue;
        }
        let picked = entry
            .get("folder")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let sure = entry.get("sure").and_then(Value::as_bool) == Some(true);
        if let Some((spam, backed)) = spam
            && spam.name.eq_ignore_ascii_case(picked)
        {
            if sure && backed.contains(&id) {
                slot.0 = Some(Pick {
                    target: spam.target(),
                    why,
                });
            }
            continue;
        }
        let flagged = automated.contains(&id);
        if !flagged && !sure {
            continue;
        }
        let why = if flagged { automated_why(&why) } else { why };
        let folder = folders
            .iter()
            .find(|folder| folder.name == picked)
            .or_else(|| {
                folders
                    .iter()
                    .find(|folder| folder.name.eq_ignore_ascii_case(picked))
            });
        if let Some(folder) = folder {
            slot.0 = Some(Pick {
                target: folder.target.clone(),
                why,
            });
        }
    }
    out
}

fn answer_json(text: &str) -> Option<Value> {
    let text = text.trim();
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end < start {
        return None;
    }
    serde_json::from_str(&text[start..=end]).ok()
}
