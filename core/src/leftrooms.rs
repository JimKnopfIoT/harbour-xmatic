//! Rooms the account has left or been banned from but not forgotten. The chat
//! list hides them; the server still counts them until they are forgotten.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use matrix_sdk::ruma::api::client::membership::forget_room;
use matrix_sdk::ruma::{OwnedRoomId, RoomId};
use matrix_sdk::{Client, Room, RoomState};
use serde_json::{json, Value};

use crate::protocol::{reply_error, reply_ok, Command};
use crate::runtime::Sink;
use crate::text::{scrub_ids, strip_bidi};

pub async fn handle(command: Command, client: Option<Client>, sink: &Arc<Sink>) {
    let id = command.id();
    let Some(client) = client else {
        sink.emit(reply_error(id, "not signed in"));
        return;
    };
    let outcome = match command {
        Command::LeftRoomsList { .. } => Ok(list(&client)),
        Command::LeftRoomsForget { room_id, .. } => forget(&client, &room_id).await,
        _ => Err("not a left-rooms command".to_owned()),
    };
    match outcome {
        Ok(data) => sink.emit(reply_ok(id, data)),
        Err(message) => sink.emit(reply_error(id, message)),
    }
}

/// Forgotten in this process: the client's own room map keeps them until restart.
static FORGOTTEN: Mutex<Option<HashSet<OwnedRoomId>>> = Mutex::new(None);

/// Clears the set, or the next account would hide rooms of the same id.
pub fn forget_session() {
    if let Ok(mut guard) = FORGOTTEN.lock() {
        *guard = None;
    }
}

pub(crate) fn was_forgotten(room_id: &RoomId) -> bool {
    FORGOTTEN
        .lock()
        .map(|guard| guard.as_ref().is_some_and(|set| set.contains(room_id)))
        .unwrap_or(false)
}

