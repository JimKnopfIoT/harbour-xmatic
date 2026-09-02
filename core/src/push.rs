//! Push over UnifiedPush: the distributor is the daemon, this is only the
//! connector. Nothing registers until the user asks; see docs/PUSH.md.

use crate::protocol::event;
use crate::runtime::Sink;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender as StdSender};
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use unifiedpush::{auth_to_string, pubkey_to_string, EventSink, PushEvent, PushMessage};
use unifiedpush::UnifiedPush;
use unifiedpush_storage::{Distributor, TokenInstance, UnifiedPushStorage};

/// Not `org.xmatic.xmatic`: Sailjail grants one name and that one is spent on
/// the share activation. This namespace comes with the UnifiedPush permission.
const DBUS_NAME: &str = "org.unifiedpush.Connector.xmatic";

/// One registration per app here. The protocol allows several ("instances");
/// this app has one account and one stream of notifications.
const INSTANCE: &str = "xmatic";

/// What the connector thread is asked to do.
enum Command {
    /// Pick a distributor and register. Answers with `push.state`, and the
    /// endpoint follows as `push.endpoint` once the distributor calls back.
    Enable,
    /// Give the registration back. The endpoint dies with it.
    Disable,
    /// Report what is on the device and what this app has, without changing
    /// anything.
    Status,
}

/// The handle the runtime keeps. Dropping it ends the connector thread.
pub struct PushHandle {
    commands: UnboundedSender<Command>,
}

impl PushHandle {
    pub fn enable(&self) {
        let _ = self.commands.send(Command::Enable);
    }

    pub fn disable(&self) {
        let _ = self.commands.send(Command::Disable);
    }

    pub fn status(&self) {
        let _ = self.commands.send(Command::Status);
    }
}

/// Starts the connector. `None` where the bus or the name cannot be had - the
/// ordinary outcome on a device with no distributor installed.
pub fn start(path: PathBuf, sink: Arc<Sink>) -> PushHandle {
    let (commands, mut rx) = unbounded_channel::<Command>();

    // The crate's event channel is synchronous, so it gets a thread of its own
    // rather than blocking either executor.
    let (events_tx, events_rx) = channel::<PushEvent>();
    {
        let sink = sink.clone();
        std::thread::spawn(move || {
            for push in events_rx.iter() {
                forward(&sink, push);
            }
        });
    }

    let handle = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        // Not `Runtime::block_on`. See the module comment.
        zbus::block_on(async move {
            let storage = FileStorage::open(path);
            let unifiedpush =
                match UnifiedPush::new(DBUS_NAME, storage, EventChannel(events_tx), handle).await {
                    Ok(unifiedpush) => unifiedpush,
                    Err(error) => {
                        // The usual cause is the Sailjail permission missing because no distributor
                        // is installed, and D-Bus reports that as `ServiceUnknown`.
                        sink.emit(event(
                            "push.state",
                            json!({
                                "state": "unavailable",
                                "error": crate::text::scrub_ids(&error.to_string()),
                            }),
                        ));
                        return;
                    }
                };

            report(&sink, &unifiedpush).await;

            while let Some(command) = rx.recv().await {
                match command {
                    Command::Enable => {
                        // Before every register: unregistering makes the crate forget the
                        // distributor. And this is no availability check - two distributors look like none.
                        if !unifiedpush.try_use_default_distributor().await {
                            let names = unifiedpush.list_distributors().await;
                            match names.first() {
                                Some(first) => unifiedpush.save_distributor(first).await,
                                None => {
                                    sink.emit(event(
                                        "push.state",
                                        json!({ "state": "no-distributor" }),
                                    ));
                                    continue;
                                }
                            }
                        }
                        report_with(&sink, &unifiedpush, Some("registering")).await;
                        unifiedpush
                            .register(INSTANCE, Some("xmatic"), None)
                            .await;
                    }
                    Command::Disable => {
                        unifiedpush.unregister(INSTANCE).await;
                        report_with(&sink, &unifiedpush, Some("off")).await;
                    }
                    Command::Status => report(&sink, &unifiedpush).await,
                }
            }
        });
    });

    PushHandle { commands }
}

/// What is on the device and what this app holds, as one object. Every
/// `push.state` goes through here - a partial one blanks the page.
async fn report_with(sink: &Arc<Sink>, unifiedpush: &UnifiedPush, state: Option<&str>) {
    let distributors = unifiedpush.list_distributors().await;
    let chosen = unifiedpush.get_distributor().await;
    let derived = if distributors.is_empty() { "no-distributor" } else { "idle" };
    sink.emit(event(
        "push.state",
        json!({
            "state": state.unwrap_or(derived),
            // Bus names, which is all a distributor is identified by. The UI
            // shows the last segment; the whole name is what a report needs.
            "distributors": distributors,
            "distributor": chosen.as_ref().map(|d| d.name.clone()),
            // Whether the chosen one has ever answered with an endpoint. A
            // registration that was never acknowledged is not a working one.
            "acknowledged": chosen.as_ref().map(|d| d.ack).unwrap_or(false),
        }),
    ));
}

