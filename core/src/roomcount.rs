//! How many rooms there are, in the counts the chat list needs: what the
//! server's sliding-sync list holds, what was left, what is joined.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use futures_util::{FutureExt, StreamExt};
use matrix_sdk::config::RequestConfig;
use matrix_sdk::ruma::api::client::membership::joined_rooms;
use matrix_sdk::ruma::events::room::member::MembershipState;
use matrix_sdk::ruma::OwnedRoomId;
use matrix_sdk::{Client, Room, RoomState};
use matrix_sdk_base::{RoomInfoNotableUpdate, RoomInfoNotableUpdateReasons};
use serde_json::json;
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use tokio::sync::broadcast::Receiver;
use tokio::sync::{watch, Notify};
use tokio::task::JoinHandle;

use crate::protocol::event;
use crate::runtime::Sink;

/// Whether the server's list counts a room. The reference homeserver drops a room
/// left by the user; kicks and bans stay until forgotten. Unknown sender counts.
pub(crate) fn listed_by_server(state: RoomState, forgotten: bool, left_by_self: Option<bool>) -> bool {
    match state {
        RoomState::Joined | RoomState::Invited | RoomState::Knocked => true,
        RoomState::Banned => !forgotten,
        RoomState::Left => !forgotten && left_by_self != Some(true),
    }
}

/// The three numbers, in one pass over the client's rooms.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Census {
    pub listed: usize,
    pub left: usize,
    pub joined: usize,
}

/// Wakes the census where no room diff and no notable update follows: a room
/// forgotten here stays in the client's map.
static CHANGED: Notify = Notify::const_new();

pub(crate) fn rooms_changed() {
    CHANGED.notify_one();
}

/// Bumped when a census finds other counts or another room shape, so filtered
/// lists can recount what matches.
static GENERATION: LazyLock<watch::Sender<u64>> = LazyLock::new(|| watch::channel(0).0);

/// Wakes collected after the first one, so a sync burst costs one census.
const SETTLE: Duration = Duration::from_millis(200);

/// Session end: a pending wake and the count belong to the account.
pub(crate) fn forget() {
    let _ = CHANGED.notified().now_or_never();
    GENERATION.send_replace(0);
}

pub(crate) fn subscribe() -> watch::Receiver<u64> {
    GENERATION.subscribe()
}

/// Whether the own leave was by the user. Read from the store, so cached per room
/// until its membership changes again.
async fn left_by_self(room: &Room) -> Option<bool> {
    let own = room.own_user_id().to_owned();
    // Store only: `get_member` would fetch the member list for a room we left.
    match room.get_member_no_sync(&own).await {
        Ok(Some(member)) => Some(
            *member.membership() == MembershipState::Leave && member.event().sender() == own,
        ),
        _ => None,
    }
}

/// What the list filters read of one room. Order-free, so a re-sorted map
/// gives the same value.
fn room_shape(room: &Room, forgotten: bool) -> u64 {
    let mut hasher = DefaultHasher::new();
    room.room_id().hash(&mut hasher);
    let state: u8 = match room.state() {
        RoomState::Joined => 0,
        RoomState::Left => 1,
        RoomState::Invited => 2,
        RoomState::Knocked => 3,
        RoomState::Banned => 4,
    };
    state.hash(&mut hasher);
    forgotten.hash(&mut hasher);
    room.is_space().hash(&mut hasher);
    room.is_favourite().hash(&mut hasher);
    room.cached_display_name().map(|name| name.to_string()).hash(&mut hasher);
    hasher.finish()
}

/// Whether the filtered lists have to recount: counts or room shape moved.
fn needs_recount(previous: Option<(Census, u64)>, now: (Census, u64)) -> bool {
    previous != Some(now)
}

/// A membership moved: the own leave is read again, a return may lift a forgotten mark.
const MEMBERSHIP: RoomInfoNotableUpdateReasons =
    RoomInfoNotableUpdateReasons::MEMBERSHIP.union(RoomInfoNotableUpdateReasons::NONE);
