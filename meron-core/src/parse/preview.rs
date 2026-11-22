//! Create short previews from formatted message text.
//!
//! Part of the shared message parser used by the IMAP and local mail paths.
//! The facade keeps its existing entry points for both platform clients.

/// Drop the markup that makes a body readable in the reader but noisy in a
/// one-line snippet: image markers and their links, link syntax (the text is
/// worth showing, the URL isn't), emphasis/code markers, and line-leading
/// quote/list/heading markers. Applies to both bodies we converted from HTML
/// and text/plain parts, which carry the same conventions (Gmail writes
/// `[image: alt] <url>`).
fn strip_preview_markup(body: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in body.lines() {
        let mut rest = line.trim();
        loop {
            let stripped = rest
                .strip_prefix("> ")
                .or_else(|| rest.strip_prefix('>'))
                .or_else(|| rest.strip_prefix("- "))
                .or_else(|| rest.strip_prefix("* "))
                .or_else(|| rest.strip_prefix("+ "))
                .or_else(|| {
                    rest.starts_with('#')
                        .then(|| rest.trim_start_matches('#').trim_start())
                });
            match stripped {
                Some(next) if next != rest => rest = next.trim_start(),
                _ => break,
            }
        }
        // Horizontal rules and other separator runs carry nothing in a snippet.
        if !rest.is_empty() && rest.chars().all(|c| matches!(c, '-' | '*' | '_' | '=')) {
            continue;
        }
        let stripped = strip_inline_markup(rest);
        if !stripped.trim().is_empty() {
            lines.push(stripped);
        }
    }
    lines.join(" ")
}

/// Inline half of [`strip_preview_markup`], run per line.
fn strip_inline_markup(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find(['[', '*', '`', '<']) {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        // A backslash-escaped delimiter is literal text, not markup.
        if is_escaped(&rest[..open]) {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
            continue;
        }
        match tail.as_bytes()[0] {
            b'[' => {
                let Some(close) = tail.find(']') else {
                    out.push('[');
                    rest = &tail[1..];
                    continue;
                };
                let label = &tail[1..close];
                let after = &tail[close + 1..];
                // `[text](url)` / `[text]<url>` keeps the text and drops the
                // destination; a bare `[text]` keeps the brackets (it may be a
                // literal like `[Attachment: x]`).
                let (after, linked) = match strip_link_target(after) {
                    Some(next) => (next, true),
                    None => (after, false),
                };
                if let Some(alt) = label.strip_prefix("image: ") {
                    // The marker stands in for a picture; in a snippet even the
                    // alt text is usually chrome ("Company Logo"), so drop it.
                    let _ = alt;
                } else if linked {
                    out.push_str(label);
                } else {
                    out.push('[');
                    out.push_str(label);
                    out.push(']');
                }
                rest = after;
            }
            b'<' => {
                // Autolink: keep the URL, drop the brackets. Anything else stays
                // (`<escaped>` text, `a < b`).
                match tail.find('>') {
                    Some(close) if is_autolink(&tail[1..close]) => {
                        out.push_str(&tail[1..close]);
                        rest = &tail[close + 1..];
                    }
                    _ => {
                        out.push('<');
                        rest = &tail[1..];
                    }
                }
            }
            // Emphasis (`*bold*`, `**x**`) and code spans: strip the delimiters
            // only when they actually pair up. Bodies here are often text/plain,
            // where a lone `*` is literal (`2 * 3`, `*.rs`) and must survive.
            marker => {
                let run = tail.bytes().take_while(|byte| *byte == marker).count();
                let inner = &tail[run..];
                match closing_run(inner, marker, run) {
                    Some(close) if marker == b'`' => {
                        // Code spans are literal: keep the content untouched.
                        out.push_str(&inner[..close]);
                        rest = &inner[close + run..];
                    }
                    Some(close) => {
                        out.push_str(&strip_inline_markup(&inner[..close]));
                        rest = &inner[close + run..];
                    }
                    None => {
                        out.push_str(&tail[..run]);
                        rest = inner;
                    }
                }
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether the delimiter right after `before` is backslash-escaped, i.e. an odd
/// number of backslashes precedes it (`\\*` is a literal backslash then markup).
fn is_escaped(before: &str) -> bool {
    before
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'\\')
        .count()
        % 2
        == 1
}

/// Byte offset of the delimiter run that closes an emphasis/code span opened by
/// `run` copies of `marker`, or `None` when the opener is unpaired. Follows the
/// CommonMark shape loosely: the span may not start or end on whitespace, and
/// the closing run must be the same length (so `**bold**` doesn't close on the
/// first `*`).
fn closing_run(inner: &str, marker: u8, run: usize) -> Option<usize> {
    let bytes = inner.as_bytes();
    if bytes.first().is_none_or(|byte| byte.is_ascii_whitespace()) {
        return None;
    }
    let mut idx = 1;
    while idx < bytes.len() {
        if bytes[idx] != marker {
            idx += 1;
            continue;
        }
        let len = bytes[idx..]
            .iter()
            .take_while(|byte| **byte == marker)
            .count();
        if len == run && !bytes[idx - 1].is_ascii_whitespace() && !is_escaped(&inner[..idx]) {
            return Some(idx);
        }
        idx += len;
    }
    None
}

/// Consume a link destination that follows a `]`, i.e. `(url)`, `<url>`, or
/// ` <url>`. Returns the remainder when one was there.
fn strip_link_target(after: &str) -> Option<&str> {
    if let Some(paren) = after.strip_prefix('(') {
        // Destinations may nest parens (`.../Foo_(bar)`) or escape them, so match
        // the closer by depth rather than taking the first `)`.
        let bytes = paren.as_bytes();
        let mut depth = 1usize;
        let mut idx = 0;
        while idx < bytes.len() {
            match bytes[idx] {
                b'\\' => idx += 1,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&paren[idx + 1..]);
                    }
                }
                _ => {}
            }
            idx += 1;
        }
        return None;
    }
    let angle = after
        .strip_prefix(" <")
        .or_else(|| after.strip_prefix('<'))?;
    let end = angle.find('>')?;
    is_autolink(&angle[..end]).then(|| &angle[end + 1..])
}

fn is_autolink(inner: &str) -> bool {
    !inner.contains(char::is_whitespace)
        && (inner.contains("://") || inner.starts_with("mailto:") || inner.contains('@'))
}

pub(crate) fn preview_of(body: &str) -> String {
    text_start(body, 200)
}

/// The start of a body as plain words: markup and links stripped, whitespace
/// squeezed, at most `max` characters (and an ellipsis when cut).
pub(crate) fn text_start(body: &str, max: usize) -> String {
    let stripped = strip_preview_markup(body);
    let collapsed = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview: String = collapsed.chars().take(max).collect();
    if collapsed.chars().count() > max {
        preview.push('…');
    }
    preview
}
