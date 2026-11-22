//! Merges per-account pages into the unified mail views.
//! The cursor records how much of each account's page has already been shown.
//! The merge preserves ordering while allowing each account to retain its own cursor format.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// Where one account's next unified page starts: its own page cursor (`None` for its first page),
/// how many rows that page was read with, and the last of its rows an earlier unified page showed.
/// Keeping the account's own cursor instead of minting one from the last row shown works for every
/// cursor format (cache, search, feeds).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountCursor {
    #[serde(default, rename = "c", skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    /// The limit the page was read with when rows of it were held back. It
    /// counts what the account's limit counts — message headers, which group
    /// into fewer threads — so only the same limit reads the same rows again.
    #[serde(default, rename = "n", skip_serializing_if = "is_zero")]
    pub depth: u32,
    #[serde(default, rename = "l", skip_serializing_if = "Option::is_none")]
    pub last: Option<ShownRow>,
}

/// The last row an earlier unified page showed from an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShownRow {
    #[serde(rename = "d")]
    pub date: i64,
    #[serde(rename = "i")]
    pub id: String,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl AccountCursor {
    /// The limit to ask the account for. A page with rows held back is read
    /// again at least as deep as before, or a smaller request (the next page
    /// after a deep reload) would cut held rows off — for good, for a feed
    /// account, which has no cursor to read on from.
    pub fn fetch_limit(&self, limit: u32) -> u32 {
        limit.max(self.depth)
    }

    /// Drop the rows of a page re-read from `cursor` that an earlier unified page already showed.
    ///
    /// The page is re-read rather than resumed, so the mailbox may have changed meanwhile: new mail at
    /// the top (a first page has no cursor to hold it back) shifts every row down, a deletion shifts
    /// them up. The shown rows therefore end at the last one shown, found by id; if that row is gone,
    /// or has moved because its thread got newer mail, they end at its date.
    fn unshown(&self, items: &[Value]) -> Vec<Value> {
        let Some(last) = &self.last else {
            return items.to_vec();
        };
        let anchor = items.iter().position(|item| {
            item.get("id").and_then(Value::as_str) == Some(last.id.as_str())
                && item_date(item) == Some(last.date)
        });
        match anchor {
            Some(position) => items[position + 1..].to_vec(),
            None => items
                .iter()
                .skip_while(|item| item_date(item).is_none_or(|date| date > last.date))
                .cloned()
                .collect(),
        }
    }

    /// Read this page again (with the `fetched` limit it was just read with)
    /// and resume after `shown`, the rows of it this unified page showed.
    fn resume(&self, shown: &[Value], fetched: u32) -> AccountCursor {
        let Some(row) = shown.last() else {
            return AccountCursor {
                depth: fetched,
                ..self.clone()
            };
        };
        AccountCursor {
            cursor: self.cursor.clone(),
            depth: fetched,
            last: Some(ShownRow {
                date: item_date(row).unwrap_or(i64::MIN),
                id: row
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            }),
        }
    }
}

/// An account absent from the map has no more pages.
pub type AccountCursors = BTreeMap<String, AccountCursor>;

pub fn encode_cursor(cursors: &AccountCursors) -> String {
    let json = serde_json::to_vec(cursors).unwrap_or_default();
    format!("unified:{}", URL_SAFE_NO_PAD.encode(json))
}

pub fn decode_cursor(cursor: &str) -> Option<AccountCursors> {
    let encoded = cursor.strip_prefix("unified:")?;
    let json = URL_SAFE_NO_PAD.decode(encoded.as_bytes()).ok()?;
    serde_json::from_slice(&json).ok().or_else(|| {
        // Cursors minted before the held-row state: a bare page cursor per account.
        let legacy: BTreeMap<String, String> = serde_json::from_slice(&json).ok()?;
        Some(
            legacy
                .into_iter()
                .map(|(account, cursor)| {
                    let cursor = AccountCursor {
                        cursor: Some(cursor),
                        ..AccountCursor::default()
                    };
                    (account, cursor)
                })
                .collect(),
        )
    })
}

