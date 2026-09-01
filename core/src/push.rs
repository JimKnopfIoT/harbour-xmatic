//! Push notifications over UnifiedPush.
//!
//! This app has no background service by decision, so nothing arrives while it
//! is closed. UnifiedPush does not change that decision: the *distributor* is
//! the daemon, one per device and shared by every app on it, and this module is
//! only the connector — the part that asks the distributor for an endpoint,
//! hands that endpoint to the homeserver as a pusher, and takes delivery.
//!
//! Three things about the shape of this, each of which cost somebody a day
//! before it was written down.
//!
//! **The D-Bus connection may not be driven by tokio.** The `unifiedpush`
//! crate's handlers call `Handle::block_on`, which is legal only because zbus
//! dispatches them on its async-io executor. Polled from a tokio worker they
//! panic with "Cannot start a runtime from within a runtime". So the connection
//! lives on a thread of its own under `zbus::block_on`, and the tokio handle is
//! only handed over, never used to drive it. Cargo unifies features across the
//! crate graph, so a single dependency enabling `zbus/tokio` would move the
//! executor and break this from a distance — checked with
//! `cargo tree -e features | grep 'zbus feature'`, which must not list `tokio`.
//!
//! **Nothing registers until the user asks.** A connector that registers at
//! startup subscribes people who never turned the feature on, and the endpoint
//! it gets is a secret that then exists for no reason.
//!
//! **The payload is not necessarily encrypted.** Web Push says it is, and the
//! crate will decrypt where it can — but a Matrix push gateway relays the
//! homeserver's plain JSON, and the distributor passes what it gets through
//! untouched. Both cases are handled; assuming ciphertext would drop every
//! message that actually arrives.

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

/// The bus name this connector owns.
///
/// Not `org.xmatic.xmatic`: Sailjail grants an application exactly one name,
/// that one is already spent on the share activation, and claiming a child of
/// it comes back as `ServiceUnknown`. The `org.unifiedpush.Connector.*`
/// namespace is granted by the `UnifiedPush` permission for this reason.
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

/// Starts the connector. Returns `None` where the session bus or the D-Bus
/// name cannot be had — an ordinary outcome on a device with no distributor
/// installed, and one the UI has to be able to say out loud rather than look
/// broken over.
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
                        // The usual cause is the Sailjail permission being
                        // absent because no distributor package is installed,
                        // and D-Bus reports that as `ServiceUnknown` rather
                        // than as a permission error.
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
                        // Before every register, not once: unregistering the
                        // last instance makes the crate forget the distributor,
                        // and `register` then returns having silently done
                        // nothing. And `try_use_default_distributor` is not an
                        // availability check — it is true only for exactly one
                        // candidate, so two distributors look like none.
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
                        sink.emit(event("push.state", json!({ "state": "registering" })));
                        unifiedpush
                            .register(INSTANCE, Some("xmatic"), None)
                            .await;
                    }
                    Command::Disable => {
                        unifiedpush.unregister(INSTANCE).await;
                        sink.emit(event("push.state", json!({ "state": "off" })));
                    }
                    Command::Status => report(&sink, &unifiedpush).await,
                }
            }
        });
    });

    PushHandle { commands }
}

