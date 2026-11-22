//! Regression tests for quote.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;

#[test]
fn plain_folds_a_trailing_quote_with_its_attribution() {
    let body = "Sounds good, see you then.\n\nOn Mon, Sep 1, 2025 at 9:00 AM Jane <jane@example.com> wrote:\n> Lunch on Friday?\n>\n> Jane\n";
    let start = plain_quote_start(body).expect("quote");
    assert_eq!(
        &body[start..],
        "On Mon, Sep 1, 2025 at 9:00 AM Jane <jane@example.com> wrote:\n> Lunch on Friday?\n>\n> Jane\n"
    );
}

#[test]
fn plain_takes_a_wrapped_attribution_whole() {
    let body = "Yes.\n\nOn Mon, Sep 1, 2025 at 9:00 AM Jane Doe <\njane@example.com> wrote:\n\n> Lunch?\n";
    let start = plain_quote_start(body).expect("quote");
    assert!(body[start..].starts_with("On Mon"), "{:?}", &body[start..]);
}

#[test]
fn plain_folds_a_signature_under_the_quote_with_it() {
    let body = "Does Tuesday work?\n\nOn Tue, Sep 8, 2026 at 8:14 PM Ping <ping@example.com>\nwrote:\n\n> Both days are good.\n>\n\n\n-- \n\n*MS*\n";
    let start = plain_quote_start(body).expect("quote");
    assert_eq!(&body[..start], "Does Tuesday work?\n\n");
}

#[test]
fn plain_keeps_a_signature_above_the_quote_out_of_it() {
    let body = "Does Tuesday work?\n-- \nMS\n\nOn Mon, Ping wrote:\n> Both days are good.";
    let start = plain_quote_start(body).expect("quote");
    assert_eq!(&body[..start], "Does Tuesday work?\n-- \nMS\n\n");
    // A signature with no quote anywhere is nothing to fold.
    assert_eq!(plain_quote_start("Hi\n-- \nMS"), None);
}

#[test]
fn plain_offset_counts_utf16_units() {
    // "é" is one UTF-16 unit but two bytes; "😀" is two units and four bytes.
    let body = "é😀\n\n> quoted";
    assert_eq!(
        plain_quote_start(body),
        Some("é😀\n\n".encode_utf16().count())
    );
}

#[test]
fn plain_leaves_inline_replies_alone() {
    let body = "> Lunch on Friday?\nYes!\n> Where?\nThe usual place.";
    assert_eq!(plain_quote_start(body), None);
}

#[test]
fn plain_only_folds_the_trailing_quote_of_an_inline_reply() {
    let body = "> Lunch?\nYes!\n\n> older history\n> more";
    let start = plain_quote_start(body).expect("quote");
    assert_eq!(&body[start..], "> older history\n> more");
}

#[test]
fn plain_keeps_a_body_that_is_only_a_quote() {
    assert_eq!(plain_quote_start("On Mon Jane wrote:\n> hi"), None);
    assert_eq!(plain_quote_start("> hi\n> there"), None);
    assert_eq!(plain_quote_start("no quote here"), None);
}

#[test]
fn plain_takes_an_attribution_wrapped_after_the_address() {
    let body = "Yes.\n\nOn Mon, Sep 1, 2025 at 9:00 AM Jane Doe <jane@example.com>\nwrote:\n> Lunch?\n";
    let start = plain_quote_start(body).expect("quote");
    assert_eq!(&body[..start], "Yes.\n\n");
}

#[test]
fn plain_keeps_the_reply_above_a_short_attribution() {
    for attribution in [
        "Jane wrote:",
        "<jane@example.com> wrote:",
        "jane@example.com wrote:",
    ] {
        let body = format!("Hi Bob,\n\nPlease proceed.\n{attribution}\n> Is this approved?");
        let start = plain_quote_start(&body).expect("quote");
        assert_eq!(
            &body[..start],
            "Hi Bob,\n\nPlease proceed.\n",
            "{attribution}"
        );
    }
}

#[test]
fn plain_does_not_fold_the_line_before_an_unwrapped_attribution() {
    let body = "Thanks\nOn Mon, Sep 1, 2025 at 9:00 AM Jane <jane@example.com> wrote:\n> hi";
    let start = plain_quote_start(body).expect("quote");
    assert_eq!(&body[..start], "Thanks\n");
}

#[test]
fn plain_leaves_forwards_alone() {
    let body = "FYI\n\n-----Original Message-----:\n> the content";
    assert_eq!(plain_quote_start(body), None);
}

