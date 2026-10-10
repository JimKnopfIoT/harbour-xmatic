//! Mentions: who or which room a message points at. Two halves - the
//! candidates the picker offers while `@` or `#` is being typed, and the pill
//! plus `m.mentions` that make a sent message ping. Nothing else knows what a
//! mention is.

use crate::compose::{escape_text, to_formatted_body};
use crate::text::{scrub_ids, strip_bidi};
use matrix_sdk::ruma::events::room::message::{RoomMessageEventContent, TextMessageEventContent};
use matrix_sdk::ruma::events::Mentions;
use matrix_sdk::ruma::{OwnedUserId, RoomId, UserId};
use matrix_sdk::Room;
use matrix_sdk::{Client, RoomMemberships};
use serde_json::{json, Value};

/// The whole room, which is not a user id. Travels as a candidate like any
/// other and turns into `m.mentions.room` on the way out.
pub const ROOM_KEY: &str = "@room";

/// How many candidates one query answers with. The picker is one scrolling
/// row; past this many, typing another letter is the faster way.
const MAX_CANDIDATES: usize = 30;

/// Whether this member answers what has been typed. Word starts only: a query
/// that matched anywhere would offer everybody for every second letter.
fn matches(query: &str, name: &str, user_id: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    // The localpart, so `@wolf` finds `@wolf:server` whatever the display name.
    let localpart = user_id
        .trim_start_matches(['@', '#'])
        .split(':')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if localpart.starts_with(&query) {
        return true;
    }
    name.to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_' || c == '.')
        .any(|word| word.starts_with(&query))
}

/// What can be mentioned here, filtered by the typed word: `#…` offers rooms,
/// anything else members. Read from the store: asked again on every keystroke.
pub async fn candidates(client: &Client, room_id: &str, query: &str) -> Result<Vec<Value>, String> {
    match query.strip_prefix('#') {
        Some(rest) => Ok(room_candidates(client, room_id, rest).await),
        None => member_candidates(client, room_id, query.strip_prefix('@').unwrap_or(query)).await,
    }
}

/// The text a room link stands as: its alias, else `#` and its name.
fn room_text(room: &Room) -> Option<String> {
    if let Some(alias) = room.canonical_alias() {
        return Some(alias.to_string());
    }
    let name = strip_bidi(&room.cached_display_name()?.to_string());
    (!name.is_empty()).then(|| format!("#{name}"))
}