/// Rooms that are joined, invited or knocked again lose their forgotten mark, so
/// a later leave lists them again. Called on every membership change.
pub(crate) async fn unforget_returned(client: &Client) {
    let returned: Vec<OwnedRoomId> = match FORGOTTEN.lock() {
        Ok(mut guard) => match guard.as_mut() {
            Some(set) => {
                let back: Vec<OwnedRoomId> = set
                    .iter()
                    .filter(|id| client.get_room(id).is_some_and(|room| returned_to(room.state())))
                    .cloned()
                    .collect();
                for id in &back {
                    set.remove(id);
                }
                back
            }
            None => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    if returned.is_empty() {
        return;
    }
    // Its cache is live again; purging it at the next start would only cost history.
    update_pending_purge(client, |pending| {
        pending.retain(|id| !returned.iter().any(|back| back.as_str() == id))
    })
    .await;
}

fn returned_to(state: RoomState) -> bool {
    !matches!(state, RoomState::Left | RoomState::Banned)
}

/// State store key: forgotten rooms whose event cache is still on disk.
const PURGE_KEY: &[u8] = b"xmatic.forgotten_event_cache";

/// One read-modify-write of the purge list at a time in this process.
static PURGE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn update_pending_purge(client: &Client, change: impl FnOnce(&mut Vec<String>)) {
    let _serial = PURGE_LOCK.lock().await;
    let mut pending = pending_purge(client).await;
    change(&mut pending);
    store_pending_purge(client, &pending).await;
}

async fn pending_purge(client: &Client) -> Vec<String> {
    match client.state_store().get_custom_value(PURGE_KEY).await {
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).unwrap_or_default(),
        _ => Vec::new(),
    }
}

async fn store_pending_purge(client: &Client, rooms: &[String]) {
    let store = client.state_store();
    let _ = if rooms.is_empty() {
        store.remove_custom_value(PURGE_KEY).await.map(|_| ())
    } else {
        match serde_json::to_vec(rooms) {
            Ok(bytes) => store.set_custom_value(PURGE_KEY, bytes).await.map(|_| ()),
            Err(_) => return,
        }
    };
}

/// Adds a room to the purge list, once.
fn add_room(rooms: &mut Vec<String>, room_id: &str) {
    if !rooms.iter().any(|id| id == room_id) {
        rooms.push(room_id.to_owned());
    }
}

/// Drops the event cache of rooms forgotten in an earlier session. Only on a
/// client nothing has used yet: `clear_all_events` under a loaded room cache
/// desyncs it, and a new `EventCache` starts with no room loaded.
pub(crate) async fn purge_forgotten_caches(client: &Client) {
    let _serial = PURGE_LOCK.lock().await;
    let pending = pending_purge(client).await;
    if pending.is_empty() {
        return;
    }
    let Ok(lock) = client.event_cache_store().lock().await else {
        return;
    };
    let store = match lock {
        matrix_sdk_base::event_cache::store::EventCacheStoreLockState::Clean(store)
        | matrix_sdk_base::event_cache::store::EventCacheStoreLockState::Dirty(store) => store,
    };
    let mut left = Vec::new();
    for room_id in pending {
        let cleared = match RoomId::parse(&room_id) {
            Ok(parsed) => store.clear_all_events(Some(&parsed)).await.is_ok(),
            Err(_) => true,
        };
        if !cleared {
            left.push(room_id);
        }
    }
    drop(store);
    store_pending_purge(client, &left).await;
}

fn is_left(room: &Room) -> bool {
    matches!(room.state(), RoomState::Left | RoomState::Banned) && !was_forgotten(room.room_id())
}

/// `Room::forget` without its event-cache step, which reloads every loaded room
/// with `ForgetAll` and so empties all of them, on disk too.
pub async fn forget_room(room: &Room) -> Result<(), String> {
    if !matches!(room.state(), RoomState::Left | RoomState::Banned) {
        return Err("only a left room can be forgotten".to_owned());
    }
    room.client()
        .send(forget_room::v3::Request::new(room.room_id().to_owned()))
        .await
        .map_err(|error| scrub_ids(&format!("could not forget the room: {error}")))?;
    // Cleanup, as in the SDK: a stale m.direct entry costs nothing.
    if room.is_direct().await.unwrap_or(false) {
        let _ = room.set_is_direct(false).await;
    }
    let _ = room.client().state_store().remove_room(room.room_id()).await;
    // Its events stay in the event cache until the next start purges them there.
    update_pending_purge(&room.client(), |pending| add_room(pending, room.room_id().as_str())).await;
    if let Ok(mut guard) = FORGOTTEN.lock() {
        guard.get_or_insert_with(HashSet::new).insert(room.room_id().to_owned());
    }
    // No diff follows: the room stays in the client's map.
    crate::roomcount::rooms_changed();
    Ok(())
}

/// How many there are, for the reply to a forget.
pub fn count(client: &Client) -> usize {
    client.rooms().iter().filter(|room| is_left(room)).count()
}

fn list(client: &Client) -> Value {
    let rooms: Vec<Value> = client
        .rooms()
        .iter()
        .filter(|room| is_left(room))
        .map(|room| {
            // Empty where no sync named it: the page says so instead of an id.
            let name = room
                .cached_display_name()
                .map(|name| strip_bidi(&name.to_string()))
                .unwrap_or_default();
            json!({
                "id": room.room_id().as_str(),
                "name": name,
                "banned": room.state() == RoomState::Banned,
            })
        })
        .collect();
    json!({ "rooms": rooms })
}

async fn forget(client: &Client, room_id: &str) -> Result<Value, String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())?;
    forget_room(&room).await?;
    Ok(json!({ "roomId": room_id, "left": count(client) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_room_is_listed_for_purging_once() {
        let mut rooms = Vec::new();
        add_room(&mut rooms, "!a:x");
        add_room(&mut rooms, "!a:x");
        add_room(&mut rooms, "!b:x");
        assert_eq!(rooms, vec!["!a:x".to_owned(), "!b:x".to_owned()]);
    }

    #[test]
    fn only_a_room_the_account_is_back_in_loses_its_mark() {
        assert!(returned_to(RoomState::Joined));
        assert!(returned_to(RoomState::Invited));
        assert!(!returned_to(RoomState::Left));
        assert!(!returned_to(RoomState::Banned));
    }
}
