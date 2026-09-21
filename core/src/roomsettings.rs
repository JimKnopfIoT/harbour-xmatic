//! A room's name, topic and picture. State events; a bridge may pass them on.

use std::sync::Arc;
use std::time::Duration;

use matrix_sdk::config::RequestConfig;
use matrix_sdk::ruma::api::client::state::get_state_events;
use matrix_sdk::ruma::RoomId;
use matrix_sdk::{Client, Room, RoomState};
use serde_json::{json, Value};

use crate::protocol::{reply_error, reply_ok, Command};
use crate::runtime::Sink;
use crate::text::{scrub_ids, strip_bidi};

/// Bytes, as the spec long required.
const NAME_BYTES: usize = 255;

const TOPIC_BYTES: usize = 16 * 1024;

/// Wait for the sync to bring the change back; past it the list's diff does.
const SETTLE: Duration = Duration::from_secs(10);

const STATE_TIMEOUT: Duration = Duration::from_secs(20);

const BRIDGE_TYPES: [&str; 2] = ["m.bridge", "uk.half-shot.bridge"];

pub async fn handle(command: Command, client: Option<Client>, sink: &Arc<Sink>) {
    let id = command.id();
    let Some(client) = client else {
        sink.emit(reply_error(id, "not signed in"));
        return;
    };
    let outcome = match command {
        Command::RoomSettingsLoad { room_id, .. } => load(&client, &room_id).await,
        Command::RoomSettingsSetName { room_id, name, .. } => {
            set_name(&client, &room_id, &name).await
        }
        Command::RoomSettingsSetTopic { room_id, topic, .. } => {
            set_topic(&client, &room_id, &topic).await
        }
        Command::RoomSettingsSetAvatar { room_id, path, .. } => {
            set_avatar(&client, &room_id, &path).await
        }
        Command::RoomSettingsRemoveAvatar { room_id, .. } => {
            remove_avatar(&client, &room_id).await
        }
        _ => Err("not a room settings command".to_owned()),
    };
    match outcome {
        Ok(data) => sink.emit(reply_ok(id, data)),
        Err(message) => sink.emit(reply_error(id, message)),
    }
}

fn known_room(client: &Client, room_id: &str) -> Result<Room, String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())
}

fn joined_room(client: &Client, room_id: &str) -> Result<Room, String> {
    let room = known_room(client, room_id)?;
    if room.state() != RoomState::Joined {
        return Err("only a joined room can be changed".to_owned());
    }
    Ok(room)
}

/// The name as set, not the display name: saving must not freeze the fallback.
async fn load(client: &Client, room_id: &str) -> Result<Value, String> {
    let room = known_room(client, room_id)?;
    let joined = room.state() == RoomState::Joined;
    let can = if joined {
        crate::members::room_permissions(client, &room).await
    } else {
        json!({})
    };
    let direct = room.is_direct().await.unwrap_or(false);

    // Not in `required_state`, so not in the store.
    let bridge = match announced_bridges(client, &room).await {
        Some(announced) => bridge_from(&announced, direct).unwrap_or(Value::Null),
        None => json!({ "unknown": true }),
    };

    Ok(json!({
        "roomId": room_id,
        "joined": joined,
        "name": room.name().unwrap_or_default(),
        "topic": room.topic().unwrap_or_default(),
        "avatar": room.avatar_url().map(|url| url.to_string()),
        "can": {
            "name": joined && can.get("name") == Some(&Value::Bool(true)),
            "topic": joined && can.get("topic") == Some(&Value::Bool(true)),
            "avatar": joined && can.get("avatar") == Some(&Value::Bool(true)),
        },
        "direct": direct,
        "bridge": bridge,
    }))
}

async fn announced_bridges(client: &Client, room: &Room) -> Option<Vec<Value>> {
    let request = get_state_events::v3::Request::new(room.room_id().to_owned());
    let config = RequestConfig::new().disable_retry().timeout(STATE_TIMEOUT);
    let response = client.send(request).with_request_config(config).await.ok()?;
    Some(
        response
            .room_state
            .iter()
            .filter_map(|raw| serde_json::from_str::<Value>(raw.json().get()).ok())
            .filter(|event| {
                event.get("type").and_then(Value::as_str).is_some_and(|kind| BRIDGE_TYPES.contains(&kind))
            })
            .filter_map(|event| event.get("content").cloned())
            .collect(),
    )
}

