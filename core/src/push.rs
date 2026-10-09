//! UnifiedPush through Leghorn. See docs/PUSH.md.

use leghorn::matrix::{Notification, Pusher as LeghornPusher};
use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

/// Org and app must match the desktop file's [X-Sailjail] names.
pub const PUSH: leghorn::Config = leghorn::Config::new("org.xmatic", "xmatic")
    .description("xmatic")
    .matrix(APP_ID);

/// Must not change: existing pushers on homeservers are filed under it.
const APP_ID: &str = "org.xmatic.xmatic";

/// Sets an environment variable; call before any thread starts.
pub fn prelude() {
    leghorn::prelude();
}

pub async fn start() -> (
    Result<leghorn::Leghorn, String>,
    UnboundedReceiver<leghorn::Event>,
) {
    // Otherwise Leghorn's gateway probe installs ring.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let (events, receiver) = unbounded_channel();
    let handler = move |push: leghorn::Event| {
        let events = events.clone();
        async move {
            let _ = events.send(push);
            Ok::<(), std::convert::Infallible>(())
        }
    };
    let started = leghorn::start(&PUSH, handler)
        .await
        .map_err(|error| crate::text::scrub_ids(&error.to_string()));
    (started, receiver)
}

/// The woken process: claims the name, collects pushes until Leghorn goes idle.
/// Does not register.
pub fn run_wake() -> Value {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    if log::set_boxed_logger(Box::new(WakeLog)).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }

    let taken = std::sync::Arc::new(std::sync::Mutex::new(Wake::default()));
    let handler = {
        let taken = taken.clone();
        move |push: leghorn::Event| {
            let taken = taken.clone();
            async move {
                if let Ok(mut wake) = taken.lock() {
                    match push {
                        leghorn::Event::Message(body) => {
                            let target = message(&body);
                            if target["roomId"].is_string() && target["eventId"].is_string() {
                                wake.messages.push(target);
                            }
                        }
                        leghorn::Event::NewEndpoint(_) => wake.new_endpoint = true,
                        leghorn::Event::Unregistered => wake.unregistered = true,
                        leghorn::Event::Raw(_) => {}
                        leghorn::Event::Failed(error) => {
                            log::warn!(target: "leghorn", "{error}");
                        }
                    }
                }
                Ok::<(), std::convert::Infallible>(())
            }
        }
    };
    leghorn::run_wake(&PUSH, handler);

    let wake = taken.lock().map(|wake| wake.clone()).unwrap_or_default();
    json!({
        "messages": wake.messages,
        "newEndpoint": wake.new_endpoint,
        "unregistered": wake.unregistered,
    })
}

#[derive(Default, Clone)]
struct Wake {
    messages: Vec<Value>,
    new_endpoint: bool,
    unregistered: bool,
}

/// Leghorn's log lines to stderr, before the core exists.
struct WakeLog;

impl log::Log for WakeLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Info && metadata.target().starts_with("leghorn")
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "xmatic: push wake {} {}: {}",
                record.level(),
                record.target(),
                crate::text::scrub_ids(&record.args().to_string())
            );
        }
    }

    fn flush(&self) {}
}

