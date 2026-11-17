//! A room's ban list and the ban itself: who is banned, by whom and why, and
//! the rules an address passes before a ban goes out. The runtime routes.

use std::sync::Arc;

use matrix_sdk::ruma::{OwnedUserId, RoomId, UserId};
use matrix_sdk::{Client, Room, RoomMemberships};
use serde_json::{json, Value};

use crate::protocol::{reply_error, reply_ok, Command};
use crate::runtime::Sink;
use crate::text::{scrub_ids, strip_bidi};

/// Exact texts: the bridge maps them to translated messages.
pub const NOT_AN_ADDRESS: &str = "not a Matrix address";
pub const NOT_YOURSELF: &str = "you cannot ban yourself";
pub const REASON_TOO_LONG: &str = "the reason is too long";

/// The reason travels in the room's state; past this it is a letter, not a reason.
const REASON_CHARS: usize = 500;
/// What a row shows of somebody else's reason.
const SHOWN_REASON_CHARS: usize = 300;
/// The spec's bound on any identifier.
const ID_MAX_BYTES: usize = 255;

pub async fn handle(command: Command, client: Option<Client>, sink: &Arc<Sink>) {
    let id = command.id();
    let Some(client) = client else {
        sink.emit(reply_error(id, "not signed in"));
        return;
    };
    let outcome = match command {
        Command::BansList { room_id, .. } => list(&client, &room_id).await,
        Command::BansAdd { room_id, user_id, reason, .. } => {
            ban(&client, &room_id, &user_id, &reason).await
        }
        Command::BansLift { room_id, user_id, .. } => lift(&client, &room_id, &user_id).await,
        // The profile page's two commands, same rules, their old reply shape.
        Command::MemberBan { room_id, user_id, .. } => ban(&client, &room_id, &user_id, "")
            .await
            .map(|_| json!({ "roomId": room_id, "userId": user_id })),
        Command::MemberUnban { room_id, user_id, .. } => lift(&client, &room_id, &user_id).await,
        _ => Err("not a ban command".to_owned()),
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

/// A typed address, or why it is none. Historical localparts pass, they
/// exist; a server without a dot is a typo far more often than a host.
pub(crate) fn parse_address(input: &str) -> Result<OwnedUserId, String> {
    let address = input.trim();
    if address.is_empty()
        || !address.starts_with('@')
        || address.len() > ID_MAX_BYTES
        || address.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(NOT_AN_ADDRESS.to_owned());
    }
    let user = UserId::parse(address).map_err(|_| NOT_AN_ADDRESS.to_owned())?;
    let host = user.server_name().host();
    let plausible_host = host.contains('.') || host.starts_with('[') || host == "localhost";
    // The SDK parses leniently; the spec's historical set is printable ASCII.
    let printable = user.localpart().bytes().all(|b| (0x21..=0x7E).contains(&b));
    if user.localpart().is_empty() || !printable || !plausible_host {
        return Err(NOT_AN_ADDRESS.to_owned());
    }
    Ok(user)
}

/// Someone the room's store knows is real whatever the host looks like; only
/// a typed stranger has to pass the plausibility rule.
async fn target(room: &Room, input: &str) -> Result<OwnedUserId, String> {
    if let Ok(user) = UserId::parse(input.trim()) {
        if matches!(room.get_member_no_sync(&user).await, Ok(Some(_))) {
            return Ok(user);
        }
    }
    parse_address(input)
}

/// A banned account cannot lift its own ban; the server would let it try.
pub(crate) fn refuse_self(target: &UserId, own: Option<&UserId>) -> Result<(), String> {
    match own {
        Some(own) if own == target => Err(NOT_YOURSELF.to_owned()),
        _ => Ok(()),
    }
}

/// Empty means none; over the bound is refused rather than cut.
pub(crate) fn checked_reason(reason: &str) -> Result<Option<String>, String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Ok(None);
    }
    if reason.chars().count() > REASON_CHARS {
        return Err(REASON_TOO_LONG.to_owned());
    }
    Ok(Some(reason.to_owned()))
}

fn shown(text: &str, limit: usize) -> String {
    let mut cut: String = text.chars().take(limit).collect();
    if cut.chars().count() < text.chars().count() {
        cut.push('…');
    }
    strip_bidi(&cut)
}