/// What wakes the census: memberships, and the name the filters match on. Not the
/// recency stamp: it moves with every message and changes no count.
const RELEVANT: RoomInfoNotableUpdateReasons =
    MEMBERSHIP.union(RoomInfoNotableUpdateReasons::DISPLAY_NAME);

/// What queued up behind the first wake.
#[derive(Debug, Default, PartialEq, Eq)]
struct Drained {
    /// The client is gone.
    closed: bool,
    /// A membership moved, or updates were missed.
    membership: bool,
}

fn drain_updates(
    updates: &mut Receiver<RoomInfoNotableUpdate>,
    cache: &mut HashMap<OwnedRoomId, Option<bool>>,
) -> Drained {
    let mut drained = Drained::default();
    loop {
        match updates.try_recv() {
            Ok(update) => {
                if update.reasons.intersects(MEMBERSHIP) {
                    cache.remove(&update.room_id);
                    drained.membership = true;
                }
            }
            Err(TryRecvError::Empty) => return drained,
            Err(TryRecvError::Lagged(_)) => {
                cache.clear();
                drained.membership = true;
            }
            Err(TryRecvError::Closed) => {
                drained.closed = true;
                return drained;
            }
        }
    }
}

async fn take_census(client: &Client, cache: &mut HashMap<OwnedRoomId, Option<bool>>) -> (Census, u64) {
    let mut census = Census::default();
    let mut shape = 0u64;
    for room in client.rooms() {
        let state = room.state();
        let forgotten = crate::leftrooms::was_forgotten(room.room_id());
        let by_self = if state == RoomState::Left && !forgotten {
            match cache.get(room.room_id()) {
                Some(known) => *known,
                None => {
                    let known = left_by_self(&room).await;
                    cache.insert(room.room_id().to_owned(), known);
                    known
                }
            }
        } else {
            None
        };
        if listed_by_server(state, forgotten, by_self) {
            census.listed += 1;
        }
        if matches!(state, RoomState::Left | RoomState::Banned) && !forgotten {
            census.left += 1;
        }
        if state == RoomState::Joined {
            census.joined += 1;
        }
        shape ^= room_shape(&room, forgotten);
    }
    (census, shape)
}

/// Recounts whenever the set of rooms or a membership changes, and reports
/// `roomlist.known` and `leftRooms.count`. A state change brings no room diff.
pub(crate) fn spawn_census(client: Client, sink: Arc<Sink>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let (_, diffs) = client.rooms_stream();
        futures_util::pin_mut!(diffs);
        let mut updates = client.room_info_notable_update_receiver();
        let mut cache = HashMap::new();
        let mut last: Option<(Census, u64)> = None;

        loop {
            let (census, shape) = take_census(&client, &mut cache).await;
            let recount = needs_recount(last, (census, shape));
            let last_census = last.map(|(previous, _)| previous);
            if last_census.map_or(true, |previous| previous.listed != census.listed || previous.joined != census.joined) {
                sink.emit(event(
                    "roomlist.known",
                    json!({ "known": census.listed, "joined": census.joined }),
                ));
            }
            if last_census.map_or(true, |previous| previous.left != census.left) {
                sink.emit(event("leftRooms.count", json!({ "count": census.left })));
            }
            last = Some((census, shape));
            if recount {
                GENERATION.send_modify(|generation| *generation = generation.wrapping_add(1));
            }

            loop {
                tokio::select! {
                    diffs = diffs.next() => {
                        if diffs.is_none() {
                            return;
                        }
                        break;
                    }
                    update = updates.recv() => match update {
                        Ok(update) if update.reasons.intersects(MEMBERSHIP) => {
                            cache.remove(&update.room_id);
                            crate::leftrooms::unforget_returned(&client).await;
                            break;
                        }
                        Ok(update) if update.reasons.intersects(RELEVANT) => break,
                        Ok(_) => continue,
                        Err(RecvError::Lagged(_)) => {
                            cache.clear();
                            crate::leftrooms::unforget_returned(&client).await;
                            break;
                        }
                        Err(RecvError::Closed) => return,
                    },
                    _ = CHANGED.notified() => break,
                }
            }

            tokio::time::sleep(SETTLE).await;
            while let Some(next) = diffs.next().now_or_never() {
                if next.is_none() {
                    return;
                }
            }
            let drained = drain_updates(&mut updates, &mut cache);
            if drained.closed {
                return;
            }
            // A return that arrived during the settle lifts its mark too.
            if drained.membership {
                crate::leftrooms::unforget_returned(&client).await;
            }
        }
    })
}