/// Room and event of a Matrix push; both null for a count-only or foreign push.
pub fn message(body: &[u8]) -> Value {
    let note = Notification::from_slice(body).ok();
    let room = note.as_ref().and_then(|note| note.room_id.clone());
    let event = note.as_ref().and_then(|note| note.event_id.clone());
    match (room, event) {
        (Some(room), Some(event)) => json!({ "roomId": room, "eventId": event }),
        _ => json!({ "roomId": null, "eventId": null }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matrix_notification_names_its_room_and_event() {
        let body = br#"{"notification":{"event_id":"$abc","room_id":"!room:example.org",
                        "counts":{"unread":1}}}"#;
        assert_eq!(
            message(body),
            json!({ "roomId": "!room:example.org", "eventId": "$abc" })
        );
    }

    #[test]
    fn anything_else_is_not_ours() {
        for probe in [
            "",
            "not json at all",
            r#"{"notification":{}}"#,
            r#"{"notification":{"counts":{"unread":3}}}"#,
            r#"{"notification":{"room_id":"!room:example.org"}}"#,
            r#"{"hello":"world"}"#,
        ] {
            assert_eq!(
                message(probe.as_bytes()),
                json!({ "roomId": null, "eventId": null }),
                "accepted: {probe}"
            );
        }
    }

    #[test]
    fn leghorns_pusher_reads_as_ruma_s() {
        use matrix_sdk::ruma::api::client::push::{Pusher, PusherKind};
        let pusher: LeghornPusher = serde_json::from_value(json!({
            "pushkey": "https://push.example.org/up/abc",
            "app_id": APP_ID,
            "gateway": "https://push.example.org/_matrix/push/v1/notify",
            "app_display_name": "xmatic",
            "device_display_name": "phone",
            "lang": "en",
            "replaces": null,
        }))
        .unwrap();
        let ruma: Pusher = pusher.convert().unwrap();
        assert_eq!(ruma.ids.pushkey, "https://push.example.org/up/abc");
        assert_eq!(ruma.ids.app_id, APP_ID);
        let PusherKind::Http(data) = ruma.kind else {
            panic!("not an http pusher");
        };
        assert_eq!(data.url, "https://push.example.org/_matrix/push/v1/notify");
        assert_eq!(
            data.format,
            Some(matrix_sdk::ruma::push::PushFormat::EventIdOnly)
        );
    }
}

/// Sets Leghorn's pusher unless already present, and deletes this app's others.
pub async fn register(client: &matrix_sdk::Client, pusher: &LeghornPusher) -> Result<(), String> {
    use matrix_sdk::ruma::api::client::push::{get_pushers, Pusher, PusherIds, PusherKind};

    let current = client
        .send(get_pushers::v3::Request::new())
        .await
        .map_err(|error| crate::text::scrub_ids(&format!("could not read the pushers: {error}")))?;
    let ours: Vec<_> = current
        .pushers
        .into_iter()
        .filter(|existing| existing.ids.app_id == APP_ID)
        .collect();

    let in_place = ours.iter().any(|existing| {
        existing.ids.pushkey == pusher.pushkey
            && matches!(&existing.kind, PusherKind::Http(data) if data.url == pusher.gateway)
    });
    if !in_place {
        let wanted: Pusher = pusher
            .convert()
            .map_err(|error| format!("Leghorn's pusher did not parse: {error}"))?;
        client
            .pusher()
            .set(wanted, false)
            .await
            .map_err(|error| {
                crate::text::scrub_ids(&format!("could not register the pusher: {error}"))
            })?;
    }

    for stale in ours {
        if stale.ids.pushkey == pusher.pushkey {
            continue;
        }
        let _ = client
            .pusher()
            .delete(PusherIds::new(stale.ids.pushkey, APP_ID.to_owned()))
            .await;
    }
    Ok(())
}