/// Joined rooms other than this one. No direct chats: a link to one opens
/// nothing for anybody else.
async fn room_candidates(client: &Client, room_id: &str, query: &str) -> Vec<Value> {
    let mut rows: Vec<(String, Value)> = Vec::new();
    for room in client.joined_rooms() {
        if room.room_id().as_str() == room_id || room.is_direct().await.unwrap_or(true) {
            continue;
        }
        let Some(insert) = room_text(&room) else { continue };
        let name = room
            .cached_display_name()
            .map(|name| strip_bidi(&name.to_string()))
            .unwrap_or_default();
        let alias = room.canonical_alias().map(|alias| alias.to_string()).unwrap_or_default();
        if !matches(query, &name, &alias) {
            continue;
        }
        let label = if name.is_empty() { insert.clone() } else { name };
        rows.push((
            label.to_lowercase(),
            json!({
                "id": room.room_id().as_str(),
                "label": label,
                "insert": insert,
                "avatar": room.avatar_url().map(|url| url.to_string()),
            }),
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows.truncate(MAX_CANDIDATES);
    rows.into_iter().map(|(_, row)| row).collect()
}

async fn member_candidates(client: &Client, room_id: &str, query: &str) -> Result<Vec<Value>, String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known yet".to_owned())?;

    let members = room
        .members_no_sync(RoomMemberships::ACTIVE)
        .await
        .map_err(|error| format!("members unavailable: {}", scrub_ids(&error.to_string())))?;

    let own_id = client.user_id().map(|own| own.to_owned());
    let levels = room.power_levels_or_default().await;

    let mut rows: Vec<(i64, String, Value)> = members
        .iter()
        // Mentioning oneself pings nobody.
        .filter(|member| own_id.as_deref() != Some(member.user_id()))
        .filter_map(|member| {
            let user_id = member.user_id().as_str().to_owned();
            let name = strip_bidi(member.display_name().unwrap_or_default());
            if !matches(query, &name, &user_id) {
                return None;
            }
            // Nameless members are mentioned by their address; the picker shows
            // what will land in the text either way.
            let label = if name.is_empty() { user_id.clone() } else { name.clone() };
            let insert = if name.is_empty() { user_id.clone() } else { format!("@{name}") };
            let row = json!({
                "id": user_id,
                "label": label.clone(),
                "insert": insert,
                "avatar": member.avatar_url().map(|url| url.to_string()),
            });
            Some((0, label.to_lowercase(), row))
        })
        .collect();

    rows.sort_by(|a, b| a.1.cmp(&b.1));
    rows.truncate(MAX_CANDIDATES);

    let mut out: Vec<Value> = Vec::with_capacity(rows.len() + 1);
    // The room itself, first and only where the power levels allow it - an
    // offer nobody may take is worse than none.
    let room_allowed = own_id
        .as_deref()
        .map(|own| levels.user_can_trigger_room_notification(own))
        .unwrap_or(false);
    if room_allowed && matches(query, "room", ROOM_KEY) {
        out.push(json!({
            "id": ROOM_KEY,
            "label": ROOM_KEY,
            "insert": ROOM_KEY,
            "avatar": Value::Null,
        }));
    }
    out.extend(rows.into_iter().map(|(_, _, row)| row));
    Ok(out)
}

/// What a mention adds to an outgoing message.
struct Outgoing {
    formatted: Option<String>,
    mentions: Option<Mentions>,
}

/// What the picked ids stand as: (text in the body, link target), the users
/// to ping, and whether `@room` is meant. Texts are read here, not taken from
/// the front end - they decide what is linked.
async fn needles(
    client: &Client,
    room_id: &str,
    ids: &[String],
) -> (Vec<(String, String)>, Vec<OwnedUserId>, bool) {
    let mut pills: Vec<(String, String)> = Vec::new();
    let mut users: Vec<OwnedUserId> = Vec::new();
    let mut room = false;

    let Ok(parsed) = RoomId::parse(room_id) else {
        return (pills, users, room);
    };
    let Some(handle) = client.get_room(&parsed) else {
        return (pills, users, room);
    };

    for id in ids {
        if id == ROOM_KEY {
            room = true;
            continue;
        }
        if let Ok(target) = <&RoomId>::try_from(id.as_str()) {
            let Some(linked) = client.get_room(target) else { continue };
            let Some(text) = room_text(&linked) else { continue };
            let href = match linked.matrix_to_permalink().await {
                Ok(uri) => uri.to_string(),
                Err(_) => target.matrix_to_uri().to_string(),
            };
            pills.push((text, href));
            continue;
        }
        let Ok(user) = UserId::parse(id.as_str()) else {
            continue;
        };
        let name = match handle.get_member_no_sync(&user).await {
            Ok(Some(member)) => strip_bidi(member.display_name().unwrap_or_default()),
            _ => String::new(),
        };
        let needle = if name.is_empty() {
            id.clone()
        } else {
            format!("@{name}")
        };
        pills.push((needle, user.matrix_to_uri().to_string()));
        users.push(user);
    }

    // The longer text first: `@Ann` must not take the head of `@Anna`.
    pills.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
    (pills, users, room)
}

/// Wraps every mention's text in a permalink. Walks the markup this process
/// wrote rather than replacing in it blindly: a match inside a tag would be
/// markup, and one inside `<code>` is quoted text, not a mention.
fn insert_pills(html: &str, pills: &[(String, String)]) -> (String, bool) {
    let mut out = String::with_capacity(html.len());
    let mut hit = false;
    let mut i = 0;

    while i < html.len() {
        let rest = &html[i..];
        if rest.starts_with('<') {
            // A tag is copied whole; `<code>` takes its content with it.
            let end = rest.find('>').map(|at| at + 1).unwrap_or(rest.len());
            if rest.starts_with("<code>") {
                let close = rest.find("</code>").map(|at| at + "</code>".len());
                let stop = close.unwrap_or(rest.len());
                out.push_str(&rest[..stop]);
                i += stop;
                continue;
            }
            out.push_str(&rest[..end]);
            i += end;
            continue;
        }

        let mut matched = false;
        for (needle, href) in pills {
            let escaped = escape_text(needle);
            if escaped.is_empty() || !rest.starts_with(&escaped) {
                continue;
            }
            out.push_str("<a href=\"");
            out.push_str(&escape_text(href).replace('"', "&quot;"));
            out.push_str("\">");
            out.push_str(&escaped);
            out.push_str("</a>");
            i += escaped.len();
            matched = true;
            hit = true;
            break;
        }
        if matched {
            continue;
        }

        let width = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
        out.push_str(&rest[..width]);
        i += width;
    }

    (out, hit)
}

/// The two halves put together, or the plain message where nothing was
/// mentioned. `ids` is what the picker collected; a name that is no longer in
/// the text still pings, because the id is what was chosen.
async fn resolve(client: Option<&Client>, room_id: &str, body: &str, ids: &[String]) -> Outgoing {
    let base = to_formatted_body(body);
    let (Some(client), false) = (client, ids.is_empty()) else {
        return Outgoing { formatted: base, mentions: None };
    };

    let (pills, users, room) = needles(client, room_id, ids).await;
    if pills.is_empty() && !room {
        return Outgoing { formatted: base, mentions: None };
    }

    // A room link pings nobody.
    let mentions = (!users.is_empty() || room).then(|| {
        let mut mentions = Mentions::with_user_ids(users);
        mentions.room = room;
        mentions
    });

    // A body with no markers has no formatted copy yet; the pill needs one.
    let source = base.clone().unwrap_or_else(|| escape_text(body));
    let (linked, hit) = insert_pills(&source, &pills);
    let formatted = if hit { Some(linked) } else { base };

    Outgoing { formatted, mentions }
}

/// A text message with its mentions, ready to send. The only way in from the
/// runtime; `ids` empty is the plain case and costs nothing.
pub async fn text_content(
    client: Option<&Client>,
    room_id: &str,
    body: String,
    ids: &[String],
) -> RoomMessageEventContent {
    let resolved = resolve(client, room_id, &body, ids).await;
    let content = match resolved.formatted {
        Some(html) => RoomMessageEventContent::text_html(body, html),
        None => RoomMessageEventContent::text_plain(body),
    };
    match resolved.mentions {
        Some(mentions) => content.add_mentions(mentions),
        None => content,
    }
}

/// An attachment's caption with its mentions: the same pills and `m.mentions`
/// a text message gets, handed to the attachment instead.
pub async fn caption_content(
    client: Option<&Client>,
    room_id: &str,
    body: &str,
    ids: &[String],
) -> (TextMessageEventContent, Option<Mentions>) {
    let resolved = resolve(client, room_id, body, ids).await;
    let content = match resolved.formatted {
        Some(html) => TextMessageEventContent::html(body, html),
        None => TextMessageEventContent::plain(body),
    };
    (content, resolved.mentions)
}

#[cfg(test)]
mod tests {
    use super::{insert_pills, matches};

    fn pill() -> Vec<(String, String)> {
        vec![("@Anna Wolf".to_owned(), "https://matrix.to/#/@anna:example.invalid".to_owned())]
    }

    #[test]
    fn a_name_becomes_a_permalink() {
        let (html, hit) = insert_pills("hi @Anna Wolf, look", &pill());
        assert!(hit);
        assert_eq!(
            html,
            "hi <a href=\"https://matrix.to/#/@anna:example.invalid\">@Anna Wolf</a>, look"
        );
    }

    #[test]
    fn markup_around_it_is_left_alone() {
        let (html, _) = insert_pills("<strong>@Anna Wolf</strong>", &pill());
        assert_eq!(
            html,
            "<strong><a href=\"https://matrix.to/#/@anna:example.invalid\">@Anna Wolf</a></strong>"
        );
    }

    #[test]
    fn quoted_text_is_not_a_mention() {
        let (html, hit) = insert_pills("<code>@Anna Wolf</code>", &pill());
        assert!(!hit);
        assert_eq!(html, "<code>@Anna Wolf</code>");
    }

    #[test]
    fn the_longer_name_wins() {
        // As `needles` sorts them: the pair that would swallow the other first.
        let pills = vec![
            ("@Anna Wolf".to_owned(), "https://matrix.to/#/@anna:example.invalid".to_owned()),
            ("@Anna".to_owned(), "https://matrix.to/#/@ann:example.invalid".to_owned()),
        ];
        let (html, _) = insert_pills("@Anna Wolf", &pills);
        assert!(html.contains("@anna:example.invalid"), "{html}");
        assert!(!html.contains("@ann:example.invalid"), "{html}");
    }

    #[test]
    fn a_name_that_is_gone_leaves_no_pill() {
        let (html, hit) = insert_pills("nothing here", &pill());
        assert!(!hit);
        assert_eq!(html, "nothing here");
    }

    #[test]
    fn the_body_is_escaped_before_a_pill_lands_in_it() {
        let (html, hit) = insert_pills("&lt;b&gt; @Anna Wolf", &pill());
        assert!(hit);
        assert!(html.starts_with("&lt;b&gt; <a href="), "{html}");
    }

    #[test]
    fn a_room_becomes_a_permalink_with_its_target_escaped() {
        let pills = vec![(
            "#Garden".to_owned(),
            "https://matrix.to/#/!r:example.invalid?via=a&via=\"b".to_owned(),
        )];
        let (html, hit) = insert_pills("see #Garden", &pills);
        assert!(hit);
        assert_eq!(
            html,
            "see <a href=\"https://matrix.to/#/!r:example.invalid?via=a&amp;via=&quot;b\">#Garden</a>"
        );
    }

    #[test]
    fn rooms_match_on_name_and_alias() {
        assert!(matches("gar", "Garden Club", "#plants:example.invalid"));
        assert!(matches("pla", "Garden Club", "#plants:example.invalid"));
        assert!(!matches("lub", "Garden Club", "#plants:example.invalid"));
    }

    #[test]
    fn candidates_match_at_word_starts_only() {
        assert!(matches("wol", "Anna Wolf", "@anna:example.invalid"));
        assert!(matches("ann", "Anna Wolf", "@anna:example.invalid"));
        // The middle of a word is not a start.
        assert!(!matches("olf", "Anna Wolf", "@anna:example.invalid"));
        // The localpart counts even where the display name says otherwise.
        assert!(matches("bot", "Anna Wolf", "@bot42:example.invalid"));
        assert!(matches("", "Anna Wolf", "@anna:example.invalid"));
    }
}
