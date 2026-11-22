//! Interprets the visible content and attachments in RSS and Atom entries.
//! Feed parsing uses these helpers to clean text and discover inline media.
//! The mailbox also uses them to produce previews and attachment descriptions.

pub(super) fn text_content(text: &Option<feed_rs::model::Text>) -> String {
    text.as_ref().map(|t| t.content.clone()).unwrap_or_default()
}

pub(super) fn clean_feed_text(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    // Replace block-level HTML tags with newlines to preserve structural breaks.
    let normalized = value
        .replace("<p>", "\n")
        .replace("</p>", "\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</div>", "\n")
        .replace("</li>", "\n")
        .replace("</td>", "\n")
        .replace("</tr>", "\n")
        .replace("</h1>", "\n")
        .replace("</h2>", "\n")
        .replace("</h3>", "\n")
        .replace("</h4>", "\n")
        .replace("</h5>", "\n")
        .replace("</h6>", "\n")
        .replace("<blockquote>", "\n")
        .replace("</blockquote>", "\n");

    let fragment = scraper::Html::parse_fragment(&normalized);
    // Concatenate text nodes without separators (like goquery's .Text()) so we
    // don't insert spaces before punctuation; normalize spacing and newlines after.
    let text: String = fragment.root_element().text().collect();
    normalize_newlines_and_spaces(&text)
}

fn normalize_newlines_and_spaces(text: &str) -> String {
    let mut result = String::new();
    let mut consecutive_newlines = 0;

    for line in text.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            consecutive_newlines += 1;
        } else {
            if !result.is_empty() {
                if consecutive_newlines > 1 {
                    result.push_str("\n\n");
                } else {
                    result.push('\n');
                }
            }
            consecutive_newlines = 0;

            // Collapse multiple spaces within the line
            let collapsed_line = trimmed_line
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            result.push_str(&collapsed_line);
        }
    }
    result.trim().to_string()
}

/// Collect `<img src>` URLs from a feed item's HTML, in document order, deduped.
/// Skips `data:` URIs (inline base64 would bloat the cache) and empty srcs.
pub(super) fn extract_image_urls(html: &str) -> Vec<String> {
    let html = html.trim();
    if html.is_empty() {
        return Vec::new();
    }
    let fragment = scraper::Html::parse_fragment(html);
    let Ok(selector) = scraper::Selector::parse("img") else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for element in fragment.select(&selector) {
        let Some(source_url) = element.value().attr("src") else {
            continue;
        };
        let source_url = source_url.trim();
        if source_url.is_empty() || source_url.starts_with("data:") {
            continue;
        }

        let mut is_tracker = false;

        // 1. Check width and height attributes
        if let (Some(w), Some(h)) = (
            element.value().attr("width"),
            element.value().attr("height"),
        ) {
            let w = w.trim();
            let h = h.trim();
            if (w == "1" || w == "0" || w == "2") && (h == "1" || h == "0" || h == "2") {
                is_tracker = true;
            }
        }

        // 2. Check inline style attributes
        if let Some(style) = element.value().attr("style") {
            let lowercase_style = style.to_lowercase();
            if lowercase_style.contains("display:none")
                || lowercase_style.contains("display: none")
                || lowercase_style.contains("visibility:hidden")
                || lowercase_style.contains("visibility: hidden")
            {
                is_tracker = true;
            }
            let has_tiny_w = lowercase_style.contains("width:0px")
                || lowercase_style.contains("width: 0px")
                || lowercase_style.contains("width:1px")
                || lowercase_style.contains("width: 1px")
                || lowercase_style.contains("width:2px")
                || lowercase_style.contains("width: 2px");
            let has_tiny_h = lowercase_style.contains("height:0px")
                || lowercase_style.contains("height: 0px")
                || lowercase_style.contains("height:1px")
                || lowercase_style.contains("height: 1px")
                || lowercase_style.contains("height:2px")
                || lowercase_style.contains("height: 2px");
            if has_tiny_w && has_tiny_h {
                is_tracker = true;
            }
        }

        // 3. Check URL pattern heuristics
        let lowercase_url = source_url.to_lowercase();
        if lowercase_url.contains("/open/")
            || lowercase_url.contains("/track")
            || lowercase_url.contains("/pixel")
            || lowercase_url.contains("pixel.gif")
            || lowercase_url.contains("cleardot.gif")
            || lowercase_url.contains("spacer.gif")
            || lowercase_url.contains("/wf/open")
            || lowercase_url.contains("/open.php")
            || lowercase_url.contains("utm_")
            || lowercase_url.contains("bounce")
        {
            is_tracker = true;
        }

        if is_tracker {
            continue;
        }

        if !out.iter().any(|existing| existing == source_url) {
            out.push(source_url.to_string());
        }
    }
    out
}

/// Collect inline video URLs from a feed item's HTML, in document order, deduped.
/// Handles both `<video src>` and `<video><source src></video>`. Skips `data:`
/// URIs and empty srcs.
pub(super) fn extract_video_urls(html: &str) -> Vec<String> {
    let html = html.trim();
    if html.is_empty() {
        return Vec::new();
    }
    let fragment = scraper::Html::parse_fragment(html);
    let Ok(selector) = scraper::Selector::parse("video, video source") else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for element in fragment.select(&selector) {
        let Some(source_url) = element.value().attr("src") else {
            continue;
        };
        let source_url = source_url.trim();
        if source_url.is_empty() || source_url.starts_with("data:") {
            continue;
        }
        if !out.iter().any(|existing| existing == source_url) {
            out.push(source_url.to_string());
        }
    }
    out
}

/// The trailing path segment of an image URL, used as its display filename.
pub(super) fn image_filename(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed_url| {
            parsed_url
                .path_segments()
                .and_then(|mut segs| segs.next_back().map(str::to_string))
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "image".to_string())
}

/// Best-effort MIME guess from an image URL's extension (display only).
pub(super) fn image_mime(url: &str) -> &'static str {
    let lower = url.split(['?', '#']).next().unwrap_or(url).to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "image/jpeg"
    }
}

/// Best-effort MIME guess from a video URL's extension (display only); defaults
/// to `video/mp4` when the URL has no recognizable extension.
pub(super) fn video_mime(url: &str) -> &'static str {
    let lower = url.split(['?', '#']).next().unwrap_or(url).to_lowercase();
    if lower.ends_with(".webm") {
        "video/webm"
    } else if lower.ends_with(".ogv") || lower.ends_with(".ogg") {
        "video/ogg"
    } else if lower.ends_with(".mov") {
        "video/quicktime"
    } else {
        "video/mp4"
    }
}

pub(super) fn first_line(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview: String = collapsed.chars().take(220).collect();
    if collapsed.chars().count() > 220 {
        preview.push_str("...");
    }
    preview
}
