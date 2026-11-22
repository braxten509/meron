//! Fetches a subscription and parses its entries for synchronization.
//! Conditional HTTP requests allow a server to report an unchanged feed.
//! HTML discovery handles pages that point to a separate RSS or Atom document.

use super::content::{clean_feed_text, extract_image_urls, extract_video_urls, text_content};
use super::{HTTP_TIMEOUT, ParsedItem, stable_hash};
use crate::store::RssItemExtra;
use crate::store::RssMedia;
use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;

pub(super) struct FetchedFeed {
    pub(super) feed: Option<feed_rs::model::Feed>,
    pub(super) resolved_url: String,
    pub(super) etag: String,
    pub(super) last_modified: String,
    pub(super) not_modified: bool,
}

pub(super) struct HttpResponse {
    pub(super) status: u16,
    etag: String,
    last_modified: String,
    pub(super) body: Vec<u8>,
    content_type: String,
}

pub(super) fn http_get(url: &str, etag: &str, last_modified: &str) -> Result<HttpResponse> {
    let mut request = crate::proxy::agent()?.get(url);
    if !etag.is_empty() {
        request = request.header("If-None-Match", etag);
    }
    if !last_modified.is_empty() {
        request = request.header("If-Modified-Since", last_modified);
    }
    let mut response = request
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .call()
        .with_context(|| format!("HTTP GET {url}"))?;
    let status = response.status().as_u16();
    let etag = header_text(&response, "etag");
    let last_modified = header_text(&response, "last-modified");
    let content_type = header_text(&response, "content-type");
    let body = if status == 304 {
        Vec::new()
    } else {
        response
            .body_mut()
            .read_to_vec()
            .with_context(|| format!("read body {url}"))?
    };
    Ok(HttpResponse {
        status,
        etag,
        last_modified,
        body,
        content_type,
    })
}

