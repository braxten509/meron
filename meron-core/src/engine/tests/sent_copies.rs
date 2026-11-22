//! Checks whether the engine should append a sent copy and how it recognizes one.
//! Message IDs and envelope fields represent mail observed in the Sent folder.
//! Provider defaults and explicit preferences exercise the append policy.

use crate::engine::{SENT_COPY_MATCH_WINDOW_SECS, sent_copy_landed, should_append_sent_copy};
use crate::{imap, parse};

fn sent_envelope(message_id: &str) -> parse::SentEnvelope {
    parse::SentEnvelope {
        message_id: message_id.to_string(),
        subject: "Re: lunch".to_string(),
        from_addr: "me@example.com".to_string(),
        recipients: vec!["a@example.com".to_string(), "b@example.com".to_string()],
        date: 1_700_000_000,
    }
}

fn sent_candidate(message_id: &str) -> imap::MessageHeader {
    imap::MessageHeader {
        uid: 7,
        subject: "Re: lunch".to_string(),
        from_addr: "me@example.com".to_string(),
        date: 1_700_000_000,
        message_id: message_id.to_string(),
        to: vec![imap::Recipient {
            name: "B".to_string(),
            addr: "B@Example.com".to_string(),
        }],
        cc: vec![imap::Recipient {
            name: String::new(),
            addr: "a@example.com".to_string(),
        }],
        ..Default::default()
    }
}

#[test]
fn a_sent_copy_lands_by_id_or_by_its_envelope() {
    // The id we sent is the direct answer when it comes back.
    assert!(sent_copy_landed(
        &sent_envelope("reply-1@meron"),
        &sent_candidate("reply-1@meron")
    ));
    // Proton Bridge replaces it, so the envelope has to answer instead —
    // recipients compare as a set, case-insensitively, across To and Cc.
    assert!(sent_copy_landed(
        &sent_envelope("reply-1@meron"),
        &sent_candidate("abc@protonmail.internalid")
    ));

    // Another message the folder gained in the same seconds is not this
    // send's copy: a different subject, recipients or sender rules it out.
    let mut other = sent_candidate("other@meron");
    other.subject = "Dinner?".to_string();
    assert!(!sent_copy_landed(&sent_envelope("reply-1@meron"), &other));
    let mut other = sent_candidate("other@meron");
    other.cc.clear();
    assert!(!sent_copy_landed(&sent_envelope("reply-1@meron"), &other));
    let mut other = sent_candidate("other@meron");
    other.from_addr = "colleague@example.com".to_string();
    assert!(!sent_copy_landed(&sent_envelope("reply-1@meron"), &other));

    // An identical envelope from far enough back is an earlier send of the
    // same message, not this one; modest skew still matches.
    let mut earlier = sent_candidate("earlier@meron");
    earlier.date -= SENT_COPY_MATCH_WINDOW_SECS + 1;
    assert!(!sent_copy_landed(&sent_envelope("reply-1@meron"), &earlier));
    let mut skewed = sent_candidate("skewed@meron");
    skewed.date += SENT_COPY_MATCH_WINDOW_SECS - 1;
    assert!(sent_copy_landed(&sent_envelope("reply-1@meron"), &skewed));

    // Unreadable dates must not make everything match everything.
    let mut undated = sent_candidate("undated@meron");
    undated.date = 0;
    assert!(!sent_copy_landed(&sent_envelope("reply-1@meron"), &undated));
    let mut undated_send = sent_envelope("reply-1@meron");
    undated_send.date = 0;
    assert!(!sent_copy_landed(
        &undated_send,
        &sent_candidate("srv@meron")
    ));

    // A blank id on our side never matches a blank id on the server's.
    let mut blank = sent_envelope("");
    blank.subject = "Other".to_string();
    assert!(!sent_copy_landed(&blank, &sent_candidate("")));
}

#[test]
fn sent_copy_policy_uses_provider_defaults_and_overrides() {
    assert!(!should_append_sent_copy("gmail_oauth", "", None));
    assert!(!should_append_sent_copy("outlook_oauth", "", None));
    assert!(!should_append_sent_copy("password", "smtp.gmail.com", None));
    assert!(!should_append_sent_copy(
        "custom",
        "smtp.office365.com",
        None
    ));
    assert!(should_append_sent_copy(
        "password",
        "smtp.example.com",
        None
    ));
    assert!(should_append_sent_copy("custom", "", None));

    assert!(should_append_sent_copy("gmail_oauth", "", Some(true)));
    assert!(should_append_sent_copy(
        "password",
        "smtp.gmail.com",
        Some(true)
    ));
    assert!(!should_append_sent_copy(
        "password",
        "smtp.example.com",
        Some(false)
    ));
}