#[test]
fn html_marks_gmail_quote() {
    let html = r#"<div dir="ltr">Sounds good</div><br><div class="gmail_quote"><div class="gmail_attr">On Mon, Jane wrote:<br></div><blockquote class="gmail_quote">Lunch?</blockquote></div>"#;
    let out = mark_html_quote(html, false);
    assert!(
        out.contains(r#"<div class="gmail_quote" data-meron-quote="">"#),
        "{out}"
    );
    // Only the outermost container is marked, not the blockquote inside it.
    assert!(out.contains(r#"<blockquote class="gmail_quote">"#), "{out}");
    // The <br> setting the quote off from the reply folds with it.
    assert!(
        out.contains(r#"</div><br data-meron-quote=""><div"#),
        "{out}"
    );
    assert!(out.contains("Sounds good"));
}

#[test]
fn html_folds_a_gmail_signature_under_the_quote() {
    let html = r#"<div dir="ltr">Does Tuesday work?</div><br><div class="gmail_quote gmail_quote_container"><div class="gmail_attr">On Tue, Ping wrote:<br></div><blockquote class="gmail_quote"><p>Both days are good.</p></blockquote></div><div><br clear="all"></div><div><br></div><span class="gmail_signature_prefix">-- </span><br><div dir="ltr" class="gmail_signature"><b>MS</b></div>"#;
    let out = mark_html_quote(html, false);
    // The quote, the <br> above it, the two empty divs, the "-- " prefix,
    // the <br> after it and the signature: nothing blank is left behind.
    assert_eq!(out.matches(HTML_QUOTE_ATTR).count(), 7, "{out}");
    assert!(
        out.starts_with(r#"<div dir="ltr">Does Tuesday work?</div><br data-meron-quote="">"#),
        "{out}"
    );
    assert!(out.contains(r#"data-meron-quote=""><b>MS</b>"#), "{out}");
    // Content that isn't a signature after the quote still keeps it open.
    let after = r#"<p>Yes</p><div class="gmail_quote">Lunch?</div><p>P.S. bring cake</p>"#;
    assert_eq!(mark_html_quote(after, false), after);
    // A quote and a signature with no reply is nothing to fold.
    let bare = r#"<div class="gmail_quote">Lunch?</div><div class="gmail_signature">MS</div>"#;
    assert_eq!(mark_html_quote(bare, false), bare);
}

#[test]
fn html_marks_apple_and_thunderbird_attribution() {
    let html = r#"<p>Yes!</p><div class="moz-cite-prefix">On 9/1/25 Jane wrote:</div><blockquote type="cite"><p>Lunch?</p></blockquote>"#;
    let out = mark_html_quote(html, false);
    assert!(
        out.contains(r#"<div class="moz-cite-prefix" data-meron-quote="">"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<blockquote type="cite" data-meron-quote="">"#),
        "{out}"
    );
}

#[test]
fn html_leaves_inline_and_quote_only_bodies_alone() {
    let inline = r#"<blockquote type="cite">Lunch?</blockquote><p>Yes!</p>"#;
    assert_eq!(mark_html_quote(inline, false), inline);
    // Nor is a Gmail quote inside it folded instead, answers and all.
    let nested = r#"<div class="elementToProof">See below</div><div id="divRplyFwdMsg"><b>From:</b> Jane</div><div><div>Lunch?</div><div class="gmail_quote">On Mon, Joe wrote:<blockquote>Hungry?</blockquote><div class="elementToProof">Yes, at noon</div></div></div>"#;
    assert_eq!(mark_html_quote(nested, false), nested);
    let only = r#"<div class="gmail_quote">Lunch?</div>"#;
    assert_eq!(mark_html_quote(only, false), only);
    let plain = "<p>no quote</p>";
    assert_eq!(mark_html_quote(plain, false), plain);
}

#[test]
fn html_ignores_trailing_whitespace_and_styles() {
    let html = r#"<p>Yes!</p><div class="gmail_quote">Lunch?</div><div><br></div><style>p{color:red}</style>"#;
    assert!(mark_html_quote(html, false).contains(HTML_QUOTE_ATTR));
}

#[test]
fn html_marks_outlook_web_quote() {
    // Outlook on the web: no container, the quote is the header block's
    // siblings, and an older quote inside it repeats the ids prefixed.
    let html = r#"<div class="elementToProof">t2</div><div id="appendonsend"></div><hr style="display:inline-block;width:98%"><div id="divRplyFwdMsg" dir="ltr"><font><b>From:</b> Jane &lt;jane@example.com&gt;<br><b>Subject:</b> Fw: Lunch</font><div>&nbsp;</div></div><style>p{margin:0}</style><div dir="ltr"><div class="x_elementToProof">t1</div><div id="x_appendonsend"></div><hr><div id="x_divRplyFwdMsg"><b>From:</b> Joe</div><div>Lunch?</div></div>"#;
    let out = mark_html_quote(html, false);
    assert!(
        out.contains(r#"<div class="elementToProof">t2</div>"#),
        "{out}"
    );
    for folded in [
        r#"<div id="appendonsend" data-meron-quote="">"#,
        r#"<hr style="display:inline-block;width:98%" data-meron-quote="">"#,
        r#"<div dir="ltr" id="divRplyFwdMsg" data-meron-quote="">"#,
        r#"<div dir="ltr" data-meron-quote="">"#,
    ] {
        assert!(out.contains(folded), "{folded}: {out}");
    }
    // Only the outermost quote is marked.
    assert!(out.contains(r#"<div id="x_divRplyFwdMsg">"#), "{out}");
    // The markup is the same for a forward: only the subject tells.
    assert_eq!(mark_html_quote(html, true), html);
}

#[test]
fn html_marks_outlook_desktop_quote() {
    let html = r#"<div class="WordSection1"><p class="MsoNormal">Bonjour</p><p class="MsoNormal">&nbsp;</p><div><div style="border:none;border-top:solid #E1E1E1 1.0pt;padding:3.0pt 0cm 0cm 0cm"><p class="MsoNormal"><b>De :</b> Jane<br><b>Objet :</b> Devis</p></div></div><p class="MsoNormal">Le devis</p></div>"#;
    let out = mark_html_quote(html, false);
    assert!(out.contains(r#"<p class="MsoNormal">Bonjour</p>"#), "{out}");
    assert!(
        out.contains(r#"<p class="MsoNormal" data-meron-quote="">&nbsp;</p>"#),
        "{out}"
    );
    assert!(
        out.contains(r#"padding:3.0pt 0cm 0cm 0cm" data-meron-quote="">"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<p class="MsoNormal" data-meron-quote="">Le devis</p>"#),
        "{out}"
    );
}

#[test]
fn html_leaves_other_outlook_shapes_alone() {
    // Nothing above the header block: the quote is the whole message.
    let only = r#"<div id="divRplyFwdMsg"><b>From:</b> Jane</div><div>Lunch?</div>"#;
    assert_eq!(mark_html_quote(only, false), only);
    // A top border that rules off something other than mail headers.
    for note in [
        "Thanks, and see you there",
        "<b>Note:</b> payment is due Friday",
    ] {
        let ruled = format!(
            r#"<p>Hi</p><div style="border-top:solid #ccc 1.0pt;padding:3.0pt 0cm 0cm 0cm">{note}</div><p>Jane</p>"#
        );
        assert_eq!(mark_html_quote(&ruled, false), ruled);
    }
    // An answer typed into the quote, which Outlook on the web marks: the
    // quote is then part of the reply.
    let inline = r#"<div class="elementToProof">See below</div><div id="divRplyFwdMsg"><b>From:</b> Jane</div><div><div class="x_elementToProof">Lunch?</div><div class="elementToProof">Yes, at noon</div></div>"#;
    assert_eq!(mark_html_quote(inline, false), inline);
    // Bare text after the block cannot be marked, so nothing is.
    let bare = r#"<p>Yes</p><div id="divRplyFwdMsg"><b>From:</b> Jane</div>Lunch?"#;
    assert_eq!(mark_html_quote(bare, false), bare);
}

#[test]
fn html_folds_the_whole_outlook_quote_over_a_gmail_one_in_it() {
    let html = r#"<div>Yes</div><div id="divRplyFwdMsg"><b>From:</b> Jane</div><div><div>Lunch?</div><div class="gmail_quote">On Mon, Joe wrote:<blockquote>Hungry</blockquote></div></div>"#;
    let out = mark_html_quote(html, false);
    assert!(
        out.contains(r#"<div id="divRplyFwdMsg" data-meron-quote="">"#),
        "{out}"
    );
    assert!(out.contains(r#"<div class="gmail_quote">"#), "{out}");
    // Quoted by Gmail in turn, the Outlook block is inside the container.
    let nested = r#"<div>Yes</div><div class="gmail_quote">On Mon, Jane wrote:<blockquote><div>t1</div><div id="divRplyFwdMsg"><b>From:</b> Joe</div><div>Lunch?</div></blockquote></div>"#;
    let out = mark_html_quote(nested, false);
    assert!(
        out.contains(r#"<div class="gmail_quote" data-meron-quote="">"#),
        "{out}"
    );
    assert!(out.contains(r#"<div id="divRplyFwdMsg">"#), "{out}");
}

#[test]
fn forward_subjects() {
    for subject in [
        "Fw: Lunch",
        "FWD: Lunch",
        "TR : Devis",
        "WG: Angebot",
        "转发：午饭",
    ] {
        assert!(is_forward_subject(subject), "{subject}");
    }
    for subject in ["Re: Fw: Lunch", "Lunch", "Forwarding address: new", ""] {
        assert!(!is_forward_subject(subject), "{subject}");
    }
}

#[test]
fn html_leaves_forwards_alone() {
    let gmail = r#"<p>FYI</p><div class="gmail_quote"><div class="gmail_attr">---------- Forwarded message ---------<br>From: Jane</div>Content</div>"#;
    assert_eq!(mark_html_quote(gmail, false), gmail);
    let apple = r#"<p>FYI</p><div>Begin forwarded message:</div><blockquote type="cite">Content</blockquote>"#;
    assert_eq!(mark_html_quote(apple, false), apple);
}