/// The whole picture, with the state derived from it.
async fn report(sink: &Arc<Sink>, unifiedpush: &UnifiedPush) {
    report_with(sink, unifiedpush, None).await;
}

/// One event from the distributor, as JSON for the bridge.
fn forward(sink: &Arc<Sink>, push: PushEvent) {
    match push {
        PushEvent::NewEndpoint { endpoint, .. } => {
            // All three: Web Push wants an encrypted body, and a gateway given only the
            // URL cannot make one. Whether it uses them is its business.
            sink.emit(event(
                "push.endpoint",
                json!({
                    "endpoint": endpoint.endpoint,
                    "p256dh": pubkey_to_string(&endpoint.pubkey).unwrap_or_default(),
                    "auth": auth_to_string(&endpoint.auth).unwrap_or_default(),
                }),
            ));
        }
        PushEvent::Message { message, .. } => {
            // Both shapes. A Matrix gateway hands the distributor the homeserver's plain
            // JSON, so the undecrypted case is the normal one, not an error.
            let (body, decrypted) = match message {
                PushMessage::Decrypted { content } => (content, true),
                PushMessage::Raw { content } => (content, false),
            };
            // Parsed here rather than in the bridge: a pure function with tests, and a
            // push that is not a Matrix notification has to be told apart from one that is.
            let text = String::from_utf8_lossy(&body).into_owned();
            let target = notification_target(&text);
            sink.emit(event(
                "push.message",
                json!({
                    "decrypted": decrypted,
                    "roomId": target.as_ref().map(|(room, _)| room.clone()),
                    "eventId": target.as_ref().map(|(_, id)| id.clone()),
                }),
            ));
        }
        PushEvent::Unregistered { .. } => {
            sink.emit(event("push.state", json!({ "state": "off" })));
        }
        PushEvent::Abort => {
            sink.emit(event(
                "push.state",
                json!({ "state": "unavailable", "error": "the distributor went away" }),
            ));
        }
    }
}

/// Adapter from the crate's `EventSink` to a plain channel, so the handling can
/// happen off both executors.
struct EventChannel(StdSender<PushEvent>);

#[async_trait::async_trait]
impl EventSink for EventChannel {
    async fn emit(&self, push: PushEvent) {
        let _ = self.0.send(push);
    }
}

/// Registration, keys and distributor in one JSON file: the keys go stale the
/// moment the process restarts. Under the app's own directory - the rest is tmpfs.
struct FileStorage {
    path: PathBuf,
    state: StoredPush,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct StoredPush {
    distributor: Option<String>,
    #[serde(default)]
    distributor_ack: bool,
    #[serde(default)]
    registrations: Vec<TokenInstance>,
    /// Subscription keys, by instance.
    #[serde(default)]
    keys: std::collections::BTreeMap<String, String>,
}

impl FileStorage {
    fn open(path: PathBuf) -> Self {
        let state = std::fs::read(&path)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default();
        FileStorage { path, state }
    }

    /// 0600 throughout: the registration is a bearer credential. Not under the
    /// store key - the woken process has no key after a reboot.
    fn save(&self) {
        let Ok(raw) = serde_json::to_vec_pretty(&self.state) else {
            return;
        };
        let _ = crate::session::write_private(&self.path, &raw);
    }
}

#[async_trait::async_trait]
impl UnifiedPushStorage for FileStorage {
    async fn distributor_get(&self) -> Option<Distributor> {
        self.state.distributor.as_ref().map(|name| Distributor {
            name: name.clone(),
            ack: self.state.distributor_ack,
        })
    }

    async fn distributor_set(&mut self, distributor: String) {
        self.state.distributor = Some(distributor);
        self.state.distributor_ack = false;
        self.save();
    }

    async fn distributor_ack(&mut self) {
        self.state.distributor_ack = true;
        self.save();
    }

    async fn distributor_remove(&mut self) {
        self.state.distributor = None;
        self.state.distributor_ack = false;
        self.save();
    }

    async fn key_get(&self, instance: &str) -> Option<String> {
        self.state.keys.get(instance).cloned()
    }

    async fn key_set(&mut self, instance: &str, key: &str) {
        self.state.keys.insert(instance.to_owned(), key.to_owned());
        self.save();
    }

    async fn key_remove(&mut self, instance: &str) {
        self.state.keys.remove(instance);
        self.save();
    }

    async fn key_remove_all(&mut self) {
        self.state.keys.clear();
        self.save();
    }

    async fn registration_get_from_instance(&self, instance: &str) -> Option<TokenInstance> {
        self.state
            .registrations
            .iter()
            .find(|entry| entry.instance == instance)
            .cloned()
    }

    async fn registration_get_from_token(&self, token: &str) -> Option<TokenInstance> {
        self.state
            .registrations
            .iter()
            .find(|entry| entry.token == token)
            .cloned()
    }