/// What is on the device and what this app holds, as one object.
async fn report(sink: &Arc<Sink>, unifiedpush: &UnifiedPush) {
    let distributors = unifiedpush.list_distributors().await;
    let chosen = unifiedpush.get_distributor().await;
    sink.emit(event(
        "push.state",
        json!({
            "state": if distributors.is_empty() { "no-distributor" } else { "idle" },
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

/// One event from the distributor, as JSON for the bridge.
fn forward(sink: &Arc<Sink>, push: PushEvent) {
    match push {
        PushEvent::NewEndpoint { endpoint, .. } => {
            // All three, because a push server needs the keys as well as the
            // URL: Web Push wants an encrypted body, and a gateway given only
            // the URL cannot make one. Whether the gateway uses them is its
            // business; withholding them would decide that for it.
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
            // Both shapes. The distributor relays what it was handed, and a
            // Matrix gateway hands it the homeserver's plain JSON — so the
            // undecrypted case is not an error here, it is the normal one.
            let (body, decrypted) = match message {
                PushMessage::Decrypted { content } => (content, true),
                PushMessage::Raw { content } => (content, false),
            };
            // Parsed here rather than in the bridge: it is a pure function
            // with tests, and a push that is not a Matrix notification —
            // another app's, a gateway that reshaped it, the distributor's own
            // test message — must be told apart from one that is.
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

/// The registration, the keys and the chosen distributor, in one JSON file.
///
/// Persisting matters more than it looks: stubbing the key methods out compiles
/// and registers cleanly, and then every push fails, because the keys handed to
/// the push server go stale the moment the process restarts.
///
/// Written temp-and-rename, and under the app's own data directory — Sailjail
/// presents the rest of `~/.local/share` as a tmpfs that firejail discards on
/// exit, so only the per-app directory survives.
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

    fn save(&self) {
        let Ok(raw) = serde_json::to_vec_pretty(&self.state) else {
            return;
        };
        let temporary = self.path.with_extension("tmp");
        if std::fs::write(&temporary, &raw).is_ok() {
            let _ = std::fs::rename(&temporary, &self.path);
        }
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

/// The notification a Matrix push carries, as far as this app needs it.
///
/// `event_id_only` is what the pusher asks for, so the body names the room and
/// the event and nothing else — the message itself is fetched and decrypted
/// here. Everything is optional because the gateway between the homeserver and
/// the device may reshape it, and a missing field must not cost the whole push.
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
        // A push meant for another app, a gateway that reshaped the body, or
        // the distributor's own test message. None of these may look like a
        // message to fetch.
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

/// Registers the endpoint with the homeserver as an HTTP pusher, or removes it.
///
/// The endpoint is the `pushkey` and the gateway is the URL the homeserver
/// posts to. That split is not ours: a homeserver requires the pusher URL to
/// end in `/_matrix/push/v1/notify`, so the endpoint cannot be the URL and has
/// to travel as the key the gateway forwards to.
///
/// `format: event_id_only` on purpose. The push then names the room and the
/// event and carries no content — which matters twice over here: an encrypted
/// room's content would be useless to the gateway anyway, and everything on
/// that path (the gateway, the push service) sees less.
///
/// The subscription keys ride along in the pusher's free-form data. Whether a
/// gateway uses them is its business; a gateway that encrypts cannot work
/// without them, and leaving them out would decide that question for it.
pub async fn set_pusher(
    client: &matrix_sdk::Client,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    gateway: &str,
) -> Result<(), String> {
    use matrix_sdk::ruma::api::client::push::{Pusher, PusherIds, PusherInit, PusherKind};
    use matrix_sdk::ruma::push::{HttpPusherData, PushFormat};

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
        // Deliberately not the device's own name: it is shown in every other
        // client's session list, and this app does not put the user's device
        // name into places they did not ask for.
        device_display_name: "xmatic".to_owned(),
        profile_tag: None,
        lang: "en".to_owned(),
    }
    .into();

    // `append: false`, which tells the server to drop any other pusher with the
    // same pushkey and app id. There is one endpoint per device here, and a
    // second registration for the same one would only mean two copies of every
    // notification.
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

/// The reverse-DNS identifier the homeserver files this pusher under. One per
/// application, and it has to stay put: changing it leaves the old pusher on
/// the server pointing at a dead endpoint.
const APP_ID: &str = "org.xmatic.xmatic";

/// Fetches the message a push named, and returns what a banner needs.
///
/// The push carries a room and an event id and nothing else — that is what
/// `event_id_only` buys: the gateway and the push service never see a word of
/// the conversation. The text is fetched and decrypted here, on the device that
/// holds the keys.
///
/// `NotificationClient` is the SDK's own answer to exactly this. It reaches an
/// event that no sync has brought in yet, applies the account's push rules, and
/// hands back the room's display name and the sender's along with it.
///
/// `MultipleProcesses`, because that is what this is: the app may be running
/// while a second, woken process asks. The setting decides how the crypto
/// store's cross-process lock behaves, and claiming a single process here where
/// there may be two is the way to a corrupted store.
pub async fn notification_for(
    client: &matrix_sdk::Client,
    room_id: &str,
    event_id: &str,
) -> Result<Value, String> {
    use matrix_sdk::ruma::{EventId, RoomId};
    use matrix_sdk_ui::notification_client::{
        NotificationClient, NotificationProcessSetup, NotificationStatus,
    };

    let room = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let event = EventId::parse(event_id).map_err(|_| "not an event identifier".to_owned())?;

    let notifications =
        NotificationClient::new(client.clone(), NotificationProcessSetup::MultipleProcesses)
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
                // The push rules decided this was worth a sound. Passed on
                // rather than judged again here: the account's own rules are
                // the answer, and a second opinion would only disagree.
                "noisy": item.is_noisy.unwrap_or(false),
                "mention": item.has_mention.unwrap_or(false),
            }))
        }
        // Each of these is a real answer, not a failure: the push rules say
        // this one is not to be shown, the event is gone, or the server never
        // had it. Silence is the correct outcome and the caller must not turn
        // it into a banner.
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
