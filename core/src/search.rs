//! Message search inside one room.
//!
//! The index is the SDK's own (`matrix-sdk-search`, Tantivy): it is filled by
//! the event cache as events arrive, lives encrypted beside the store under
//! the same key, and never leaves the device. There is deliberately no
//! server-side fallback for unencrypted rooms - a search that sometimes asks
//! the homeserver would send the query out of the process for reasons the
//! user cannot see.

use matrix_sdk::Client;
use serde_json::{Value, json};

use crate::text::scrub_ids;

/// How much of a matching message a result row carries.
const SNIPPET_CHARS: usize = 160;

/// Turns what somebody typed into a query the index understands.
///
/// Tantivy's parser has a syntax - `+ - " * ? : ( ) [ ] { } ^ ~ \ /` and the
/// words AND, OR, NOT all mean something in it. A search box on a phone is
/// not the place for that: a stray quote would change the meaning of the
/// search silently, and an unbalanced one would fail it outright. So the
/// input is reduced to its words and each is required, which is what somebody
/// typing two words into a search box means. Without the `+` the parser
/// defaults to "any of these", and a second word would then widen the search
/// instead of narrowing it.
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

/// One line of a matching message: enough to recognise it, not the whole
/// thing. Whitespace collapses, because a message with newlines would
/// otherwise paint itself down the results page.
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

/// Feeds what this device already holds for a room into its index.
///
/// The SDK's indexer hangs off the event cache's *store writes*: an event is
/// indexed when the cache saves it, and events that were already saved before
/// the index existed are only ever read back, never written again. So a room
/// full of history known to this device is invisible to the search until
/// somebody hands it over - which is what this does. Local, no network, and
/// re-running it costs nothing: the index is keyed by event id, so an event it
/// already holds is replaced rather than duplicated.
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
    let events = cache
        .events()
        .await
        .map_err(|error| format!("stored events unreadable: {}", scrub_ids(&error.to_string())))?;
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

/// Searches `room_id` for `query` and returns one page of rows.
///
/// Stateless: the offset comes in and the caller asks for the next page with
/// a larger one. The SDK offers an iterator that holds the offset, but holding
/// it across commands would mean a search that a second command can invalidate
/// without the first noticing.
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
        // From the cache where the cache has it. The index only ever names
        // events that passed through this device, so the fetch is the
        // exception rather than the rule.
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
        // A hit whose body cannot be read any more - redacted after it was
        // indexed - is not a row. The index is cleaned up on redaction, but
        // this runs against whatever the store holds now.
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