    async fn registration_save(&mut self, token: &TokenInstance) {
        self.state
            .registrations
            .retain(|entry| entry.instance != token.instance);
        self.state.registrations.push(token.clone());
        self.save();
    }

    async fn registration_list(&self) -> Vec<TokenInstance> {
        self.state.registrations.clone()
    }

    async fn registration_remove(&mut self, instance: &str) -> bool {
        let before = self.state.registrations.len();
        self.state
            .registrations
            .retain(|entry| entry.instance != instance);
        self.state.keys.remove(instance);
        let removed = self.state.registrations.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    async fn registration_remove_all(&mut self) {
        self.state.registrations.clear();
        self.state.keys.clear();
        self.save();
    }
}

/// What a Matrix push carries. `event_id_only`, so it names room and event and
/// nothing else; every field is optional because a gateway may reshape it.
pub fn notification_target(body: &str) -> Option<(String, String)> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let notification = parsed.get("notification")?;
    let room = notification.get("room_id")?.as_str()?.to_owned();
    let event = notification.get("event_id")?.as_str()?.to_owned();
    Some((room, event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matrix_notification_names_its_room_and_event() {
        let body = r#"{"notification":{"event_id":"$abc","room_id":"!room:example.org",
                        "counts":{"unread":1}}}"#;
        assert_eq!(
            notification_target(body),
            Some(("!room:example.org".to_owned(), "$abc".to_owned()))
        );
    }

    #[test]
    fn anything_else_is_not_ours() {
        // A push meant for another app, a reshaped body, or the distributor's test
        // message. None of these may look like a message to fetch.
        for probe in [
            "",
            "not json at all",
            r#"{"notification":{}}"#,
            r#"{"notification":{"room_id":"!room:example.org"}}"#,
            r#"{"hello":"world"}"#,
        ] {
            assert_eq!(notification_target(probe), None, "accepted: {probe}");
        }
    }
}

/// True for a gateway this app will hand to the homeserver. https only: the
/// server posts room and event ids to it for every notification.
pub fn gateway_is_sound(gateway: &str) -> bool {
    let gateway = gateway.trim();
    gateway.starts_with("https://") && gateway.len() > "https://".len()
}

/// Registers the endpoint as an HTTP pusher. The endpoint is the pushkey and
/// the gateway the URL: a homeserver requires the URL to end in `/notify`.
pub async fn set_pusher(
    client: &matrix_sdk::Client,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    gateway: &str,
) -> Result<(), String> {
    use matrix_sdk::ruma::api::client::push::{Pusher, PusherIds, PusherInit, PusherKind};
    use matrix_sdk::ruma::push::{HttpPusherData, PushFormat};

    if !gateway_is_sound(gateway) {
        return Err("the push gateway has to be an https address".to_owned());
    }

    let mut data = HttpPusherData::new(gateway.to_owned());
    data.format = Some(PushFormat::EventIdOnly);
    if !p256dh.is_empty() {
        data.data
            .insert("p256dh".to_owned(), serde_json::json!(p256dh));
    }
    if !auth.is_empty() {
        data.data.insert("auth".to_owned(), serde_json::json!(auth));
    }

    let pusher: Pusher = PusherInit {
        ids: PusherIds::new(endpoint.to_owned(), APP_ID.to_owned()),
        kind: PusherKind::Http(data),
        app_display_name: "xmatic".to_owned(),
        // Deliberately not the device's own name: it shows in every other client's
        // session list, and this app does not put it where nobody asked.
        device_display_name: "xmatic".to_owned(),
        profile_tag: None,
        lang: "en".to_owned(),
    }
    .into();

    // `append: false` drops any other pusher with the same pushkey - one endpoint
    // per device, and a second registration means two of every notification.
    client
        .pusher()
        .set(pusher, false)
        .await
        .map_err(|error| crate::text::scrub_ids(&format!("could not register the pusher: {error}")))
}

/// Removes the pusher again, so the homeserver stops posting to an endpoint
/// that no longer exists.
pub async fn clear_pusher(client: &matrix_sdk::Client, endpoint: &str) -> Result<(), String> {
    use matrix_sdk::ruma::api::client::push::PusherIds;

    client
        .pusher()
        .delete(PusherIds::new(endpoint.to_owned(), APP_ID.to_owned()))
        .await
        .map(|_| ())
        .map_err(|error| crate::text::scrub_ids(&format!("could not remove the pusher: {error}")))
}

/// Removes every pusher this app registered. The endpoint is never persisted,
/// so a registration that outlived it can only be found on the server.
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

/// The identifier the homeserver files this pusher under. It has to stay put:
/// changing it leaves the old pusher pointing at a dead endpoint.
const APP_ID: &str = "org.xmatic.xmatic";

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
        // Encrypted here means it could not be decrypted: the keys for it never
        // reached this device. The banner says so rather than counting it.
        Some(AnyMessageLikeEventContent::RoomEncrypted(_)) | None => ("encrypted", String::new()),
        _ => ("", String::new()),
    }
}