/// One `/joined_rooms` at a time; a request asked for while one runs is made
/// once after it, so an older answer never lands after a newer one.
#[derive(Default)]
struct JoinedAsk {
    running: bool,
    again: bool,
}

static JOINED_ASK: Mutex<JoinedAsk> = Mutex::new(JoinedAsk { running: false, again: false });

/// Whether to start a request now; otherwise the running one repeats.
fn claim_joined_ask(ask: &mut JoinedAsk) -> bool {
    if ask.running {
        ask.again = true;
        false
    } else {
        ask.running = true;
        ask.again = false;
        true
    }
}

/// Whether the running request has to go once more.
fn finish_joined_ask(ask: &mut JoinedAsk) -> bool {
    if ask.again {
        ask.again = false;
        true
    } else {
        ask.running = false;
        false
    }
}

/// Releases the claim when the task is aborted mid-request.
struct JoinedAskGuard;

impl Drop for JoinedAskGuard {
    fn drop(&mut self) {
        let mut ask = JOINED_ASK.lock().unwrap_or_else(|error| error.into_inner());
        *ask = JoinedAsk::default();
    }
}

/// Rooms the server says the account is in, as `roomlist.joined`. Could not
/// ask: no event, so no claim; the next `Running` asks again.
pub(crate) fn ask_joined_count(client: Client, sink: Arc<Sink>) -> Option<JoinHandle<()>> {
    {
        let mut ask = JOINED_ASK.lock().unwrap_or_else(|error| error.into_inner());
        if !claim_joined_ask(&mut ask) {
            return None;
        }
    }
    // Made before the spawn: a task aborted before its first poll still drops it.
    let guard = JoinedAskGuard;
    Some(tokio::spawn(async move {
        let guard = guard;
        loop {
            let config = RequestConfig::new().disable_retry().timeout(Duration::from_secs(20));
            if let Ok(response) = client
                .send(joined_rooms::v3::Request::new())
                .with_request_config(config)
                .await
            {
                let joined = response.joined_rooms.len();
                sink.emit(event("roomlist.joined", json!({ "joined": joined })));
            }
            let again = {
                let mut ask = JOINED_ASK.lock().unwrap_or_else(|error| error.into_inner());
                finish_joined_ask(&mut ask)
            };
            if !again {
                std::mem::forget(guard);
                return;
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(room: &str, reasons: RoomInfoNotableUpdateReasons) -> RoomInfoNotableUpdate {
        RoomInfoNotableUpdate { room_id: OwnedRoomId::try_from(room).unwrap(), reasons }
    }

    #[test]
    fn a_membership_in_the_settle_is_noted_and_a_rename_wakes() {
        assert!(RELEVANT.contains(RoomInfoNotableUpdateReasons::DISPLAY_NAME));
        assert!(!RELEVANT.intersects(RoomInfoNotableUpdateReasons::RECENCY_STAMP));

        let (sender, mut receiver) = tokio::sync::broadcast::channel(8);
        let mut cache = HashMap::new();
        cache.insert(OwnedRoomId::try_from("!a:x").unwrap(), Some(true));
        sender.send(update("!b:x", RoomInfoNotableUpdateReasons::DISPLAY_NAME)).unwrap();
        assert_eq!(drain_updates(&mut receiver, &mut cache), Drained::default());
        assert_eq!(cache.len(), 1);

        sender.send(update("!a:x", RoomInfoNotableUpdateReasons::MEMBERSHIP)).unwrap();
        let drained = drain_updates(&mut receiver, &mut cache);
        assert!(drained.membership && !drained.closed);
        assert!(cache.is_empty());

        drop(sender);
        assert!(drain_updates(&mut receiver, &mut cache).closed);
    }

    #[test]
    fn server_list_drops_only_own_leaves_and_forgotten_rooms() {
        assert!(listed_by_server(RoomState::Joined, false, None));
        assert!(listed_by_server(RoomState::Invited, false, None));
        assert!(listed_by_server(RoomState::Knocked, false, None));
        assert!(listed_by_server(RoomState::Banned, false, None));
        assert!(!listed_by_server(RoomState::Banned, true, None));
        // Kicked: someone else sent the leave.
        assert!(listed_by_server(RoomState::Left, false, Some(false)));
        assert!(!listed_by_server(RoomState::Left, false, Some(true)));
        assert!(!listed_by_server(RoomState::Left, true, Some(false)));
        // Could not read the member event: counted, so the counter hides early.
        assert!(listed_by_server(RoomState::Left, false, None));
    }

    #[test]
    fn joined_asks_run_one_at_a_time_and_repeat_once() {
        let mut ask = JoinedAsk::default();
        assert!(claim_joined_ask(&mut ask));
        assert!(!claim_joined_ask(&mut ask));
        assert!(!claim_joined_ask(&mut ask));
        assert!(finish_joined_ask(&mut ask));
        assert!(!finish_joined_ask(&mut ask));
        assert!(!ask.running);
        assert!(claim_joined_ask(&mut ask));
    }

    #[tokio::test]
    async fn a_task_cut_off_before_its_first_poll_releases_the_claim() {
        *JOINED_ASK.lock().unwrap() = JoinedAsk { running: true, again: true };
        let guard = JoinedAskGuard;
        let task = tokio::spawn(async move {
            let _guard = guard;
        });
        task.abort();
        let _ = task.await;
        let ask = JOINED_ASK.lock().unwrap();
        assert!(!ask.running && !ask.again);
    }

    #[test]
    fn recount_only_when_counts_or_shape_move() {
        let census = Census { listed: 3, left: 1, joined: 2 };
        assert!(needs_recount(None, (census, 7)));
        assert!(!needs_recount(Some((census, 7)), (census, 7)));
        assert!(needs_recount(Some((census, 7)), (census, 8)));
        let joined_one = Census { joined: 3, ..census };
        assert!(needs_recount(Some((census, 7)), (joined_one, 7)));
    }


    #[test]
    fn a_burst_is_drained_in_one_go() {
        let (sender, mut receiver) = tokio::sync::broadcast::channel(8);
        let mut cache: HashMap<OwnedRoomId, Option<bool>> = HashMap::new();
        for room in ["!a:x", "!b:x", "!c:x"] {
            cache.insert(OwnedRoomId::try_from(room).unwrap(), Some(true));
        }
        sender.send(update("!a:x", RoomInfoNotableUpdateReasons::MEMBERSHIP)).unwrap();
        sender.send(update("!b:x", RoomInfoNotableUpdateReasons::RECENCY_STAMP)).unwrap();
        sender.send(update("!c:x", RoomInfoNotableUpdateReasons::NONE)).unwrap();
        assert!(drain_updates(&mut receiver, &mut cache).membership);
        assert!(receiver.try_recv().is_err());
        assert_eq!(cache.len(), 1);
        assert!(cache.contains_key(&OwnedRoomId::try_from("!b:x").unwrap()));
    }

    #[test]
    fn a_lagged_drain_forgets_every_reading() {
        let (sender, mut receiver) = tokio::sync::broadcast::channel(2);
        let mut cache: HashMap<OwnedRoomId, Option<bool>> = HashMap::new();
        cache.insert(OwnedRoomId::try_from("!z:x").unwrap(), Some(false));
        for _ in 0..4 {
            sender.send(update("!a:x", RoomInfoNotableUpdateReasons::RECENCY_STAMP)).unwrap();
        }
        let drained = drain_updates(&mut receiver, &mut cache);
        assert!(drained.membership && !drained.closed);
        assert!(cache.is_empty());
        drop(sender);
        assert!(drain_updates(&mut receiver, &mut cache).closed);
    }

    #[test]
    fn forget_resets_the_generation() {
        GENERATION.send_modify(|generation| *generation = 5);
        forget();
        assert_eq!(*subscribe().borrow(), 0);
    }
}