/// The room's banned, by name, plus whether this account may ban at all.
/// `members()` fetches the whole list once; sync keeps it current after that.
async fn list(client: &Client, room_id: &str) -> Result<Value, String> {
    let room = known_room(client, room_id)?;
    let banned = room
        .members(RoomMemberships::BAN)
        .await
        .map_err(|error| format!("bans unavailable: {}", scrub_ids(&error.to_string())))?;

    let own = client.user_id().map(|own| own.to_owned());
    let levels = room.power_levels_or_default().await;
    let can_ban = own.as_deref().is_some_and(|own| levels.user_can_ban(own));

    let mut rows: Vec<(String, Value)> = Vec::with_capacity(banned.len());
    for member in &banned {
        let event = member.event();
        let reason = event
            .reason()
            .map(|reason| shown(reason, SHOWN_REASON_CHARS))
            .unwrap_or_default();
        let by = event.sender().to_owned();
        let by_name = match room.get_member_no_sync(&by).await {
            Ok(Some(sender)) => strip_bidi(sender.display_name().unwrap_or_default()),
            _ => String::new(),
        };
        let since_ms: u64 = event.origin_server_ts().map(|ts| ts.0.into()).unwrap_or(0);
        let can_unban = own
            .as_deref()
            .is_some_and(|own| own != member.user_id() && levels.user_can_unban_user(own, member.user_id()));
        let name = strip_bidi(member.display_name().unwrap_or_default());
        let key = if name.is_empty() { member.user_id().as_str() } else { name.as_str() }.to_lowercase();
        rows.push((
            key,
            json!({
                "userId": member.user_id().as_str(),
                "displayName": name,
                "reason": reason,
                "bannedBy": by.as_str(),
                "bannedByName": by_name,
                "sinceMs": since_ms,
                "canUnban": can_unban,
            }),
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(json!({
        "roomId": room_id,
        "canBan": can_ban,
        "bans": rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>(),
    }))
}

/// Bans by address, member or not. Answers the row the list shows: no diff
/// brings it, and the store learns of it only on a later sync.
async fn ban(client: &Client, room_id: &str, user_id: &str, reason: &str) -> Result<Value, String> {
    let reason = checked_reason(reason)?;
    let room = known_room(client, room_id)?;
    let user = target(&room, user_id).await?;
    refuse_self(&user, client.user_id())?;

    room.ban_user(&user, reason.as_deref())
        .await
        .map_err(|error| format!("could not ban: {}", scrub_ids(&error.to_string())))?;

    // The store's last name for them, if they were ever here; the address otherwise.
    let name = match room.get_member_no_sync(&user).await {
        Ok(Some(member)) => strip_bidi(member.display_name().unwrap_or_default()),
        _ => String::new(),
    };
    let own = client.user_id().map(|own| own.to_owned());
    let levels = room.power_levels_or_default().await;
    let can_unban = own.as_deref().is_some_and(|own| levels.user_can_unban_user(own, &user));
    let now_ms: u64 = matrix_sdk::ruma::MilliSecondsSinceUnixEpoch::now().0.into();

    Ok(json!({
        "roomId": room_id,
        "userId": user.as_str(),
        "row": {
            "userId": user.as_str(),
            "displayName": name,
            "reason": reason.as_deref().map(|r| shown(r, SHOWN_REASON_CHARS)).unwrap_or_default(),
            "bannedBy": own.as_deref().map(|own| own.as_str()).unwrap_or_default(),
            "bannedByName": "",
            "sinceMs": now_ms,
            "canUnban": can_unban,
        },
    }))
}

/// Lifts a ban. The address is checked like a ban's.
async fn lift(client: &Client, room_id: &str, user_id: &str) -> Result<Value, String> {
    let room = known_room(client, room_id)?;
    let user = target(&room, user_id).await?;
    refuse_self(&user, client.user_id())?;
    room.unban_user(&user, None)
        .await
        .map_err(|error| format!("could not lift the ban: {}", scrub_ids(&error.to_string())))?;
    Ok(json!({ "roomId": room_id, "userId": user.as_str() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_address_parses() {
        let user = parse_address("@spam-bot:example.org").unwrap();
        assert_eq!(user.as_str(), "@spam-bot:example.org");
    }

    #[test]
    fn surrounding_space_is_trimmed() {
        let user = parse_address("  @a:example.org\n").unwrap();
        assert_eq!(user.as_str(), "@a:example.org");
    }

    #[test]
    fn port_and_ip_literal_pass() {
        assert!(parse_address("@a:example.org:8448").is_ok());
        assert!(parse_address("@a:[::1]").is_ok());
        assert!(parse_address("@a:localhost").is_ok());
    }

    #[test]
    fn historical_localpart_passes() {
        assert!(parse_address("@Old_Name:example.org").is_ok());
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for input in [
            "",
            "   ",
            "name:example.org",
            "@:example.org",
            "@name",
            "@name:",
            "@na me:example.org",
            "@name:exa mple.org",
            "@name:server",
            "#room:example.org",
            "!room:example.org",
            "@name:example.org\u{0}",
            "@näme:example.org",
        ] {
            assert_eq!(parse_address(input), Err(NOT_AN_ADDRESS.to_owned()), "{input:?}");
        }
    }

    #[test]
    fn overlong_address_is_refused() {
        let long = format!("@{}:example.org", "a".repeat(250));
        assert_eq!(parse_address(&long), Err(NOT_AN_ADDRESS.to_owned()));
    }

    #[test]
    fn self_ban_is_refused() {
        let own = UserId::parse("@me:example.org").unwrap();
        let other = UserId::parse("@bot:example.org").unwrap();
        assert_eq!(refuse_self(&own, Some(&own)), Err(NOT_YOURSELF.to_owned()));
        assert_eq!(refuse_self(&other, Some(&own)), Ok(()));
        assert_eq!(refuse_self(&other, None), Ok(()));
    }

    #[test]
    fn reason_is_optional_and_bounded() {
        assert_eq!(checked_reason(""), Ok(None));
        assert_eq!(checked_reason("  \n "), Ok(None));
        assert_eq!(checked_reason(" spam "), Ok(Some("spam".to_owned())));
        assert!(checked_reason(&"x".repeat(REASON_CHARS)).is_ok());
        assert_eq!(
            checked_reason(&"x".repeat(REASON_CHARS + 1)),
            Err(REASON_TOO_LONG.to_owned())
        );
    }
}
