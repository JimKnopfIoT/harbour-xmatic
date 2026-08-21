//! The member list of one room.
//!
//! Unlike the directory, this is a one-shot read rather than a live stream:
//! the page asks for a room's members, the SDK hands back a `Vec`, and the
//! whole list ships as a single `members.diff` reset. Joining and leaving
//! while the page is open is rare enough that a refresh covers it; a live
//! stream would be the follow-up if that ever matters.

use crate::timeline::scrub_ids;
use matrix_sdk::ruma::events::ignored_user_list::{IgnoredUser, IgnoredUserListEventContent};
use matrix_sdk::ruma::events::room::member::MembershipState;
use matrix_sdk::ruma::events::room::power_levels::UserPowerLevel;
use matrix_sdk::ruma::{Int, RoomId, UserId};
use matrix_sdk::{Client, Room, RoomMemberships};
use serde_json::{json, Value};
use std::collections::BTreeMap;

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
        MembershipState::Ban => "ban",
        MembershipState::Leave => "leave",
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
        .map_err(|error| format!("members unavailable: {}", scrub_ids(&error.to_string())))?;

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
        .map_err(|error| format!("members unavailable: {}", scrub_ids(&error.to_string())))?;

    let own_user = client.user_id().map(|user| user.to_owned());
    let own_device = client.device_id().map(|device| device.to_owned());
    let encryption = client.encryption();

    let mut out = Vec::new();
    for member in members {
        let user_id = member.user_id();
        let mut devices = encryption
            .get_user_devices(user_id)
            .await
            .map_err(|error| format!("could not read the devices: {}", scrub_ids(&error.to_string())))?;

        // Nothing known about this member yet: ask the server before drawing
        // any conclusion. A failure here is left to fall through to the empty
        // list rather than failing the whole check — one unreachable user must
        // not suppress the warning about the others.
        if devices.devices().next().is_none() {
            if encryption.request_user_identity(user_id).await.is_ok() {
                devices = encryption
                    .get_user_devices(user_id)
                    .await
                    .map_err(|error| format!("could not read the devices: {}", scrub_ids(&error.to_string())))?;
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
        .map_err(|error| format!("could not remove the member: {}", scrub_ids(&error.to_string())))
}

fn parsed_user(user_id: &str) -> Result<&UserId, String> {
    <&UserId>::try_from(user_id).map_err(|_| "not a user identifier".to_owned())
}

/// Everything the member-profile page shows about one user in one room, as
/// one object. Verification: `verified`, `violation` (was verified, keys
/// changed since), `unverified`, `unknown`.
pub async fn profile(client: &Client, room_id: &str, user_id: &str) -> Result<Value, String> {
    let room = known_room(client, room_id)?;
    let user = parsed_user(user_id)?;

    // The store first: `get_member` syncs the whole member list, and opening
    // a room already spawns that fetch — asking again would run a second full
    // `/members` for the same room.
    let member = match room.get_member_no_sync(user).await {
        Ok(Some(member)) => member,
        _ => room
            .get_member(user)
            .await
            .map_err(|error| format!("member unavailable: {}", scrub_ids(&error.to_string())))?
            .ok_or_else(|| "not a member of this room".to_owned())?,
    };

    let own_id = client.user_id().map(|own| own.to_owned());
    let is_self = own_id.as_deref() == Some(user);
    let levels = room.power_levels_or_default().await;
    let target_power = power(member.power_level());
    let own_power = own_id
        .as_deref()
        .map(|own| power(levels.for_user(own)))
        .unwrap_or(0);

    // Moderation flags decide what the page offers; the server enforces
    // everything again. Each one is ruma's own rule, never a rebuilt
    // comparison: unbanning also needs the kick level, and changing a role
    // has to respect a v12 creator's infinite level, which `power()` flattens
    // to 100 for display.
    let banned = member.membership() == &MembershipState::Ban;
    let can_remove = !is_self
        && !banned
        && own_id
            .as_deref()
            .map(|own| levels.user_can_kick_user(own, user))
            .unwrap_or(false);
    let can_ban = !is_self
        && !banned
        && own_id
            .as_deref()
            .map(|own| levels.user_can_ban_user(own, user))
            .unwrap_or(false);
    let can_unban = !is_self
        && banned
        && own_id
            .as_deref()
            .map(|own| levels.user_can_unban_user(own, user))
            .unwrap_or(false);
    let can_set_power = !is_self
        && own_id
            .as_deref()
            .map(|own| levels.user_can_change_user_power_level(own, user))
            .unwrap_or(false);

    // Member event: when the membership last changed; its sender is the
    // inviter while invited.
    let event = member.event();
    let since_ms: u64 = event
        .origin_server_ts()
        .map(|ts| ts.0.into())
        .unwrap_or(0);
    let invited_by = if member.membership() == &MembershipState::Invite {
        event.sender().as_str().to_owned()
    } else {
        String::new()
    };

    // Shared rooms, names only, from the local store. `get_member_no_sync`:
    // one network round per joined room would invite rate limits; rooms
    // whose member list never synced undercount.
    let mut shared: Vec<String> = Vec::new();
    if !is_self {
        for other in client.joined_rooms() {
            if other.room_id() == room.room_id() || other.is_space() {
                continue;
            }
            let joined = matches!(
                other.get_member_no_sync(user).await,
                Ok(Some(m)) if m.membership() == &MembershipState::Join
            );
            if joined {
                shared.push(
                    other
                        .cached_display_name()
                        .map(|name| name.to_string())
                        .unwrap_or_else(|| other.room_id().as_str().to_owned()),
                );
            }
        }
        shared.sort_by_key(|name| name.to_lowercase());
    }

    // Empty store means "never seen", not "unverified": ask the server once
    // before judging (same rule as `unverified_recipients`). Only in an
    // encrypted room — `request_user_identity` is a real `/keys/query`, and
    // behind a rate limit the SDK retries for up to fifteen minutes, which
    // would hang this page on a room where the answer means nothing anyway.
    let encryption = client.encryption();
    let mut identity = encryption.get_user_identity(user).await.ok().flatten();
    if identity.is_none() && room.encryption_state().is_encrypted() {
        if let Ok(fetched) = encryption.request_user_identity(user).await {
            identity = fetched;
        }
    }
    let verification = match &identity {
        Some(identity) if identity.is_verified() => "verified",
        Some(identity) if identity.has_verification_violation() => "violation",
        Some(_) => "unverified",
        None => "unknown",
    };
    let devices = encryption
        .get_user_devices(user)
        .await
        .map(|devices| devices.devices().count())
        .unwrap_or(0);

    Ok(json!({
        "roomId": room.room_id().as_str(),
        "userId": user.as_str(),
        "displayName": member.display_name().unwrap_or_default(),
        "avatar": member.avatar_url().map(|url| url.to_string()),
        "membership": membership(member.membership()),
        "power": target_power,
        "isSelf": is_self,
        "ignored": member.is_ignored(),
        "sinceMs": since_ms,
        "invitedBy": invited_by,
        "sharedRooms": shared,
        "verification": verification,
        "devices": devices,
        "canRemove": can_remove,
        "canBan": can_ban,
        "canUnban": can_unban,
        "canSetPower": can_set_power,
        "ownPower": own_power,
    }))
}

/// Bans a member from a room.
pub async fn ban(client: &Client, room_id: &str, user_id: &str) -> Result<(), String> {
    let room = known_room(client, room_id)?;
    let user = parsed_user(user_id)?;
    room.ban_user(user, None)
        .await
        .map_err(|error| format!("could not ban the member: {}", scrub_ids(&error.to_string())))
}

/// Lifts a ban.
pub async fn unban(client: &Client, room_id: &str, user_id: &str) -> Result<(), String> {
    let room = known_room(client, room_id)?;
    let user = parsed_user(user_id)?;
    room.unban_user(user, None)
        .await
        .map_err(|error| format!("could not lift the ban: {}", scrub_ids(&error.to_string())))
}

/// Sets a member's power level (0 member, 50 moderator, 100 admin).
pub async fn set_power(client: &Client, room_id: &str, user_id: &str, level: i64) -> Result<(), String> {
    let room = known_room(client, room_id)?;
    let user = parsed_user(user_id)?;
    let level = Int::new(level).ok_or_else(|| "not a power level".to_owned())?;
    room.update_power_levels(vec![(user, level)])
        .await
        .map(|_| ())
        .map_err(|error| format!("could not change the role: {}", scrub_ids(&error.to_string())))
}

/// The account's ignore list, asked of the server rather than read from the
/// store.
///
/// The SDK's own `ignore_user` reads the local copy, and an empty store there
/// is indistinguishable from an empty list — right after a login or a store
/// reset it would write a list of one and drop everybody else's entries. The
/// store is only the fallback for when the server cannot be reached, and then
/// nothing is written.
async fn ignore_list(client: &Client) -> Result<IgnoredUserListEventContent, String> {
    let account = client.account();

    if let Ok(Some(raw)) = account
        .fetch_account_data_static::<IgnoredUserListEventContent>()
        .await
    {
        if let Ok(content) = raw.deserialize() {
            return Ok(content);
        }
    }

    match account
        .account_data::<IgnoredUserListEventContent>()
        .await
        .map_err(|error| format!("could not read the ignore list: {}", scrub_ids(&error.to_string())))?
    {
        Some(raw) => raw
            .deserialize()
            .map_err(|error| format!("could not read the ignore list: {}", scrub_ids(&error.to_string()))),
        None => Ok(IgnoredUserListEventContent::new(BTreeMap::new())),
    }
}

/// Ignores or unignores a user, account-wide. Read-modify-write against the
/// server's copy of the list, see `ignore_list`.
pub async fn set_ignored(client: &Client, user_id: &str, ignored: bool) -> Result<(), String> {
    let user = parsed_user(user_id)?;
    let mut content = ignore_list(client).await?;

    let changed = if ignored {
        content
            .ignored_users
            .insert(user.to_owned(), IgnoredUser::new())
            .is_none()
    } else {
        content.ignored_users.remove(user).is_some()
    };

    if !changed {
        return Ok(());
    }

    client
        .account()
        .set_account_data(content)
        .await
        .map(|_| ())
        .map_err(|error| format!("could not change the ignore list: {}", scrub_ids(&error.to_string())))
}

/// The account's ignored users (`m.ignored_user_list`), sorted. Asked of the
/// server, so the page does not show an empty list before the first sync.
pub async fn ignored(client: &Client) -> Result<Vec<String>, String> {
    let content = ignore_list(client).await?;
    let mut users: Vec<String> = content
        .ignored_users
        .keys()
        .map(|user| user.as_str().to_owned())
        .collect();
    users.sort_by_key(|user| user.to_lowercase());
    Ok(users)
}

/// Withdraws a verification after the other side's identity changed.
pub async fn withdraw_verification(client: &Client, user_id: &str) -> Result<(), String> {
    let user = parsed_user(user_id)?;
    let identity = client
        .encryption()
        .get_user_identity(user)
        .await
        .map_err(|error| format!("could not read the identity: {}", scrub_ids(&error.to_string())))?
        .ok_or_else(|| "no identity known for this user".to_owned())?;
    identity
        .withdraw_verification()
        .await
        .map_err(|error| format!("could not withdraw the verification: {}", scrub_ids(&error.to_string())))
}
