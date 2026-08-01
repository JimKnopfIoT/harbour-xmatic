//! The member list of one room.
//!
//! Unlike the directory, this is a one-shot read rather than a live stream:
//! the page asks for a room's members, the SDK hands back a `Vec`, and the
//! whole list ships as a single `members.diff` reset. Joining and leaving
//! while the page is open is rare enough that a refresh covers it; a live
//! stream would be the follow-up if that ever matters.

use matrix_sdk::ruma::events::room::member::MembershipState;
use matrix_sdk::ruma::events::room::power_levels::UserPowerLevel;
use matrix_sdk::ruma::{RoomId, UserId};
use matrix_sdk::{Client, Room, RoomMemberships};
use serde_json::{json, Value};

/// The room, or the reason there is none.
fn known_room(client: &Client, room_id: &str) -> Result<Room, String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())
}

fn membership(state: &MembershipState) -> &'static str {
    match state {
        MembershipState::Join => "join",
        MembershipState::Invite => "invite",
        _ => "other",
    }
}

/// A room creator's level is "infinite" from room version 12 on; treat that as
/// admin (100) so it both sorts to the top and shows the admin marker.
fn power(level: UserPowerLevel) -> i64 {
    match level {
        UserPowerLevel::Infinite => 100,
        UserPowerLevel::Int(int) => int.into(),
        // The enum is non-exhaustive; an unknown future level counts as none.
        _ => 0,
    }
}

/// Fetches a room's active members (joined and invited), most powerful first
/// and then by name, as rows shaped for the member model.
pub async fn load(client: &Client, room_id: &str) -> Result<Vec<Value>, String> {
    let room = known_room(client, room_id)?;

    let members = room
        .members(RoomMemberships::ACTIVE)
        .await
        .map_err(|error| format!("members unavailable: {error}"))?;

    // What the signed-in user may do decides which per-member actions the UI
    // offers; computed here because the power levels live next to the members.
    let own_id = client.user_id().map(|own| own.to_owned());
    let levels = room.power_levels_or_default().await;

    let mut rows: Vec<(i64, String, Value)> = members
        .iter()
        .map(|member| {
            let user_id = member.user_id().as_str().to_owned();
            let name = member.display_name().unwrap_or_default().to_owned();
            let level = power(member.power_level());
            let is_self = own_id.as_deref() == Some(member.user_id());
            let can_remove = !is_self
                && own_id
                    .as_deref()
                    .map(|own| levels.user_can_kick_user(own, member.user_id()))
                    .unwrap_or(false);
            // The sort key falls back to the id so nameless members still land
            // in a stable place.
            let key = if name.is_empty() { user_id.clone() } else { name.clone() };
            let row = json!({
                "userId": user_id,
                "displayName": name,
                "membership": membership(member.membership()),
                "power": level,
                "isSelf": is_self,
                "canRemove": can_remove,
                // The picture itself is fetched by the UI on demand; this is
                // only where to find it.
                "avatar": member.avatar_url().map(|url| url.to_string()),
            });
            (level, key.to_lowercase(), row)
        })
        .collect();

    // Highest power level first, then alphabetically by the display key.
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    Ok(rows.into_iter().map(|(_, _, row)| row).collect())
}

/// Joined recipients of an encrypted room that still have unverified devices,
/// so the UI can warn before a message goes to a session the user never
/// checked. One entry per user, with how many of their devices are unverified.
///
/// Empty for an unencrypted room and when everything is verified. This device
/// is skipped — it is always trusted to itself.
///
/// A member whose devices this client has never downloaded is asked for
/// explicitly. `get_user_devices` reads the local crypto store and nothing
/// else: it passes no timeout, and without one the SDK's `wait_if_user_pending`
/// returns at once (`matrix-sdk-crypto`, `machine/mod.rs`). Loading the member
/// list only *marks* those users for a later `/keys/query`. So on the first
/// open of a room, and right after someone joins, the store is empty for them
/// and this used to report "nothing unverified" — a warning that says everything
/// was checked when nothing was, which is worse than no warning at all. The SDK
/// itself waits for the query at the equivalent point, with the comment that it
/// "may not yet have seen any devices for the user".
pub async fn unverified_recipients(client: &Client, room_id: &str) -> Result<Vec<Value>, String> {
    let room = known_room(client, room_id)?;
    if !room.encryption_state().is_encrypted() {
        return Ok(Vec::new());
    }

    let members = room
        .members(RoomMemberships::JOIN)
        .await
        .map_err(|error| format!("members unavailable: {error}"))?;

    let own_user = client.user_id().map(|user| user.to_owned());
    let own_device = client.device_id().map(|device| device.to_owned());
    let encryption = client.encryption();

    let mut out = Vec::new();
    for member in members {
        let user_id = member.user_id();
        let mut devices = encryption
            .get_user_devices(user_id)
            .await
            .map_err(|error| format!("could not read the devices: {error}"))?;

        // Nothing known about this member yet: ask the server before drawing
        // any conclusion. A failure here is left to fall through to the empty
        // list rather than failing the whole check — one unreachable user must
        // not suppress the warning about the others.
        if devices.devices().next().is_none() {
            if encryption.request_user_identity(user_id).await.is_ok() {
                devices = encryption
                    .get_user_devices(user_id)
                    .await
                    .map_err(|error| format!("could not read the devices: {error}"))?;
            }
        }

        let unverified = devices
            .devices()
            .filter(|device| {
                // This very device is us; it needs no verifying against itself.
                let is_own_current = own_user.as_deref() == Some(user_id)
                    && own_device.as_deref() == Some(device.device_id());
                !is_own_current && !device.is_verified()
            })
            .count();

        if unverified > 0 {
            out.push(json!({
                "userId": user_id.as_str(),
                "name": member.display_name().unwrap_or_default(),
                "devices": unverified,
            }));
        }
    }

    Ok(out)
}

/// Removes (kicks) a member from a room. The server enforces the power levels
/// again; the `canRemove` flag above only decides whether the action is shown.
pub async fn remove(client: &Client, room_id: &str, user_id: &str) -> Result<(), String> {
    let room = known_room(client, room_id)?;
    let user = UserId::parse(user_id).map_err(|_| "not a user identifier".to_owned())?;
    room.kick_user(&user, None)
        .await
        .map_err(|error| format!("could not remove the member: {error}"))
}