/// `com.beeper.room_type` beats the account's `direct`. Empty content: bridge gone.
fn bridge_from(contents: &[Value], direct: bool) -> Option<Value> {
    let content = contents
        .iter()
        .find(|content| content.as_object().is_some_and(|object| !object.is_empty()))?;
    let protocol = content.get("protocol");
    let field = |key: &str| {
        protocol
            .and_then(|protocol| protocol.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(|text| text.chars().take(64).collect::<String>())
    };
    let network = field("displayname").or_else(|| field("id")).unwrap_or_default();
    let room_type = content.get("com.beeper.room_type").and_then(Value::as_str);
    let bridged_direct = match room_type {
        Some("dm") => true,
        Some(_) => false,
        None => direct,
    };
    Some(json!({ "network": network, "direct": bridged_direct }))
}

fn name_rule(name: &str) -> Result<String, String> {
    let line = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.len() > NAME_BYTES {
        return Err("the name is too long".to_owned());
    }
    Ok(line)
}

fn topic_rule(topic: &str) -> Result<String, String> {
    let trimmed = topic.trim();
    if trimmed.len() > TOPIC_BYTES {
        return Err("the topic is too long".to_owned());
    }
    Ok(trimmed.to_owned())
}

async fn set_name(client: &Client, room_id: &str, name: &str) -> Result<Value, String> {
    let room = joined_room(client, room_id)?;
    let name = name_rule(name)?;
    room.set_name(name.clone())
        .await
        .map_err(|error| format!("could not change the name: {}", scrub_ids(&error.to_string())))?;
    let settled = settle(&room, |room| room.name().unwrap_or_default() == name).await;
    Ok(json!({ "roomId": room_id, "field": "name", "name": name, "row": row(&room, settled).await }))
}

async fn set_topic(client: &Client, room_id: &str, topic: &str) -> Result<Value, String> {
    let room = joined_room(client, room_id)?;
    let topic = topic_rule(topic)?;
    room.set_room_topic(&topic).await.map_err(|error| {
        format!("could not change the topic: {}", scrub_ids(&error.to_string()))
    })?;
    Ok(json!({ "roomId": room_id, "field": "topic", "topic": topic }))
}

async fn set_avatar(client: &Client, room_id: &str, path: &str) -> Result<Value, String> {
    let room = joined_room(client, room_id)?;
    let (mime, data) = crate::profile::read_picture(path).await?;
    let failed =
        |error: matrix_sdk::Error| format!("could not change the picture: {}", scrub_ids(&error.to_string()));
    // Not `upload_avatar`: it keeps the URL to itself.
    let uploaded = client
        .media()
        .upload(&mime, data, None)
        .await
        .map_err(|error| failed(error.into()))?;
    let mut info = matrix_sdk::ruma::events::room::avatar::ImageInfo::new();
    info.mimetype = Some(mime.to_string());
    info.blurhash = uploaded.blurhash;
    room.set_avatar_url(&uploaded.content_uri, Some(info)).await.map_err(failed)?;
    let url = uploaded.content_uri;
    let settled = settle(&room, |room| room.avatar_url().as_ref() == Some(&url)).await;
    Ok(json!({
        "roomId": room_id,
        "field": "avatar",
        "avatar": url.to_string(),
        "row": row(&room, settled).await,
    }))
}

async fn remove_avatar(client: &Client, room_id: &str) -> Result<Value, String> {
    let room = joined_room(client, room_id)?;
    room.remove_avatar().await.map_err(|error| {
        format!("could not remove the picture: {}", scrub_ids(&error.to_string()))
    })?;
    let settled = settle(&room, |room| room.avatar_url().is_none()).await;
    Ok(json!({ "roomId": room_id, "field": "avatar", "avatar": null, "row": row(&room, settled).await }))
}

async fn settle(room: &Room, done: impl Fn(&Room) -> bool) -> bool {
    let mut info = room.subscribe_info();
    if done(room) {
        return true;
    }
    tokio::time::timeout(SETTLE, async {
        while info.next().await.is_some() {
            if done(room) {
                return;
            }
        }
    })
    .await
    .is_ok()
        && done(room)
}

async fn row(room: &Room, settled: bool) -> Value {
    if !settled {
        return Value::Null;
    }
    let name = match room.display_name().await {
        Ok(name) => strip_bidi(&name.to_string()),
        Err(_) => return Value::Null,
    };
    json!({ "name": name, "avatar": crate::roomlist::room_avatar(room) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_one_line() {
        assert_eq!(name_rule("  Family\n chat \t ").unwrap(), "Family chat");
        assert_eq!(name_rule("   ").unwrap(), "");
    }

    #[test]
    fn name_limit_counts_bytes() {
        assert!(name_rule(&"a".repeat(NAME_BYTES)).is_ok());
        assert!(name_rule(&"a".repeat(NAME_BYTES + 1)).is_err());
        assert!(name_rule(&"ä".repeat(128)).is_err());
    }

    #[test]
    fn topic_keeps_lines() {
        assert_eq!(topic_rule("\n one\ntwo \n").unwrap(), "one\ntwo");
        assert!(topic_rule(&"a".repeat(TOPIC_BYTES + 1)).is_err());
    }

    #[test]
    fn no_bridge_without_content() {
        assert!(bridge_from(&[], false).is_none());
        assert!(bridge_from(&[json!({})], true).is_none());
    }

    #[test]
    fn bridge_names_its_network() {
        let content = json!({
            "bridgebot": "@bot:example.org",
            "protocol": { "id": "whatsapp", "displayname": "WhatsApp" },
            "com.beeper.room_type": "dm",
        });
        assert_eq!(
            bridge_from(&[content], false).unwrap(),
            json!({ "network": "WhatsApp", "direct": true })
        );
    }

    #[test]
    fn bridge_room_type_beats_the_account() {
        let group = json!({ "protocol": { "id": "signal" }, "com.beeper.room_type": "group" });
        assert_eq!(
            bridge_from(&[group], true).unwrap(),
            json!({ "network": "signal", "direct": false })
        );
        let silent = json!({ "protocol": { "id": "telegram" } });
        assert_eq!(bridge_from(&[silent], true).unwrap()["direct"], json!(true));
    }
}
