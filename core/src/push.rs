//! UnifiedPush through Leghorn. See docs/PUSH.md.

use leghorn::matrix::{Notification, Pusher as LeghornPusher};
use leghorn::{Config as PushConfig, Event as PushEvent, Leghorn};
use serde_json::{json, Value};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

/// Org and app must match the desktop file's [X-Sailjail] names. No fallback
/// gateway; the user picks one.
pub const PUSH: PushConfig = PushConfig::new("org.xmatic", "xmatic")
    .description("xmatic")
    .matrix(APP_ID)
    .matrix_without_fallback()
    .matrix_device("xmatic", "en");

/// Must not change: existing pushers on homeservers are filed under it.
const APP_ID: &str = "org.xmatic.xmatic";

pub async fn start() -> (
    Result<Leghorn, String>,
    UnboundedReceiver<PushEvent>,
) {
    // Otherwise Leghorn's gateway probe installs ring.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let (events, receiver) = unbounded_channel();
    let handler = move |push: PushEvent| {
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

/// Whether `gateway` is an https URL.
pub fn gateway_is_sound(gateway: &str) -> bool {
    let gateway = gateway.trim();
    gateway.len() > 8
        && gateway
            .get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
}

/// The host of `url`, without scheme, port or path.
pub fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    authority.split(':').next().unwrap_or_default().to_owned()
}

/// The UnifiedPush project's gateway, used only where the user picked it.
pub const PUBLIC_GATEWAY: &str = "https://matrix.gateway.unifiedpush.org/_matrix/push/v1/notify";

/// Which gateway the user picked. Nothing registers before a pick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Gateway {
    #[default]
    Unset,
    /// The push server's own, found by Leghorn.
    Server,
    Public,
    Other(String),
}

impl Gateway {
    pub fn parse(mode: &str, url: &str) -> Result<Self, &'static str> {
        match mode {
            "" => Ok(Self::Unset),
            "server" => Ok(Self::Server),
            "public" => Ok(Self::Public),
            "other" if gateway_is_sound(url) => Ok(Self::Other(url.trim().to_owned())),
            "other" => Err("the push gateway has to be an https address"),
            _ => Err("unknown gateway choice"),
        }
    }

    pub fn mode(&self) -> &'static str {
        match self {
            Self::Unset => "",
            Self::Server => "server",
            Self::Public => "public",
            Self::Other(_) => "other",
        }
    }

    /// The gateway to register, given what the push server offered.
    pub fn resolve(&self, discovered: Option<&str>) -> Option<String> {
        match self {
            Self::Unset => None,
            Self::Server => discovered.filter(|gateway| gateway_is_sound(gateway)).map(str::to_owned),
            Self::Public => Some(PUBLIC_GATEWAY.to_owned()),
            Self::Other(url) => Some(url.clone()),
        }
    }
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
    fn only_https_gateways_are_sound() {
        assert!(gateway_is_sound("https://push.example.org/_matrix/push/v1/notify"));
        assert!(gateway_is_sound(" HTTPS://push.example.org "));
        assert!(!gateway_is_sound("http://push.example.org/_matrix/push/v1/notify"));
        assert!(!gateway_is_sound("https://"));
        assert!(!gateway_is_sound(""));
    }

    #[test]
    fn the_gateway_is_what_was_picked() {
        let found = "https://push.example.org/_matrix/push/v1/notify";
        let mine = "https://gateway.example.net/_matrix/push/v1/notify";
        assert_eq!(Gateway::Unset.resolve(Some(found)), None);
        assert_eq!(Gateway::Server.resolve(Some(found)).as_deref(), Some(found));
        assert_eq!(Gateway::Server.resolve(None), None);
        assert_eq!(Gateway::Server.resolve(Some("http://push.example.org/notify")), None);
        assert_eq!(Gateway::Public.resolve(None).as_deref(), Some(PUBLIC_GATEWAY));
        let other = Gateway::parse("other", mine).unwrap();
        assert_eq!(other.resolve(Some(found)).as_deref(), Some(mine));
    }

    #[test]
    fn only_the_host_is_kept() {
        assert_eq!(host("https://updates.push.services.mozilla.com/wpush/v2/gAAA"), "updates.push.services.mozilla.com");
        assert_eq!(host("https://ntfy.example:8443/upAbc?up=1"), "ntfy.example");
        assert_eq!(host("https://user:pw@ntfy.example/up"), "ntfy.example");
    }

    #[test]
    fn a_pick_is_parsed_strictly() {
        assert_eq!(Gateway::parse("", ""), Ok(Gateway::Unset));
        assert_eq!(Gateway::parse("server", "ignored"), Ok(Gateway::Server));
        assert!(Gateway::parse("other", "http://gateway.example.net").is_err());
        assert!(Gateway::parse("other", "").is_err());
        assert!(Gateway::parse("fallback", "").is_err());
        assert_eq!(Gateway::parse("public", "").map(|pick| pick.mode()), Ok("public"));
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

/// Sets the pusher with `gateway` unless already present, and deletes this app's others.
pub async fn register(
    client: &matrix_sdk::Client,
    pusher: &LeghornPusher,
    gateway: &str,
) -> Result<(), String> {
    use matrix_sdk::ruma::api::client::push::{get_pushers, Pusher, PusherIds, PusherKind};

    if !gateway_is_sound(gateway) || !gateway_is_sound(&pusher.pushkey) {
        return Err("the push gateway and address have to be https".to_owned());
    }
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
            && matches!(&existing.kind, PusherKind::Http(data) if data.url == gateway)
    });
    if !in_place {
        let mut chosen = pusher.clone();
        chosen.gateway = Some(gateway.to_owned());
        let wanted: Pusher = chosen
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

/// Whether a `push.json` from the old connector holds a registration.
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
