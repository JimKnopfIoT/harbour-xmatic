//! Link previews. The homeserver fetches the page (`preview_url`) and answers
//! with OpenGraph fields; this module asks, trims, and caches. The phone never
//! touches the link, and the server learns every link it is asked about — the
//! UI decides when to ask, this file only answers.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use matrix_sdk::config::RequestConfig;
use matrix_sdk::ruma::api::client::{authenticated_media, media};
use matrix_sdk::ruma::api::Metadata;
use matrix_sdk::Client;
use serde_json::{json, Value};

use crate::protocol::reply_ok;
use crate::runtime::Sink;
use crate::text::strip_bidi;

/// What a card holds at most. Both are a stranger's text about a page.
const TITLE_CHARS: usize = 120;
const DESCRIPTION_CHARS: usize = 300;

/// Answered previews, by exact url. A link is scrolled past many times and
/// every ask is a request the server rate-limits.
static KNOWN: LazyLock<Mutex<HashMap<String, Value>>> = LazyLock::new(Default::default);

/// Where the memory stops: a room full of distinct links is a way to grow it,
/// and past this the oldest answers are simply asked for again.
const KNOWN_LIMIT: usize = 500;

/// Decoration is asked once and briefly. The SDK's default policy retries a 429
/// for up to fifteen minutes - see PITFALLS on rate limits - and a preview is
/// never worth a queue of retries behind the messages themselves.
const VERSIONS_TIMEOUT: Duration = Duration::from_secs(20);

fn request_config() -> RequestConfig {
    RequestConfig::new().disable_retry().timeout(Duration::from_secs(20))
}

/// The server said it has no preview endpoint at all. One answer for the
/// session, not one request per link.
static UNSUPPORTED: AtomicBool = AtomicBool::new(false);

pub fn forget() {
    KNOWN.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clear();
    UNSUPPORTED.store(false, Ordering::Relaxed);
}

/// The one command. Never an error reply: "could not ask" and "no preview" look
/// the same to a card, and the card must not spin over either.
pub async fn handle(client: Option<Client>, sink: &Sink, id: u64, url: String) {
    let answer = match host_of(&url) {
        None => unavailable(&url),
        Some(host) => {
            let cached = KNOWN
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&url)
                .cloned();
            match cached {
                Some(known) => known,
                None => {
                    let asked = match client {
                        Some(client) => fetch(&client, &url).await,
                        None => Asked::CouldNotAsk,
                    };
                    match asked {
                        Asked::CouldNotAsk => transient(&url),
                        answered => {
                            let fresh = match answered {
                                Asked::Fields(fields) => card(&url, &host, &fields),
                                _ => unavailable(&url),
                            };
                            let mut known =
                                KNOWN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                            if known.len() >= KNOWN_LIMIT {
                                known.clear();
                            }
                            known.insert(url.clone(), fresh.clone());
                            fresh
                        }
                    }
                }
            }
        }
    };
    sink.emit(reply_ok(id, answer));
}

fn unavailable(url: &str) -> Value {
    json!({ "url": url, "available": false })
}

/// "Could not ask" - offline, rate-limited, timed out. Not remembered here, and
/// `retry` tells the bridge not to either.
fn transient(url: &str) -> Value {
    json!({ "url": url, "available": false, "retry": true })
}

/// Only http and https, and only with a host: `preview_url` would take more,
/// but nothing else is a page.
fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    parsed.host_str().map(|host| host.to_lowercase())
}

/// What asking the server came to. Two kinds of nothing, kept apart: a page
/// that has nothing to say is remembered, a server that could not be asked
/// is not.
enum Asked {
    Fields(HashMap<String, Value>),
    Nothing,
    CouldNotAsk,
}

