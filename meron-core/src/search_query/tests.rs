//! Regression tests for search query.
//!
//! Tests access the parent module directly to exercise its internal behavior.
//! They run as part of the shared core test suite.

use super::*;

#[test]
fn renaming_a_tag_changes_only_terms_that_search_it() {
    let rename = |query| rename_tag_terms(query, "old", "new");
    assert_eq!(rename("tag:old"), "tag:new");
    assert_eq!(
        rename("from:a  TAG:Old tag:\"old\" tag:#old"),
        "from:a  tag:new tag:new tag:new"
    );
    assert_eq!(rename("tag:old AND is:unread"), "tag:new AND is:unread");
    for untouched in [
        "tag:older",
        "-tag:old",
        "subject:\"Discuss tag:old tomorrow\"",
        "from:(tag:old OR b)",
        "\"tag:old\"",
        "tag:",
    ] {
        assert_eq!(rename(untouched), untouched);
    }
}

#[test]
fn plain_query_is_unchanged_free_text() {
    let query = SearchQuery::parse("  quarterly  report ");
    assert_eq!(query.text, ["quarterly  report"]);
    assert!(!query.has_operators());
}

#[test]
fn operators_are_taken_out_of_the_text() {
    let query = SearchQuery::parse("from:a@b.c to:d@e.f budget is:unread");
    assert_eq!(query.from, [["a@b.c"]]);
    assert_eq!(query.to, [["d@e.f"]]);
    assert_eq!(query.text, ["budget"]);
    assert!(query.unread && !query.read);
}