/// Removes every pusher this app registered.
pub async fn clear_own_pushers(client: &matrix_sdk::Client) -> Result<(), String> {
    use matrix_sdk::ruma::api::client::push::{get_pushers, PusherIds};

    let response = client
        .send(get_pushers::v3::Request::new())
        .await
        .map_err(|error| {
            crate::text::scrub_ids(&format!("could not read the pushers: {error}"))
        })?;

    let mut failure = None;
    for pusher in response.pushers {
        if pusher.ids.app_id != APP_ID {
            continue;
        }
        let ids = PusherIds::new(pusher.ids.pushkey, APP_ID.to_owned());
        if let Err(error) = client.pusher().delete(ids).await {
            failure = Some(crate::text::scrub_ids(&format!(
                "could not remove the pusher: {error}"
            )));
        }
    }
    match failure {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

/// Whether the pre-Leghorn push.json had a registration.
pub fn legacy_enabled(path: &std::path::Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .and_then(|state| state.get("registrations")?.as_array().map(|list| !list.is_empty()))
        .unwrap_or(false)
}

/// Fetches the message a push named. `NotificationClient` reaches an event no
/// sync brought in; `MultipleProcesses`, because a woken process may be second.
pub async fn notification_for(
    client: &matrix_sdk::Client,
    room_id: &str,
    event_id: &str,
    sync: Option<std::sync::Arc<matrix_sdk_ui::sync_service::SyncService>>,
) -> Result<Value, String> {
    use matrix_sdk::ruma::{EventId, RoomId};
    use matrix_sdk_ui::notification_client::{
        NotificationClient, NotificationProcessSetup, NotificationStatus,
    };

    let room = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let event = EventId::parse(event_id).map_err(|_| "not an event identifier".to_owned())?;

    // One process: `MultipleProcesses` mints a dummy permit and starts a second
    // encryption sync beside the running one, both claiming to-device events.
    let setup = match sync {
        Some(sync_service) => NotificationProcessSetup::SingleProcess { sync_service },
        None => NotificationProcessSetup::MultipleProcesses,
    };
    let notifications =
        NotificationClient::new(client.clone(), setup)
            .await
            .map_err(|error| {
                crate::text::scrub_ids(&format!("could not open the notification client: {error}"))
            })?;

    match notifications.get_notification(&room, &event).await {
        Ok(NotificationStatus::Event(item)) => {
            let (kind, text) = notification_body(&item.event);
            Ok(json!({
                "roomId": room_id,
                "roomName": crate::text::strip_bidi(&item.room_computed_display_name),
                "sender": item
                    .sender_display_name
                    .as_deref()
                    .map(crate::text::strip_bidi)
                    .unwrap_or_default(),
                "previewKind": kind,
                "previewText": text,
                // The push rules decided this was worth a sound. Passed on rather than judged
                // again: a second opinion would only disagree with the account's own rules.
                "noisy": item.is_noisy.unwrap_or(false),
                "mention": item.has_mention.unwrap_or(false),
            }))
        }
        // Real answers, not failures: the rules say do not show it, the event is gone,
        // or the server never had it. Silence is correct.
        Ok(NotificationStatus::EventFilteredOut) => Err("filtered out".to_owned()),
        Ok(NotificationStatus::EventRedacted) => Err("redacted".to_owned()),
        Ok(NotificationStatus::EventNotFound) => Err("event not found".to_owned()),
        Err(error) => Err(crate::text::scrub_ids(&format!(
            "could not fetch the message: {error}"
        ))),
    }
}

/// The preview a banner shows, in the same two fields the room list produces —
/// so a push and an ordinary arrival read identically.
fn notification_body(
    event: &matrix_sdk_ui::notification_client::NotificationEvent,
) -> (&'static str, String) {
    use matrix_sdk::ruma::events::poll::unstable_start::UnstablePollStartEventContent;
    use matrix_sdk::ruma::events::room::message::MessageType;
    use matrix_sdk::ruma::events::{AnyMessageLikeEventContent, AnySyncTimelineEvent};
    use matrix_sdk_ui::notification_client::NotificationEvent;

    let NotificationEvent::Timeline(timeline) = event else {
        // An invitation. It has no body, and naming it by kind is what the
        // room list does too.
        return ("invite", String::new());
    };
    let AnySyncTimelineEvent::MessageLike(message) = timeline.as_ref() else {
        return ("", String::new());
    };
    match message.original_content() {
        Some(AnyMessageLikeEventContent::RoomMessage(content)) => match content.msgtype {
            MessageType::Text(body) => ("text", crate::text::strip_bidi(&body.body)),
            MessageType::Notice(body) => ("text", crate::text::strip_bidi(&body.body)),
            MessageType::Emote(body) => ("emote", crate::text::strip_bidi(&body.body)),
            MessageType::Image(_) => ("image", String::new()),
            MessageType::Video(_) => ("video", String::new()),
            MessageType::Audio(_) => ("audio", String::new()),
            MessageType::File(_) => ("file", String::new()),
            MessageType::Location(_) => ("location", String::new()),
            _ => ("", String::new()),
        },
        // The question alone: the fallback text carries the answers as well and
        // would spill them into the banner.
        Some(AnyMessageLikeEventContent::UnstablePollStart(
            UnstablePollStartEventContent::New(started),
        )) => ("poll", crate::text::strip_bidi(&started.poll_start.question.text)),
        // Encrypted here means it could not be decrypted: the keys for it never
        // reached this device. The banner says so rather than counting it.
        Some(AnyMessageLikeEventContent::RoomEncrypted(_)) | None => ("encrypted", String::new()),
        _ => ("", String::new()),
    }
}
