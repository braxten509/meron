//! Which of the reader's lessons are like a new email.
//!
//! A lesson is an email the reader moved by hand and where they put it (see
//! [`crate::store::ai_lessons`]). For each new email AI sorting picks the few
//! lessons most like it and shows them to the AI next to the email; the AI
//! decides whether the new email is the same kind. Nothing here decides a
//! move: this only chooses what the AI gets to see, and when "Spam" may be
//! among its answers at all.

use std::collections::BTreeSet;

use crate::store::LessonPlace;

/// Most lessons shown with one email.
pub const ALIKE_MAX: usize = 4;
/// How alike two subjects must be (shared words over all words) for a lesson
/// from the same company, but another address, to back a Spam answer.
pub const CLOSE_SUBJECT: f64 = 0.34;
/// And how many words they must share at least: one common word ("Invoice",
/// "Hello") never ties two addresses together.
pub const CLOSE_WORDS: usize = 2;

/// One lesson, as the choosing needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Lesson {
    pub from_name: String,
    pub from_addr: String,
    pub subject: String,
    pub place: LessonPlace,
    pub at: i64,
}

impl Lesson {
    pub fn sender(&self) -> String {
        match (self.from_name.trim(), self.from_addr.trim()) {
            ("", addr) => addr.to_string(),
            (name, "") => name.to_string(),
            (name, addr) => format!("{name} <{addr}>"),
        }
    }
}

/// One lesson as the AI sees it with an email: who sent the earlier email,
/// its subject, and the folder name the reader put it in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromptLesson {
    pub from: String,
    pub subject: String,
    pub moved_to: String,
}

/// The sender's site: the address's domain cut down to the part a company
/// registers (`news.shop.com` and `mail.shop.com` are both `shop.com`). A
/// rough cut without a suffix list: the last two labels, or three under a
/// country's common second level (`shop.co.uk`).
pub fn site_of(addr: &str) -> String {
    let domain = addr.trim().rsplit('@').next().unwrap_or_default().to_lowercase();
    if !addr.contains('@') || domain.is_empty() {
        return String::new();
    }
    let labels: Vec<&str> = domain.split('.').filter(|label| !label.is_empty()).collect();
    let keep = match labels.as_slice() {
        [.., second, top]
            if labels.len() >= 3
                && top.len() == 2
                && matches!(*second, "co" | "com" | "org" | "net" | "ac" | "gov" | "edu" | "or" | "ne") =>
        {
            3
        }
        _ => 2,
    };
    labels[labels.len().saturating_sub(keep)..].join(".")
}

/// The words of a subject that say what it is about: lowercase, three letters
/// or more, no bare numbers, no reply or forward marks.
fn subject_words(subject: &str) -> BTreeSet<String> {
    subject
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3)
        .filter(|word| !word.chars().all(|c| c.is_ascii_digit()))
        .filter(|word| !matches!(*word, "fwd" | "fyi"))
        .map(str::to_string)
        .collect()
}

/// How alike two subjects are, 0 to 1: the words they share over all their
/// words.
pub fn subject_likeness(a: &str, b: &str) -> f64 {
    let a = subject_words(a);
    let b = subject_words(b);
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let shared = a.intersection(&b).count() as f64;
    let all = a.union(&b).count() as f64;
    shared / all
}

/// How many telling words two subjects share.
fn shared_words(a: &str, b: &str) -> usize {
    subject_words(a).intersection(&subject_words(b)).count()
}

fn same_address(lesson: &Lesson, from_addr: &str) -> bool {
    let addr = from_addr.trim();
    !addr.is_empty() && lesson.from_addr.trim().eq_ignore_ascii_case(addr)
}

fn same_site(lesson: &Lesson, from_addr: &str) -> bool {
    let site = site_of(from_addr);
    !site.is_empty() && site_of(&lesson.from_addr) == site
}

/// How like the email a lesson is: 0 when it is from another site; from the
/// same site 1, from the same address 2, each plus how alike the subjects are.
fn likeness(lesson: &Lesson, from_addr: &str, subject: &str) -> f64 {
    let base = if same_address(lesson, from_addr) {
        2.0
    } else if same_site(lesson, from_addr) {
        1.0
    } else {
        return 0.0;
    };
    base + subject_likeness(&lesson.subject, subject)
}

/// Whether a lesson is close enough to back a Spam answer for the email: the
/// very same address, or the same site with an alike subject (at least
/// [`CLOSE_WORDS`] shared words, [`CLOSE_SUBJECT`] of all). A lesson from a
/// shared mail service (two strangers at one big provider) with another
/// subject, or with one common word alike, never is.
pub fn close(lesson: &Lesson, from_addr: &str, subject: &str) -> bool {
    same_address(lesson, from_addr)
        || (same_site(lesson, from_addr)
            && shared_words(&lesson.subject, subject) >= CLOSE_WORDS
            && subject_likeness(&lesson.subject, subject) >= CLOSE_SUBJECT)
}