fn header_text(response: &ureq::http::Response<ureq::Body>, name: &str) -> String {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

/// Fetch and parse a feed. On a parse failure, try discovering a feed `<link>`
/// in the page's HTML and parse that instead.
pub(super) fn fetch_feed(raw_url: &str, etag: &str, last_modified: &str) -> Result<FetchedFeed> {
    let response = http_get(raw_url, etag, last_modified)?;
    if response.status == 304 {
        return Ok(FetchedFeed {
            feed: None,
            resolved_url: raw_url.to_string(),
            etag: etag.to_string(),
            last_modified: last_modified.to_string(),
            not_modified: true,
        });
    }
    if let Ok(feed) = feed_rs::parser::parse(&response.body[..]) {
        return Ok(FetchedFeed {
            feed: Some(feed),
            resolved_url: raw_url.to_string(),
            etag: response.etag,
            last_modified: response.last_modified,
            not_modified: false,
        });
    }
    // Direct parse failed: treat the body as HTML and look for a feed link.
    let html = String::from_utf8_lossy(&response.body);
    let discovered = discover_feed_url(raw_url, &html)
        .ok_or_else(|| anyhow!("no RSS feed found at {raw_url}"))?;
    let feed_response = http_get(&discovered, "", "")?;
    let feed = feed_rs::parser::parse(&feed_response.body[..])
        .with_context(|| format!("parse discovered feed {discovered}"))?;
    let _ = response.content_type;
    Ok(FetchedFeed {
        feed: Some(feed),
        resolved_url: discovered,
        etag: feed_response.etag,
        last_modified: feed_response.last_modified,
        not_modified: false,
    })
}

/// Find a `<link rel="alternate" type="application/rss+xml" ...>` (or similar)
/// in an HTML head and resolve it against the page URL.
pub(super) fn discover_feed_url(page_url: &str, html: &str) -> Option<String> {
    let doc = scraper::Html::parse_document(html);
    let selector = scraper::Selector::parse("link").ok()?;
    let base = url::Url::parse(page_url).ok();
    for element in doc.select(&selector) {
        let rel = element.value().attr("rel").unwrap_or_default();
        let media_type = element.value().attr("type").unwrap_or_default();
        let href = element.value().attr("href").unwrap_or_default().trim();
        if href.is_empty() || !rel_has_alternate(rel) || !is_feed_link_type(media_type) {
            continue;
        }
        return match &base {
            Some(b) => b
                .join(href)
                .ok()
                .map(|resolved_url| resolved_url.to_string()),
            None => Some(href.to_string()),
        };
    }
    None
}

pub(super) fn parse_items(feed: &feed_rs::model::Feed, fetched_at: i64) -> Vec<ParsedItem> {
    feed.entries
        .iter()
        .filter_map(|entry| parse_item(entry, fetched_at))
        .collect()
}

pub(super) fn parse_item(entry: &feed_rs::model::Entry, fetched_at: i64) -> Option<ParsedItem> {
    let mut title = text_content(&entry.title).trim().to_string();
    if title.is_empty() {
        title = "(untitled)".to_string();
    }
    let author = entry
        .authors
        .first()
        .map(|person| person.name.trim().to_string())
        .unwrap_or_default();
    let summary_html = text_content(&entry.summary);
    let content_html = entry
        .content
        .as_ref()
        .and_then(|c| c.body.clone())
        .unwrap_or_default();
    let summary = clean_feed_text(&summary_html);
    let content = clean_feed_text(&content_html);
    // Lift inline <img> URLs out of the HTML (stripped by clean_feed_text) so the
    // UI can show them as images. Union content + summary, content first. Keys are
    // filled in later by the image cacher (sync time) for image-enabled accounts.
    let mut image_urls = extract_image_urls(&content_html);
    for url in extract_image_urls(&summary_html) {
        if !image_urls.iter().any(|existing| existing == &url) {
            image_urls.push(url);
        }
    }
    let images = image_urls
        .into_iter()
        .map(|url| RssMedia { url, key: None })
        .collect();
    // Lift inline <video>/<source> URLs out of the HTML the same way. These are
    // rendered straight from their remote URL (no local caching), so they only
    // ever carry a `url`.
    let mut video_urls = extract_video_urls(&content_html);
    for url in extract_video_urls(&summary_html) {
        if !video_urls.iter().any(|existing| existing == &url) {
            video_urls.push(url);
        }
    }
    let videos = video_urls
        .into_iter()
        .map(|url| RssMedia { url, key: None })
        .collect();
    let link = entry
        .links
        .first()
        .map(|link| link.href.trim().to_string())
        .unwrap_or_default();
    let published_at = entry.published.map(|date| date.timestamp()).unwrap_or(0);
    let updated_at = entry.updated.map(|date| date.timestamp()).unwrap_or(0);

    // Keep the raw HTML (content preferred, summary as fallback) for HTML mode.
    let content_html = if !content_html.trim().is_empty() {
        Some(content_html)
    } else if !summary_html.trim().is_empty() {
        Some(summary_html)
    } else {
        None
    };

    let key_source = if !entry.id.trim().is_empty() {
        entry.id.trim().to_string()
    } else if !link.is_empty() {
        link.clone()
    } else {
        format!("{title}|{published_at}|{updated_at}")
    };

    Some(ParsedItem {
        item_key: stable_hash(&key_source),
        title,
        unread: true,
        content_html,
        extra: RssItemExtra {
            author,
            link,
            summary,
            content,
            images,
            videos,
            published_at,
            updated_at,
            fetched_at,
        },
    })
}

fn rel_has_alternate(rel: &str) -> bool {
    rel.to_lowercase()
        .split_whitespace()
        .any(|token| token == "alternate")
}

fn is_feed_link_type(value: &str) -> bool {
    matches!(
        value.trim().to_lowercase().as_str(),
        "application/rss+xml"
            | "application/atom+xml"
            | "application/feed+json"
            | "application/json"
            | "application/xml"
            | "text/xml"
    )
}
