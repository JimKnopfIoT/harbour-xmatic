//! Message search in one room, on the SDK's own encrypted index. No
//! server-side fallback: a search that sometimes asks the homeserver is worse.

use matrix_sdk::Client;
use serde_json::{Value, json};

use crate::text::scrub_ids;

/// How much of a matching message a result row carries.
const SNIPPET_CHARS: usize = 160;

/// Reduces what was typed to its words and requires each: Tantivy's syntax
/// would change the meaning silently, and "any of these" widens instead of narrows.
fn build_query(input: &str) -> String {
    let mut query = String::new();
    for word in input.split(|c: char| !c.is_alphanumeric() && c != '\'') {
        if word.is_empty() {
            continue;
        }
        if !query.is_empty() {
            query.push(' ');
        }
        query.push('+');
        query.push_str(word);
    }
    query
}

/// One line of a match: whitespace collapses, or a message with newlines paints
/// itself down the results page.
fn snippet(body: &str) -> String {
    let mut text = String::new();
    for word in body.split_whitespace() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(word);
        if text.chars().count() >= SNIPPET_CHARS {
            text.push('…');
            break;
        }
    }
    text
}

/// Feeds what this device already holds into the index: the SDK indexes on
/// store writes only. Local, and re-running costs nothing - keyed by event id.
pub async fn index_room(client: &Client, room_id: &str) -> Result<usize, String> {
    let parsed = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())?;

    let (cache, _drop_handles) = room
        .event_cache()
        .await
        .map_err(|error| format!("event cache unavailable: {}", scrub_ids(&error.to_string())))?;
    let mut events = cache
        .events()
        .await
        .map_err(|error| format!("stored events unreadable: {}", scrub_ids(&error.to_string())))?;
    // Bounded: this loads every stored event at once and holds the index lock
    // while it writes them. The newest are what a search wants anyway.
    const MOST: usize = 20_000;
    if events.len() > MOST {
        let drop_to = events.len() - MOST;
        events.drain(..drop_to);
    }
    let count = events.len();
    if count == 0 {
        return Ok(0);
    }

    let rules = room.clone_info().room_version_rules_or_default().redaction;
    let mut index = client.search_index().lock().await;
    index
        .bulk_handle_timeline_event(events.into_iter(), &cache, &parsed, &rules)
        .await
        .map_err(|error| format!("indexing failed: {}", scrub_ids(&error.to_string())))?;
    Ok(count)
}

/// Searches one room, one page per call. Stateless: holding the SDK's iterator
/// across commands means a search a second command can invalidate unnoticed.
pub async fn room(
    client: &Client,
    room_id: &str,
    query: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<Value>, String> {
    let parsed = matrix_sdk::ruma::RoomId::parse(room_id)
        .map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())?;

    let query = build_query(query);
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let ids = room
        .search(&query, limit, Some(offset))
        .await
        .map_err(|error| format!("search failed: {}", scrub_ids(&error.to_string())))?;

    let mut rows = Vec::with_capacity(ids.len());
    for id in ids {
        // From the cache where it has it: the index only names events that passed
        // through this device, so the fetch is the exception.
        let Ok(fetched) = room.load_or_fetch_event(&id, None).await else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(fetched.raw().json().get()) else {
            continue;
        };

        let body = value
            .get("content")
            .and_then(|content| content.get("body"))
            .and_then(|body| body.as_str())
            .unwrap_or_default();
        // A hit whose body cannot be read any more - redacted after indexing - is not
        // a row.
        if body.is_empty() {
            continue;
        }

        let sender = value.get("sender").and_then(|s| s.as_str()).unwrap_or_default();
        let name = match matrix_sdk::ruma::UserId::parse(sender) {
            Ok(user) => room
                .get_member_no_sync(&user)
                .await
                .ok()
                .flatten()
                .and_then(|member| member.display_name().map(str::to_owned))
                .unwrap_or_else(|| sender.to_owned()),
            Err(_) => sender.to_owned(),
        };

        rows.push(json!({
            "eventId": id.to_string(),
            "sender": sender,
            "senderName": name,
            "timestamp": value.get("origin_server_ts").and_then(Value::as_u64).unwrap_or(0),
            "body": snippet(body),
        }));
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_word_is_required() {
        assert_eq!(build_query("hello world"), "+hello +world");
    }

    #[test]
    fn syntax_is_not_syntax() {
        // None of these may reach the parser as operators.
        assert_eq!(build_query("a AND b"), "+a +AND +b");
        assert_eq!(build_query("\"quoted"), "+quoted");
        assert_eq!(build_query("wild*card"), "+wild +card");
        assert_eq!(build_query("-minus +plus"), "+minus +plus");
        assert_eq!(build_query("a:b (c)"), "+a +b +c");
    }

    #[test]
    fn nothing_searchable_is_no_query() {
        assert_eq!(build_query(""), "");
        assert_eq!(build_query("   "), "");
        assert_eq!(build_query("*?:"), "");
    }

    #[test]
    fn an_apostrophe_stays_inside_a_word() {
        assert_eq!(build_query("don't"), "+don't");
    }

    #[test]
    fn a_snippet_is_one_line() {
        assert_eq!(snippet("two\n\nlines  here"), "two lines here");
        assert!(snippet(&"word ".repeat(200)).ends_with('…'));
    }
}