/// The lessons most like an email from `from_addr` about `subject`: from the
/// same address first, then the same site, alike subjects first, at most
/// [`ALIKE_MAX`]. Answered newest first, the order the AI is told they are in.
///
/// Spam is the careful place, and the reader's latest word on such mail
/// decides whether it is open at all: a Spam lesson is only among them when
/// it is [`close`] and the newest close lesson, of any place, is a Spam one
/// (of two as new as each other, the one that isn't Spam counts). Once the
/// reader moved such mail back to the Inbox, their older Spam lessons for it
/// stay out until they send such mail to Spam again.
pub fn alike<'a>(lessons: &'a [Lesson], from_addr: &str, subject: &str) -> Vec<&'a Lesson> {
    let spam_open = lessons
        .iter()
        .filter(|lesson| close(lesson, from_addr, subject))
        .max_by_key(|lesson| (lesson.at, lesson.place != LessonPlace::Spam))
        .is_some_and(|newest| newest.place == LessonPlace::Spam);
    let mut scored: Vec<(f64, &Lesson)> = lessons
        .iter()
        .filter(|lesson| {
            lesson.place != LessonPlace::Spam || (spam_open && close(lesson, from_addr, subject))
        })
        .map(|lesson| (likeness(lesson, from_addr, subject), lesson))
        .filter(|(score, _)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.at.cmp(&a.1.at)));
    scored.truncate(ALIKE_MAX);
    scored.sort_by(|a, b| b.1.at.cmp(&a.1.at));
    scored.into_iter().map(|(_, lesson)| lesson).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lesson(addr: &str, subject: &str, place: LessonPlace, at: i64) -> Lesson {
        Lesson {
            from_name: String::new(),
            from_addr: addr.to_string(),
            subject: subject.to_string(),
            place,
            at,
        }
    }

    #[test]
    fn only_alike_lessons_are_shown_and_spam_needs_a_close_one() {
        let lessons = vec![
            lesson("promo@shop.com", "Big weekend sale: 50% off", LessonPlace::Spam, 10),
            lesson("orders@shop.com", "Your receipt for order 1234", LessonPlace::Inbox, 20),
            lesson("stranger@bigmail.com", "You won a prize", LessonPlace::Spam, 30),
            lesson("news@other.org", "Monthly news", LessonPlace::Meron("mf_a".into()), 40),
        ];
        let subjects = |found: Vec<&Lesson>| -> Vec<String> {
            found.iter().map(|lesson| lesson.subject.clone()).collect()
        };

        // The same address: its Spam lesson shows, and so does the same
        // company's Inbox lesson, newest first.
        assert_eq!(
            subjects(alike(&lessons, "promo@shop.com", "Flash sale today")),
            ["Your receipt for order 1234", "Big weekend sale: 50% off"]
        );
        // Another address of the same company with an alike subject: close.
        assert!(close(&lessons[0], "deals@mail.shop.com", "Weekend sale: 30% off"));
        // The same company with another kind of subject: the Spam lesson is
        // not shown at all, the Inbox one is.
        assert_eq!(
            subjects(alike(&lessons, "orders@shop.com", "Your receipt for order 9876")),
            ["Your receipt for order 1234"]
        );
        // The reader later moved such an advert back to the Inbox: their
        // latest word counts, and the older Spam lesson stays out.
        let mut taken_back = lessons.clone();
        taken_back.push(lesson("promo@shop.com", "Big spring sale: 50% off", LessonPlace::Inbox, 50));
        assert!(
            alike(&taken_back, "promo@shop.com", "Flash sale today")
                .iter()
                .all(|lesson| lesson.place != LessonPlace::Spam)
        );
        // As new as the Spam lesson: the one that isn't Spam still counts.
        let mut tied = lessons.clone();
        tied.push(lesson("promo@shop.com", "Big spring sale: 50% off", LessonPlace::Inbox, 10));
        assert!(
            alike(&tied, "promo@shop.com", "Flash sale today")
                .iter()
                .all(|lesson| lesson.place != LessonPlace::Spam)
        );
        // A friend at the same big mail service as a spammer: nothing, also
        // when one common word of the subjects is the same.
        assert!(alike(&lessons, "friend@bigmail.com", "Lunch on Friday?").is_empty());
        assert!(!close(&lessons[2], "friend@bigmail.com", "Prize question"));
        // Another site entirely: nothing, however alike the subject.
        assert!(alike(&lessons, "promo@elsewhere.com", "Big weekend sale: 50% off").is_empty());
    }

    #[test]
    fn a_site_is_the_part_a_company_registers() {
        assert_eq!(site_of("a@news.shop.com"), "shop.com");
        assert_eq!(site_of("A@Shop.Com"), "shop.com");
        assert_eq!(site_of("a@mail.shop.co.uk"), "shop.co.uk");
        assert_eq!(site_of("a@mail.gmx.de"), "gmx.de");
        assert_eq!(site_of("no-address"), "");
    }
}