/// One account's answer to a unified page request, read from `from` with
/// [`AccountCursor::fetch_limit`] of the request's limit.
pub struct AccountPage {
    pub account_id: String,
    pub from: AccountCursor,
    pub result: Result<Value, String>,
}

fn item_date(item: &Value) -> Option<i64> {
    item.get("date").and_then(Value::as_i64)
}

/// Merge per-account pages into one newest-first page.
///
/// Each account pages independently, so their pages end at different dates: a busy account's 50
/// rows may reach back a week while a quiet one's reach back a year. Showing every row would leave
/// a hole — the quiet account's old mail sits right under the busy one's last row, with the busy
/// account's rows in between still unread.
///
/// So the page stops at the newest of the oldest dates among accounts that have more to give; rows
/// below it are held back and the account's cursor records the last row shown, to resume after it.
/// `limit` is the request's own limit, which each page was read with per
/// [`AccountCursor::fetch_limit`].
pub fn merge_pages(pages: Vec<AccountPage>, items_field: &str, limit: u32) -> Value {
    let mut accounts = Vec::new();
    let mut next_cursors = AccountCursors::new();
    let mut folder_unreads = Map::new();
    let mut failures = Vec::new();
    let mut failed = Vec::new();
    let mut search_incomplete = false;
    for AccountPage {
        account_id,
        from,
        result,
    } in pages
    {
        match result {
            Ok(result) => {
                // One account's live search failing leaves the merged list
                // short of its server-only hits; the view must still say so.
                search_incomplete |= result
                    .get("search_incomplete")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let items = result
                    .get(items_field)
                    .and_then(Value::as_array)
                    .map(|items| from.unshown(items))
                    .unwrap_or_default();
                let unread = result
                    .get("folder_unread")
                    .and_then(Value::as_u64)
                    .unwrap_or_default();
                folder_unreads.insert(account_id.clone(), json!(unread));
                let next = result
                    .get("next_cursor")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                accounts.push((account_id, from, items, next));
            }
            Err(message) => {
                failures.push(json!({ "account_id": account_id, "message": message }));
                failed.push((account_id, from));
            }
        }
    }
    let cutoff = accounts
        .iter()
        .filter(|(_, _, _, next)| next.is_some())
        .filter_map(|(_, _, items, _)| items.iter().filter_map(item_date).min())
        .max();
    let mut items = Vec::new();
    for (account_id, from, page, next) in accounts {
        let shown = match cutoff {
            Some(cutoff) => page
                .iter()
                .take_while(|item| item_date(item).is_none_or(|date| date >= cutoff))
                .count(),
            None => page.len(),
        };
        if shown < page.len() {
            let fetched = from.fetch_limit(limit);
            next_cursors.insert(account_id, from.resume(&page[..shown], fetched));
        } else if let Some(next) = next {
            let next = AccountCursor {
                cursor: Some(next),
                ..AccountCursor::default()
            };
            next_cursors.insert(account_id, next);
        }
        items.extend(page.into_iter().take(shown));
    }
    // An account that failed this time (offline, a live search erroring) is
    // asked again from the same place on the next page rather than dropped
    // for good. Only while the others still page on, though: once they are
    // done, a retry alone would keep offering a next page that may never come.
    if !next_cursors.is_empty() {
        next_cursors.extend(failed);
    }
    items.sort_by_key(|item| std::cmp::Reverse(item_date(item)));
    let folder_unread = folder_unreads
        .values()
        .filter_map(Value::as_u64)
        .sum::<u64>();
    let mut out = json!({
        items_field: items,
        "folder_unread": folder_unread,
        "folder_unreads": folder_unreads,
        "failures": failures,
    });
    if !next_cursors.is_empty() {
        out.as_object_mut().unwrap().insert(
            "next_cursor".to_string(),
            Value::String(encode_cursor(&next_cursors)),
        );
    }
    if search_incomplete {
        out.as_object_mut()
            .unwrap()
            .insert("search_incomplete".to_string(), Value::Bool(true));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: u32 = 50;

    fn page(account: &str, from: AccountCursor, result: Result<Value, String>) -> AccountPage {
        AccountPage {
            account_id: account.to_string(),
            from,
            result,
        }
    }

    fn at(cursor: &str) -> AccountCursor {
        AccountCursor {
            cursor: Some(cursor.to_string()),
            ..AccountCursor::default()
        }
    }

    fn after(depth: u32, date: i64, id: &str) -> AccountCursor {
        AccountCursor {
            cursor: None,
            depth,
            last: Some(ShownRow {
                date,
                id: id.to_string(),
            }),
        }
    }

    fn ids(merged: &Value) -> Vec<&str> {
        merged["threads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["id"].as_str().unwrap())
            .collect()
    }

    fn next(merged: &Value) -> AccountCursors {
        decode_cursor(merged["next_cursor"].as_str().unwrap()).unwrap()
    }

    // A quiet account with one shown row (q27) of its first page; the page is
    // re-read as given.
    fn quiet_rows(rows: Value) -> AccountPage {
        page(
            "quiet",
            after(LIMIT, 27, "q27"),
            Ok(json!({ "threads": rows })),
        )
    }

    #[test]
    fn cursor_round_trips_per_account_state() {
        let cursors = AccountCursors::from([
            ("first@example.com".to_string(), at("date:200:7")),
            ("second@example.com".to_string(), after(100, 100, "t")),
        ]);
        let encoded = encode_cursor(&cursors);
        assert!(encoded.starts_with("unified:"));
        assert_eq!(decode_cursor(&encoded), Some(cursors));
        assert_eq!(decode_cursor("date:100:3"), None);
    }

    #[test]
    fn legacy_cursors_resume_at_each_account_cursor() {
        let legacy = BTreeMap::from([("first".to_string(), "date:200:7".to_string())]);
        let encoded = format!(
            "unified:{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&legacy).unwrap())
        );
        assert_eq!(decode_cursor(&encoded).unwrap()["first"], at("date:200:7"));
    }

    #[test]
    fn pages_merge_sort_counts_cursors_and_failures() {
        let merged = merge_pages(
            vec![
                page(
                    "first",
                    AccountCursor::default(),
                    Ok(json!({
                        "threads": [{"id":"old","date":100}],
                        "folder_unread": 2,
                        "next_cursor": "date:100:1"
                    })),
                ),
                page(
                    "second",
                    AccountCursor::default(),
                    Ok(json!({"threads":[{"id":"new","date":200}],"folder_unread":3})),
                ),
                page(
                    "broken",
                    AccountCursor::default(),
                    Err("offline".to_string()),
                ),
            ],
            "threads",
            LIMIT,
        );
        assert_eq!(ids(&merged), ["new", "old"]);
        assert_eq!(merged["folder_unread"], 5);
        assert_eq!(merged["folder_unreads"]["first"], 2);
        assert_eq!(merged["failures"][0]["account_id"], "broken");
        assert!(merged.get("search_incomplete").is_none());
    }

    #[test]
    fn rows_older_than_an_unfinished_account_wait_for_its_next_page() {
        // "busy" has more mail after day 26; "quiet" has nothing more at all.
        // Its day-21 row must not land right under day 26 while busy's days
        // 25..22 are still unread.
        let merged = merge_pages(
            vec![
                page(
                    "busy",
                    AccountCursor::default(),
                    Ok(json!({
                        "threads": [{"id":"b28","date":28}, {"id":"b26","date":26}],
                        "next_cursor": "date:26:1"
                    })),
                ),
                page(
                    "quiet",
                    AccountCursor::default(),
                    Ok(
                        json!({"threads": [{"id":"q27","date":27}, {"id":"q21","date":21}, {"id":"q10","date":10}]}),
                    ),
                ),
            ],
            "threads",
            LIMIT,
        );
        assert_eq!(ids(&merged), ["b28", "q27", "b26"]);
        assert_eq!(
            next(&merged),
            AccountCursors::from([
                ("busy".to_string(), at("date:26:1")),
                ("quiet".to_string(), after(LIMIT, 27, "q27")),
            ])
        );
    }

    #[test]
    fn a_resumed_page_drops_the_rows_already_shown() {
        let merged = merge_pages(
            vec![
                page(
                    "busy",
                    at("date:26:1"),
                    Ok(json!({"threads": [{"id":"b24","date":24}, {"id":"b22","date":22}]})),
                ),
                quiet_rows(
                    json!([{"id":"q27","date":27}, {"id":"q21","date":21}, {"id":"q10","date":10}]),
                ),
            ],
            "threads",
            LIMIT,
        );
        assert_eq!(ids(&merged), ["b24", "b22", "q21", "q10"]);
        assert!(merged.get("next_cursor").is_none());
    }

    #[test]
    fn mail_arriving_above_the_shown_rows_does_not_repeat_them() {
        // q30 arrived since: counting one shown row would show q27 again.
        let merged = merge_pages(
            vec![quiet_rows(json!([
                {"id":"q30","date":30}, {"id":"q27","date":27}, {"id":"q21","date":21}
            ]))],
            "threads",
            LIMIT,
        );
        assert_eq!(ids(&merged), ["q21"]);
    }

    #[test]
    fn a_shown_row_deleted_since_does_not_hide_the_next_one() {
        // q27 is gone: counting one shown row would skip q21 unseen.
        let merged = merge_pages(
            vec![quiet_rows(
                json!([{"id":"q21","date":21}, {"id":"q10","date":10}]),
            )],
            "threads",
            LIMIT,
        );
        assert_eq!(ids(&merged), ["q21", "q10"]);
    }

    #[test]
    fn held_rows_are_read_again_as_deep_as_they_were_read() {
        // A deep reload (limit 100) reads a feed account, which has no cursor,
        // and holds back its older cards. The next page, at the usual limit,
        // must still read the feed 100 deep or the held cards fall off it.
        let merged = merge_pages(
            vec![
                page(
                    "busy",
                    AccountCursor::default(),
                    Ok(json!({"threads": [{"id":"b28","date":28}], "next_cursor": "date:28:1"})),
                ),
                page(
                    "feeds",
                    AccountCursor::default(),
                    Ok(json!({"threads": [{"id":"f30","date":30}, {"id":"f20","date":20}]})),
                ),
            ],
            "threads",
            100,
        );
        let feeds = next(&merged)["feeds"].clone();
        assert_eq!(feeds, after(100, 30, "f30"));
        assert_eq!(feeds.fetch_limit(LIMIT), 100);
        assert_eq!(AccountCursor::default().fetch_limit(LIMIT), LIMIT);
    }

    #[test]
    fn a_failed_account_is_asked_again_while_others_page_on() {
        let merged = merge_pages(
            vec![
                page(
                    "busy",
                    AccountCursor::default(),
                    Ok(json!({"threads": [{"id":"b28","date":28}], "next_cursor": "date:28:1"})),
                ),
                page("broken", at("date:50:2"), Err("offline".to_string())),
            ],
            "threads",
            LIMIT,
        );
        assert_eq!(
            next(&merged),
            AccountCursors::from([
                ("busy".to_string(), at("date:28:1")),
                ("broken".to_string(), at("date:50:2")),
            ])
        );

        let last = merge_pages(
            vec![
                page("done", AccountCursor::default(), Ok(json!({"threads": []}))),
                page("broken", at("date:50:2"), Err("offline".to_string())),
            ],
            "threads",
            LIMIT,
        );
        assert!(last.get("next_cursor").is_none());
    }

    #[test]
    fn one_incomplete_account_search_marks_the_merged_page() {
        let merged = merge_pages(
            vec![
                page(
                    "first",
                    AccountCursor::default(),
                    Ok(json!({"threads": []})),
                ),
                page(
                    "second",
                    AccountCursor::default(),
                    Ok(json!({"threads": [], "search_incomplete": true})),
                ),
            ],
            "threads",
            LIMIT,
        );
        assert_eq!(merged["search_incomplete"], true);
    }
}