/// The endpoint the server advertises - the authenticated one where it speaks
/// Matrix 1.11, the older one otherwise - asked once. No second request on a
/// failure: under a 429 that would be the retry the config turned off.
async fn fetch(client: &Client, url: &str) -> Asked {
    if UNSUPPORTED.load(Ordering::Relaxed) {
        return Asked::Nothing;
    }
    // Cached by the SDK for a day; the refresh after that has no config of its
    // own, so it is bounded here like the request itself.
    let versions = match tokio::time::timeout(VERSIONS_TIMEOUT, client.supported_versions()).await {
        Ok(Ok(versions)) => versions,
        _ => return Asked::CouldNotAsk,
    };
    let authenticated = authenticated_media::get_media_preview::v1::Request::PATH_BUILDER
        .is_supported(&versions);
    let raw = if authenticated {
        client
            .send(authenticated_media::get_media_preview::v1::Request::new(url.to_owned()))
            .with_request_config(request_config())
            .await
            .map(|response| response.data)
    } else {
        #[allow(deprecated)]
        client
            .send(media::get_media_preview::v3::Request::new(url.to_owned()))
            .with_request_config(request_config())
            .await
            .map(|response| response.data)
    };
    match raw {
        // The server answered about the page: a 404 or an empty body is its
        // verdict on the page, not on the connection.
        Ok(Some(data)) => match serde_json::from_str(data.get()) {
            Ok(fields) => Asked::Fields(fields),
            Err(_) => Asked::Nothing,
        },
        Ok(None) => Asked::Nothing,
        // A server without the endpoint says so once. A 400/404 is its answer
        // about the page; 401/403/429, 5xx and silence are about the connection.
        Err(error) if error.is_endpoint_not_implemented() => {
            UNSUPPORTED.store(true, Ordering::Relaxed);
            Asked::Nothing
        }
        Err(error) => match error.as_client_api_error().map(|api| api.status_code.as_u16()) {
            Some(400) | Some(404) => Asked::Nothing,
            _ => Asked::CouldNotAsk,
        },
    }
}

fn text_field(fields: &HashMap<String, Value>, key: &str, limit: usize) -> Option<String> {
    let text = fields.get(key)?.as_str()?.trim();
    if text.is_empty() {
        return None;
    }
    let mut cut: String = text.chars().take(limit).collect();
    if cut.chars().count() < text.chars().count() {
        cut.push('…');
    }
    Some(strip_bidi(&cut))
}

fn card(url: &str, host: &str, fields: &HashMap<String, Value>) -> Value {
    let title = text_field(fields, "og:title", TITLE_CHARS);
    let description = text_field(fields, "og:description", DESCRIPTION_CHARS);
    let site = text_field(fields, "og:site_name", TITLE_CHARS);
    // Only a picture the server re-hosted: `og:image` may only be an mxc uri
    // here, a stray http address would send the phone to the page after all.
    let image = fields
        .get("og:image")
        .and_then(Value::as_str)
        .filter(|uri| uri.starts_with("mxc://"))
        .map(|uri| json!({ "url": uri }));
    let image_size = fields
        .get("matrix:image:size")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    // A card with nothing on it is no card: the host alone is already in the link.
    let available = title.is_some() || description.is_some() || image.is_some();
    json!({
        "url": url,
        "available": available,
        "host": host,
        "title": title,
        "description": description,
        "siteName": site,
        "image": image,
        "imageSize": image_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_have_a_host() {
        assert_eq!(host_of("https://Example.org/a?b").as_deref(), Some("example.org"));
        assert!(host_of("ftp://example.org/").is_none());
        assert!(host_of("javascript:alert(1)").is_none());
        assert!(host_of("mailto:someone@example.org").is_none());
    }

    #[test]
    fn a_card_needs_something_on_it() {
        let empty = card("https://example.org", "example.org", &HashMap::new());
        assert_eq!(empty["available"], json!(false));

        let mut fields = HashMap::new();
        fields.insert("og:title".to_owned(), json!("  A page  "));
        let some = card("https://example.org", "example.org", &fields);
        assert_eq!(some["available"], json!(true));
        assert_eq!(some["title"], json!("A page"));
    }

    /// The picture is the server's copy or nothing - never an address the
    /// phone would fetch itself.
    #[test]
    fn only_rehosted_pictures_are_passed_on() {
        let mut fields = HashMap::new();
        fields.insert("og:image".to_owned(), json!("https://tracker.example/pixel.gif"));
        let card_value = card("https://example.org", "example.org", &fields);
        assert_eq!(card_value["image"], Value::Null);
        assert_eq!(card_value["available"], json!(false));

        fields.insert("og:image".to_owned(), json!("mxc://example.org/abc"));
        let card_value = card("https://example.org", "example.org", &fields);
        assert_eq!(card_value["image"]["url"], json!("mxc://example.org/abc"));
    }

    #[test]
    fn long_text_is_cut_with_an_ellipsis() {
        let mut fields = HashMap::new();
        fields.insert("og:description".to_owned(), json!("x".repeat(500)));
        let value = card("https://example.org", "example.org", &fields);
        let text = value["description"].as_str().unwrap();
        assert_eq!(text.chars().count(), DESCRIPTION_CHARS + 1);
        assert!(text.ends_with('…'));
    }
}