#[test]
fn quoted_values_keep_spaces() {
    let query = SearchQuery::parse(r#"From:"Ann Lee" subject:"Q3 plan" notes"#);
    assert_eq!(query.from, [["Ann Lee"]]);
    assert_eq!(query.subject, [["Q3 plan"]]);
    assert_eq!(query.text, ["notes"]);
}

#[test]
fn unknown_or_empty_operators_stay_text() {
    let query = SearchQuery::parse("re:foo https://x.y from: is:muted after:soon");
    assert!(!query.has_operators());
    assert_eq!(query.text, ["re:foo https://x.y from: is:muted after:soon"]);
}

#[test]
fn alternatives_in_gmail_syntax() {
    let any = |query: &str| SearchQuery::parse(query).from;
    assert_eq!(any("from:{a@b.c x@y.z}"), [["a@b.c", "x@y.z"]]);
    assert_eq!(any(r#"from:{"Ann Lee", bob}"#), [["Ann Lee", "bob"]]);
    assert_eq!(any("from:(a OR b | c)"), [["a", "b", "c"]]);
    // Parentheses without separators hold one value.
    assert_eq!(any("from:(Ann Lee)"), [["Ann Lee"]]);
    assert_eq!(any("from:a,b"), [["a", "b"]]);
    assert_eq!(any("from:a OR from:b OR from:c"), [["a", "b", "c"]]);
    assert_eq!(any(r#"from:"Ann Lee",bob"#), [["Ann Lee", "bob"]]);
    assert_eq!(any(r#"from:"Ann Lee"#), [["Ann Lee"]]);
    // A quoted comma is part of the name.
    assert_eq!(any(r#"from:"Lee, Ann""#), [["Lee, Ann"]]);
    // Each term is still ANDed with the others.
    assert_eq!(any("from:{a b} from:c"), [vec!["a", "b"], vec!["c"]]);

    // `OR` that can't merge stays a word: lowercase, across keys, or
    // between words.
    let q = SearchQuery::parse("from:a or from:b");
    assert_eq!(
        (q.from.len(), q.text.as_slice()),
        (2, &["or".to_string()][..])
    );
    let q = SearchQuery::parse("from:a OR to:b");
    assert_eq!(
        (q.from.len(), q.to.len(), q.text.as_slice()),
        (1, 1, &["OR".to_string()][..])
    );
    assert_eq!(SearchQuery::parse("cats OR dogs").text, ["cats OR dogs"]);
    // A group opens only right after a key's colon; an empty one is text.
    assert_eq!(SearchQuery::parse("(a b) from:{}").text, ["(a b) from:{}"]);
    assert_eq!(any("subject:x from:{a b"), [["a", "b"]]);
}

#[test]
fn fields_list_alternatives_comma_separated() {
    let fields = SearchFields::parse("from:a OR from:b to:{c \"D E\"} plan");
    assert_eq!(fields.from, "a, b");
    assert_eq!(fields.to, "c, D E");
    assert_eq!(fields.words, "plan");
    assert_eq!(fields.to_query(), r#"from:{a b} to:{c "D E"} plan"#);
    assert_eq!(SearchFields::parse(&fields.to_query()), fields);
    // Submitting the form unchanged keeps a value it would otherwise split.
    for query in [
        r#"subject:"Q3  plan""#,
        r#"from:"Lee, Ann""#,
        r#"subject:"A|B""#,
        r#"subject:"OR""#,
        r#"subject:"this OR that""#,
    ] {
        let fields = SearchFields::parse(query);
        assert_eq!(
            SearchQuery::parse(&fields.to_query()),
            SearchQuery::parse(query),
            "{query}"
        );
    }
    assert_eq!(
        SearchFields::parse(r#"from:"Lee, Ann" OR from:bob"#).from,
        r#""Lee, Ann", bob"#
    );
    // A lone `OR` is a value, not a separator.
    for (field, query) in [
        ("OR", "subject:\"OR\""),
        ("a OR", "subject:\"a OR\""),
        ("OR, b", "subject:{\"OR\" b}"),
    ] {
        let fields = SearchFields {
            subject: field.into(),
            ..Default::default()
        };
        assert_eq!(fields.to_query(), query, "{field}");
    }
    assert_eq!(SearchQuery::parse("from:{OR b}").from, [["OR", "b"]]);
    // A picked suggestion leaves a trailing comma.
    let fields = SearchFields {
        from: "a@b.c, ".into(),
        ..Default::default()
    };
    assert_eq!(fields.to_query(), "from:a@b.c");
}

#[test]
fn flags_and_dates() {
    let query = SearchQuery::parse("tag:Work tag:\"to do\" tag:a:b");
    assert_eq!(query.tags, ["work", "to-do"]);
    assert_eq!(query.text, ["tag:a:b"]);

    let query = SearchQuery::parse("is:starred is:read after:2026/01/02 before:2026-02-03");
    assert!(query.starred);
    assert!(query.read && !query.unread);
    assert_eq!(query.after, NaiveDate::from_ymd_opt(2026, 1, 2));
    assert_eq!(query.before, NaiveDate::from_ymd_opt(2026, 2, 3));
    assert!(query.text.is_empty());
    assert!(!query.is_empty());
}

#[test]
fn has_attachment_is_an_operator() {
    let query = SearchQuery::parse("HAS:Attachment budget");
    assert!(query.has_attachment);
    assert_eq!(query.text, ["budget"]);
    assert!(SearchQuery::parse("has:attachment").has_operators());
    assert!(SearchQuery::parse("x AND has:attachment").has_attachment);
    // Gmail's other `has:` values stay text, as any unknown operator.
    let query = SearchQuery::parse("has:drive");
    assert!(!query.has_operators());
    assert_eq!(query.text, ["has:drive"]);
}

#[test]
fn and_joins_queries_that_must_all_match() {
    let q = SearchQuery::parse("invoice  due AND paid");
    assert_eq!(q.text, ["invoice  due", "paid"]);
    let q = SearchQuery::parse("from:boss budget AND from:ann notes");
    assert_eq!(q.from, [["boss"], ["ann"]]);
    assert_eq!(q.text, ["budget", "notes"]);
    // Each side's dates and read state narrow the other's.
    let q = SearchQuery::parse(
        "after:2026/10/01 before:2026/12/01 AND after:2026/01/01 before:2026/11/01",
    );
    assert_eq!(q.after, NaiveDate::from_ymd_opt(2026, 10, 1));
    assert_eq!(q.before, NaiveDate::from_ymd_opt(2026, 11, 1));
    let q = SearchQuery::parse("is:unread AND is:read");
    assert!(q.read && q.unread);
    // Within a side the last read state still wins.
    let q = SearchQuery::parse("is:unread is:read AND x");
    assert!(q.read && !q.unread);
    // Lowercase or quoted `and` is a word; an empty side is nothing.
    assert_eq!(
        SearchQuery::parse("salt and pepper").text,
        ["salt and pepper"]
    );
    assert_eq!(
        SearchQuery::parse(r#"subject:"Q AND A""#).subject,
        [["Q AND A"]]
    );
    assert_eq!(SearchQuery::parse("AND budget AND").text, ["budget"]);
    // An unclosed group or quote ends at the `AND`, not swallowing the rest.
    let q = SearchQuery::parse("from:{boss ann AND budget");
    assert_eq!(q.from, [["boss", "ann"]]);
    assert_eq!(q.text, ["budget"]);
    let q = SearchQuery::parse(r#"subject:"Q3 plan AND budget"#);
    assert_eq!(q.subject, [["Q3 plan"]]);
    assert_eq!(q.text, ["budget"]);
    // A closed one keeps its `AND`.
    // A group never takes in a bare `AND`, closed or not.
    let q = SearchQuery::parse("from:{boss ann AND from:{c d}");
    assert_eq!(q.from, [vec!["boss", "ann"], vec!["c", "d"]]);
    // Closing what a saved query leaves open, as narrowing it does, reads
    // the same, so the narrowed search can be flattened onto one line.
    assert_eq!(
        SearchQuery::parse(r#"x subject:"Q3"#),
        SearchQuery::parse(r#"x subject:"Q3""#)
    );
    assert_eq!(
        SearchQuery::parse("x from:{a b"),
        SearchQuery::parse("x from:{a b}")
    );
    assert_eq!(
        SearchQuery::parse("subject:\"Q3\"\nAND subject:\"x y\""),
        SearchQuery::parse(r#"subject:"Q3" AND subject:"x y""#)
    );
    // A quote may hold one, but doesn't run past a line, where a line left
    // open reads as it would alone.
    let q = SearchQuery::parse("subject:\"Q3 plan\nAND subject:\"x\"");
    assert_eq!(q.subject, [["Q3 plan"], ["x"]]);
    let alone = SearchQuery::parse(r#"subject:"Q3 AND budget"#);
    let mut narrowed = SearchQuery::parse("subject:\"Q3 AND budget\nAND paid");
    assert_eq!(narrowed.text.pop().as_deref(), Some("paid"));
    assert_eq!(narrowed, alone);
    let q = SearchQuery::parse("27\" monitor\nAND 24\" monitor");
    assert_eq!(q.text, ["27\" monitor", "24\" monitor"]);
}

#[test]
fn fields_hold_the_first_side_of_an_and() {
    let fields = SearchFields::parse("from:boss is:read AND from:ann is:unread budget");
    assert_eq!(fields.from, "boss");
    assert!(!fields.unread);
    assert_eq!(fields.words, "is:read AND from:ann is:unread budget");
    let ticked = SearchFields {
        unread: true,
        ..SearchFields::parse("from:boss AND is:read")
    };
    assert_eq!(ticked.to_query(), "from:boss is:unread AND is:read");
    let q = SearchQuery::parse(&ticked.to_query());
    assert!(q.read && q.unread);
    for query in [
        "from:boss is:read AND from:ann is:unread budget",
        "a  b AND c",
    ] {
        let fields = SearchFields::parse(query);
        assert_eq!(
            SearchQuery::parse(&fields.to_query()),
            SearchQuery::parse(query),
            "{query}"
        );
    }
}

#[test]
fn fields_round_trip_through_query_text() {
    let fields = SearchFields::parse(
        r#"budget tag:work from:"Ann Lee" tag:to-do to:d@e.f is:unread is:starred has:attachment after:2026/01/02 before:2026-02-03"#,
    );
    assert_eq!(
        fields,
        SearchFields {
            from: "Ann Lee".into(),
            to: "d@e.f".into(),
            subject: String::new(),
            tags: "work to-do".into(),
            words: "budget".into(),
            after: "2026-01-02".into(),
            before: "2026-02-03".into(),
            unread: true,
            starred: true,
            has_attachment: true,
        }
    );
    assert_eq!(
        fields.to_query(),
        r#"from:"Ann Lee" to:d@e.f tag:work tag:to-do is:starred has:attachment after:2026/01/02 before:2026/02/03 budget is:unread"#
    );
    assert_eq!(SearchFields::parse(&fields.to_query()), fields);
}

#[test]
fn the_tag_field_takes_commas_and_builds_a_query_saved_as_a_tag_column() {
    let fields = SearchFields {
        tags: "work, to-do".into(),
        ..Default::default()
    };
    assert_eq!(fields.to_query(), "tag:work tag:to-do");
    assert_eq!(SearchFields::parse("tag:work").to_query(), "tag:work");
}

#[test]
fn fields_keep_terms_they_cannot_hold_in_words() {
    let fields = SearchFields::parse("from:a@b.c from:x@y.z is:read re:foo");
    assert_eq!(fields.from, "a@b.c");
    assert_eq!(fields.words, "from:x@y.z is:read re:foo");
    assert_eq!(fields.to_query(), "from:a@b.c from:x@y.z is:read re:foo");
    // A plain query is all words, spacing intact.
    assert_eq!(SearchFields::parse(" a  b ").words, "a  b");
}

#[test]
fn fields_keep_the_last_read_state() {
    let unread = SearchFields::parse("is:read budget is:unread");
    assert!(unread.unread);
    assert_eq!(unread.words, "budget");
    assert!(SearchQuery::parse(&unread.to_query()).unread);

    let read = SearchFields::parse("is:unread is:read budget is:read");
    assert!(!read.unread);
    assert_eq!(read.words, "is:read budget is:read");
    assert!(SearchQuery::parse(&read.to_query()).read);

    // Ticking Unread on a read search wins over the kept `is:read`.
    let ticked = SearchFields {
        unread: true,
        ..SearchFields::parse("is:read budget")
    };
    assert!(!SearchQuery::parse(&ticked.to_query()).read);
}

#[test]
fn fields_drop_unusable_values() {
    let fields = SearchFields {
        from: r#" "x" "#.into(),
        subject: "  ".into(),
        after: "soon".into(),
        ..Default::default()
    };
    assert_eq!(fields.to_query(), "from:x");
}
