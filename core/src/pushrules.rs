//! The account's notification rules per kind of message: off, on, or on with a
//! tone. Stored as push rules, so every client of the account follows them.

use std::fmt::Display;
use std::future::IntoFuture;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use matrix_sdk::config::RequestConfig;
use matrix_sdk::ruma::api::client::push::{
    get_pushrules_all, set_pushrule_actions, set_pushrule_enabled,
};
use matrix_sdk::ruma::push::{Action, AnyPushRuleRef, RuleKind, Ruleset, Tweak};
use matrix_sdk::{Client, HttpError};
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::protocol::{reply_error, reply_ok, Command};
use crate::runtime::Sink;
use crate::text::scrub_ids;

/// One row of the page: the rules it reads and writes.
struct Category {
    key: &'static str,
    /// The first is the switch: written first going off, last coming back on.
    rules: &'static [(RuleKind, &'static str)],
    /// Legacy rules clients switch off on their own: while the switch is on, a
    /// disabled one is left out, neither read as "mixed" nor woken by a pick.
    optional: &'static [&'static str],
    /// The tone a noisy rule names; calls ring rather than chime.
    sound: &'static str,
    /// Mentions keep the highlight, which is what counts them as mentions.
    highlight: bool,
}

const CATEGORIES: &[Category] = &[
    Category {
        key: "direct",
        rules: &[
            (RuleKind::Underride, ".m.rule.room_one_to_one"),
            (RuleKind::Underride, ".m.rule.encrypted_room_one_to_one"),
            (RuleKind::Underride, ".org.matrix.msc3930.rule.poll_start_one_to_one"),
            (RuleKind::Underride, ".org.matrix.msc3930.rule.poll_end_one_to_one"),
        ],
        optional: &[],
        sound: "default",
        highlight: false,
    },
    Category {
        key: "group",
        rules: &[
            (RuleKind::Underride, ".m.rule.message"),
            (RuleKind::Underride, ".m.rule.encrypted"),
            (RuleKind::Underride, ".org.matrix.msc3930.rule.poll_start"),
            (RuleKind::Underride, ".org.matrix.msc3930.rule.poll_end"),
        ],
        optional: &[],
        sound: "default",
        highlight: false,
    },
    Category {
        key: "mention",
        rules: &[
            (RuleKind::Override, ".m.rule.is_user_mention"),
            (RuleKind::Override, ".m.rule.contains_display_name"),
            (RuleKind::Content, ".m.rule.contains_user_name"),
        ],
        optional: &[".m.rule.contains_display_name", ".m.rule.contains_user_name"],
        sound: "default",
        highlight: true,
    },
    Category {
        key: "room",
        rules: &[
            (RuleKind::Override, ".m.rule.is_room_mention"),
            (RuleKind::Override, ".m.rule.roomnotif"),
        ],
        optional: &[".m.rule.roomnotif"],
        sound: "default",
        highlight: true,
    },
    Category {
        key: "invite",
        rules: &[(RuleKind::Override, ".m.rule.invite_for_me")],
        optional: &[],
        sound: "default",
        highlight: false,
    },
    Category {
        key: "call",
        rules: &[(RuleKind::Underride, ".m.rule.call")],
        optional: &[],
        sound: "ring",
        highlight: false,
    },
];

pub async fn handle(command: Command, client: Option<Client>, sink: &Arc<Sink>) {
    let id = command.id();
    let Some(client) = client else {
        sink.emit(reply_error(id, "not signed in"));
        return;
    };
    let outcome = match command {
        // No turn: it only reads, and its number, taken before it asks, orders it.
        Command::PushRulesGet { .. } => {
            let seq = next_sequence();
            read(&client).await.map(|rules| answer(&rules, seq)).map_err(|error| error.to_string())
        }
        Command::PushRulesSet { category, level, .. } => {
            // One change at a time: its read-back is one answer with it.
            let _turn = TURN.lock().await;
            set(&client, &category, &level).await
        }
        _ => Err("not a push-rules command".to_owned()),
    };
    match outcome {
        Ok(data) => sink.emit(reply_ok(id, data)),
        Err(message) => sink.emit(reply_error(id, message)),
    }
}

static TURN: Mutex<()> = Mutex::const_new(());

/// Orders the answers; the bridge keeps only the highest.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn next_sequence() -> u64 {
    SEQUENCE.fetch_add(1, Ordering::SeqCst) + 1
}

/// Per request, the SDK's retries and any wait the server names included.
const REQUEST_LIMIT: Duration = Duration::from_secs(45);

/// A whole change up to its read-back, which has a request limit of its own.
const CHANGE_LIMIT: Duration = Duration::from_secs(60);

fn bounded() -> RequestConfig {
    RequestConfig::new().retry_limit(2).timeout(Duration::from_secs(20))
}

/// Why a request came back empty: no answer is not a refusal.
enum Failure {
    Refused(String),
    Late,
}

impl Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Refused(error) => f.write_str(error),
            Failure::Late => f.write_str("no answer in time"),
        }
    }
}

/// Only an answer from the server is a refusal; a transport error is none.
async fn ask<T>(
    request: impl IntoFuture<Output = Result<T, HttpError>>,
    limit: Duration,
) -> Result<T, Failure> {
    if limit.is_zero() {
        return Err(Failure::Late);
    }
    match tokio::time::timeout(limit, request).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(error)) if error.as_ruma_api_error().is_some() => {
            Err(Failure::Refused(scrub_ids(&error.to_string())))
        }
        Ok(Err(_)) | Err(_) => Err(Failure::Late),
    }
}

/// The server's copy; the store's falls back to the defaults when it has none.
async fn read(client: &Client) -> Result<Ruleset, Failure> {
    let request = get_pushrules_all::v3::Request::new();
    ask(client.send(request).with_request_config(bounded()), REQUEST_LIMIT)
        .await
        .map(|response| response.global)
}

/// `{ "levels": { "direct": "noisy", ... }, "seq": n }`.
fn answer(rules: &Ruleset, seq: u64) -> Value {
    let mut data = levels(rules);
    data["seq"] = json!(seq);
    data
}

/// A kind the server has no rule for is left out.
fn levels(rules: &Ruleset) -> Value {
    let mut map = Map::new();
    for category in CATEGORIES {
        if let Some(level) = level_of(rules, category) {
            map.insert(category.key.to_owned(), json!(level));
        }
    }
    json!({ "levels": map })
}

/// The first matching override or content rule ends the evaluation, empty
/// actions included: "off" there is a disabled rule.
fn disables(kind: &RuleKind) -> bool {
    matches!(kind, RuleKind::Override | RuleKind::Content)
}

/// A disabled optional rule beside an enabled switch.
fn kept_apart(rules: &Ruleset, category: &Category, rule_id: &str) -> bool {
    category.optional.contains(&rule_id)
        && category.rules.first().is_some_and(|(kind, switch)| {
            rules.get(kind.clone(), switch).is_some_and(|rule| rule.enabled())
        })
}

/// A disabled rule is silent, whatever its actions say.
fn share(rule: AnyPushRuleRef<'_>) -> &'static str {
    if rule.enabled() {
        level_name(rule.actions())
    } else {
        "off"
    }
}

/// Every present rule's share, disabled ones as "off"; "mixed" where they
/// disagree.
fn level_of(rules: &Ruleset, category: &Category) -> Option<&'static str> {
    let mut level = None;
    for (kind, rule_id) in category.rules {
        let Some(rule) = rules.get(kind.clone(), rule_id) else {
            continue;
        };
        if !rule.enabled() && kept_apart(rules, category, rule_id) {
            continue;
        }
        let this = share(rule);
        match level {
            None => level = Some(this),
            Some(seen) if seen != this => return Some("mixed"),
            Some(_) => {}
        }
    }
    level
}

fn level_name(actions: &[Action]) -> &'static str {
    if !actions.iter().any(Action::should_notify) {
        "off"
    } else if actions.iter().any(|action| action.sound().is_some()) {
        "noisy"
    } else {
        "on"
    }
}

fn actions_for(category: &Category, level: &str) -> Result<Vec<Action>, String> {
    let mut actions = Vec::new();
    match level {
        "off" => return Ok(actions),
        "on" => actions.push(Action::Notify),
        "noisy" => {
            actions.push(Action::Notify);
            actions.push(Action::SetTweak(Tweak::Sound(category.sound.into())));
        }
        other => return Err(format!("unknown level {other}")),
    }
    if category.highlight {
        actions.push(Action::SetTweak(Tweak::Highlight(true.into())));
    }
    Ok(actions)
}

fn same_actions(left: &[Action], right: &[Action]) -> bool {
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

enum Write {
    Actions(RuleKind, &'static str, Vec<Action>),
    Enabled(RuleKind, &'static str, bool),
}

/// Everything that differs from `level`, so a second pick finishes an aborted
/// one; the switch last coming on, so an aborted wake reads "mixed".
fn plan(rules: &Ruleset, category: &Category, level: &str) -> Result<Vec<Write>, String> {
    let actions = actions_for(category, level)?;
    let mut writes = Vec::new();
    let mut switch = Vec::new();
    for (index, (kind, rule_id)) in category.rules.iter().enumerate() {
        let Some(rule) = rules.get(kind.clone(), rule_id) else {
            continue;
        };
        let into = if index == 0 && level != "off" { &mut switch } else { &mut writes };
        if level == "off" {
            if disables(kind) {
                if rule.enabled() {
                    into.push(Write::Enabled(kind.clone(), *rule_id, false));
                }
            } else if share(rule) != "off" {
                into.push(Write::Actions(kind.clone(), *rule_id, actions.clone()));
            }
            continue;
        }
        if !rule.enabled() && kept_apart(rules, category, rule_id) {
            continue;
        }
        if !same_actions(rule.actions(), &actions) {
            into.push(Write::Actions(kind.clone(), *rule_id, actions.clone()));
        }
        // Actions first: a rule enabled with the old ones would ring meanwhile.
        if !rule.enabled() {
            into.push(Write::Enabled(kind.clone(), *rule_id, true));
        }
    }
    writes.append(&mut switch);
    Ok(writes)
}

async fn write(client: &Client, change: &Write, limit: Duration) -> Result<(), Failure> {
    match change {
        Write::Actions(kind, rule_id, actions) => {
            let request = set_pushrule_actions::v3::Request::new(
                kind.clone(),
                (*rule_id).to_owned(),
                actions.clone(),
            );
            ask(client.send(request).with_request_config(bounded()), limit).await.map(drop)
        }
        Write::Enabled(kind, rule_id, enabled) => {
            let request = set_pushrule_enabled::v3::Request::new(
                kind.clone(),
                (*rule_id).to_owned(),
                *enabled,
            );
            ask(client.send(request).with_request_config(bounded()), limit).await.map(drop)
        }
    }
}

/// `failure`: `read`, `write` (refused) or `late` (a write went unanswered);
/// `seq` always, so no older read can stand in for it; `levels` where read back.
fn failed(mut data: Value, failure: &str, error: &Failure) -> Value {
    data["failure"] = json!(failure);
    data["error"] = json!(scrub_ids(&error.to_string()));
    data
}

/// Answers the levels the server holds afterwards, after a write that failed
/// part-way too; only a bad category or level is an error reply.
async fn set(client: &Client, key: &str, level: &str) -> Result<Value, String> {
    let category = CATEGORIES
        .iter()
        .find(|category| category.key == key)
        .ok_or_else(|| format!("unknown category {key}"))?;
    actions_for(category, level)?;
    let deadline = Instant::now() + CHANGE_LIMIT;
    let rules = match read(client).await {
        Ok(rules) => rules,
        Err(error) => return Ok(failed(json!({ "seq": next_sequence() }), "read", &error)),
    };
    let mut failure = None;
    for change in &plan(&rules, category, level)? {
        let left = deadline.saturating_duration_since(Instant::now()).min(REQUEST_LIMIT);
        if let Err(error) = write(client, change, left).await {
            failure = Some(error);
            break;
        }
    }
    let seq = next_sequence();
    let after = read(client).await;
    Ok(match failure {
        None => match after {
            Ok(rules) => answer(&rules, seq),
            Err(error) => failed(json!({ "seq": seq }), "read", &error),
        },
        Some(error) => {
            let data = after.map(|rules| answer(&rules, seq)).unwrap_or_else(|_| json!({ "seq": seq }));
            let kind = if matches!(error, Failure::Late) { "late" } else { "write" };
            failed(data, kind, &error)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::push::PushConditionRoomCtx;
    use matrix_sdk::ruma::serde::Raw;
    use matrix_sdk::ruma::{room_id, uint, user_id};

    /// The spec's defaults plus the legacy mention rules servers still carry.
    fn defaults() -> Ruleset {
        let mut value = serde_json::to_value(Ruleset::server_default(user_id!("@a:example.org")))
            .unwrap();
        let noisy = json!(["notify", { "set_tweak": "sound", "value": "default" }, { "set_tweak": "highlight" }]);
        let overrides = value["override"].as_array_mut().unwrap();
        let mention = overrides
            .iter()
            .position(|rule| rule["rule_id"] == ".m.rule.is_user_mention")
            .unwrap();
        overrides.insert(mention + 1, json!({
            "rule_id": ".m.rule.contains_display_name", "default": true, "enabled": true,
            "conditions": [{ "kind": "contains_display_name" }], "actions": noisy.clone(),
        }));
        let room = overrides
            .iter()
            .position(|rule| rule["rule_id"] == ".m.rule.is_room_mention")
            .unwrap();
        overrides.insert(room + 1, json!({
            "rule_id": ".m.rule.roomnotif", "default": true, "enabled": true,
            "conditions": [
                { "kind": "event_match", "key": "content.body", "pattern": "@room" },
                { "kind": "sender_notification_permission", "key": "room" }
            ],
            "actions": ["notify", { "set_tweak": "highlight" }],
        }));
        value["content"] = json!([{
            "rule_id": ".m.rule.contains_user_name", "default": true, "enabled": true,
            "pattern": "a", "actions": noisy,
        }]);
        serde_json::from_value(value).unwrap()
    }

    fn category(key: &str) -> &'static Category {
        CATEGORIES.iter().find(|category| category.key == key).unwrap()
    }

    /// What the server does with the first `count` writes; the rest is lost.
    fn apply_some(rules: &mut Ruleset, key: &str, level: &str, count: usize) {
        for change in plan(rules, category(key), level).unwrap().into_iter().take(count) {
            match change {
                Write::Actions(kind, rule_id, actions) => {
                    rules.set_actions(kind, rule_id, actions).unwrap()
                }
                Write::Enabled(kind, rule_id, enabled) => {
                    rules.set_enabled(kind, rule_id, enabled).unwrap()
                }
            }
        }
    }

    fn apply(rules: &mut Ruleset, key: &str, level: &str) {
        apply_some(rules, key, level, usize::MAX);
    }

    fn enabled(rules: &Ruleset, kind: RuleKind, rule_id: &str) -> bool {
        rules.get(kind, rule_id).unwrap().enabled()
    }

    fn level(rules: &Ruleset, key: &str) -> Option<&'static str> {
        level_of(rules, category(key))
    }

    #[test]
    fn server_defaults_ring_for_direct_and_mentions_only() {
        let value = levels(&defaults());
        assert_eq!(value["levels"]["direct"], "noisy");
        assert_eq!(value["levels"]["group"], "on");
        assert_eq!(value["levels"]["mention"], "noisy");
        assert_eq!(value["levels"]["call"], "noisy");
    }

    #[test]
    fn written_levels_read_back() {
        let mut rules = defaults();
        for category in CATEGORIES {
            for level in ["off", "on", "noisy", "off", "noisy", "on"] {
                apply(&mut rules, category.key, level);
                assert_eq!(level_of(&rules, category), Some(level), "{}", category.key);
                assert!(plan(&rules, category, level).unwrap().is_empty(), "{}", category.key);
            }
        }
    }

    #[test]
    fn off_disables_an_override_rule_and_keeps_its_actions() {
        let mut rules = defaults();
        apply(&mut rules, "mention", "off");
        let rule = rules.get(RuleKind::Override, ".m.rule.is_user_mention").unwrap();
        assert!(!rule.enabled());
        assert_eq!(level_name(rule.actions()), "noisy");
        assert!(!enabled(&rules, RuleKind::Content, ".m.rule.contains_user_name"));
        assert_eq!(level(&rules, "mention"), Some("off"));
    }

    #[test]
    fn off_on_underride_empties_the_actions_and_keeps_the_rule() {
        let mut rules = defaults();
        apply(&mut rules, "direct", "off");
        let rule = rules.get(RuleKind::Underride, ".m.rule.room_one_to_one").unwrap();
        assert!(rule.enabled());
        assert!(rule.actions().is_empty());
    }

    #[test]
    fn rules_that_disagree_read_as_mixed() {
        let mut rules = defaults();
        rules
            .set_actions(RuleKind::Override, ".m.rule.contains_display_name", vec![Action::Notify])
            .unwrap();
        assert_eq!(level(&rules, "mention"), Some("mixed"));
        apply(&mut rules, "mention", "noisy");
        assert_eq!(level(&rules, "mention"), Some("noisy"));
    }

    #[test]
    fn a_rule_disabled_elsewhere_stays_disabled() {
        let mut rules = defaults();
        rules.set_enabled(RuleKind::Override, ".m.rule.contains_display_name", false).unwrap();
        assert_eq!(level(&rules, "mention"), Some("noisy"));
        apply(&mut rules, "mention", "on");
        assert!(!enabled(&rules, RuleKind::Override, ".m.rule.contains_display_name"));
        assert_eq!(level(&rules, "mention"), Some("on"));
        // Back from "off", the whole category is on again.
        apply(&mut rules, "mention", "off");
        apply(&mut rules, "mention", "noisy");
        assert!(enabled(&rules, RuleKind::Override, ".m.rule.contains_display_name"));
    }

    #[test]
    fn an_optional_room_rule_disabled_elsewhere_stays_disabled() {
        let mut rules = defaults();
        rules.set_enabled(RuleKind::Override, ".m.rule.roomnotif", false).unwrap();
        assert_eq!(level(&rules, "room"), Some("on"));
        apply(&mut rules, "room", "noisy");
        assert!(!enabled(&rules, RuleKind::Override, ".m.rule.roomnotif"));
        assert_eq!(level(&rules, "room"), Some("noisy"));
    }

    #[test]
    fn a_disabled_switch_beside_an_enabled_legacy_rule_is_mixed() {
        let mut rules = defaults();
        rules.set_enabled(RuleKind::Override, ".m.rule.is_user_mention", false).unwrap();
        assert_eq!(level(&rules, "mention"), Some("mixed"));
    }

    #[test]
    fn an_aborted_off_reads_as_mixed_and_a_second_pick_finishes_it() {
        let mut rules = defaults();
        // The switch goes first.
        apply_some(&mut rules, "mention", "off", 1);
        assert!(!enabled(&rules, RuleKind::Override, ".m.rule.is_user_mention"));
        assert_eq!(level(&rules, "mention"), Some("mixed"));
        apply(&mut rules, "mention", "off");
        assert_eq!(level(&rules, "mention"), Some("off"));
        assert!(!enabled(&rules, RuleKind::Content, ".m.rule.contains_user_name"));
    }

    #[test]
    fn an_aborted_wake_reads_as_mixed_and_a_second_pick_finishes_it() {
        let mut rules = defaults();
        apply(&mut rules, "mention", "off");
        let writes = plan(&rules, category("mention"), "noisy").unwrap();
        // The switch goes last.
        assert!(matches!(
            writes.last(),
            Some(Write::Enabled(_, ".m.rule.is_user_mention", true))
        ));
        // One legacy rule woken, the switch and the other still off.
        apply_some(&mut rules, "mention", "noisy", 1);
        assert!(enabled(&rules, RuleKind::Override, ".m.rule.contains_display_name"));
        assert_eq!(level(&rules, "mention"), Some("mixed"));
        apply(&mut rules, "mention", "noisy");
        assert_eq!(level(&rules, "mention"), Some("noisy"));
        for (kind, rule_id) in category("mention").rules {
            assert!(enabled(&rules, kind.clone(), rule_id), "{rule_id}");
        }
    }

    #[test]
    fn a_partly_written_level_is_finished_by_the_same_pick() {
        let mut rules = defaults();
        apply_some(&mut rules, "group", "noisy", 1);
        assert_eq!(level(&rules, "group"), Some("mixed"));
        apply(&mut rules, "group", "noisy");
        assert_eq!(level(&rules, "group"), Some("noisy"));
    }

    #[test]
    fn a_disabled_underride_rule_is_silent() {
        let mut rules = defaults();
        rules.set_enabled(RuleKind::Underride, ".m.rule.encrypted", false).unwrap();
        assert_eq!(level(&rules, "group"), Some("mixed"));
        apply(&mut rules, "group", "on");
        assert!(enabled(&rules, RuleKind::Underride, ".m.rule.encrypted"));
        assert_eq!(level(&rules, "group"), Some("on"));

        // All disabled is off, and "off" has nothing left to write.
        for (kind, rule_id) in category("group").rules {
            rules.set_enabled(kind.clone(), rule_id, false).unwrap();
        }
        assert_eq!(level(&rules, "group"), Some("off"));
        assert!(plan(&rules, category("group"), "off").unwrap().is_empty());
        apply(&mut rules, "group", "noisy");
        assert_eq!(level(&rules, "group"), Some("noisy"));
    }

    #[test]
    fn a_silent_enabled_override_rule_is_disabled_by_off() {
        let mut rules = defaults();
        rules.set_actions(RuleKind::Override, ".m.rule.invite_for_me", Vec::new()).unwrap();
        assert_eq!(level(&rules, "invite"), Some("off"));
        apply(&mut rules, "invite", "off");
        assert!(!enabled(&rules, RuleKind::Override, ".m.rule.invite_for_me"));
    }

    #[tokio::test]
    async fn a_reply_in_a_direct_chat_rings_with_mentions_off() {
        let user = user_id!("@a:example.org");
        let mut rules = defaults();
        apply(&mut rules, "mention", "off");
        apply(&mut rules, "direct", "noisy");
        // The SDK names the quoted sender in every reply's `m.mentions`.
        let event = Raw::new(&json!({
            "type": "m.room.message",
            "sender": "@b:example.org",
            "room_id": "!r:example.org",
            "event_id": "$e",
            "origin_server_ts": 1,
            "content": {
                "msgtype": "m.text",
                "body": "> <@a:example.org> hi\n\nyes",
                "m.mentions": { "user_ids": [user.as_str()] },
                "m.relates_to": { "m.in_reply_to": { "event_id": "$q" } }
            }
        }))
        .unwrap();
        let context = PushConditionRoomCtx::new(
            room_id!("!r:example.org").to_owned(),
            uint!(2),
            user.to_owned(),
            "A".to_owned(),
        );
        let actions = rules.get_actions(&event, &context).await;
        assert_eq!(level_name(actions), "noisy");
        assert!(!actions.iter().any(|action| action.is_highlight()));

        // The mention rule does match: switched on, it highlights.
        apply(&mut rules, "mention", "on");
        let actions = rules.get_actions(&event, &context).await;
        assert!(actions.iter().any(|action| action.is_highlight()));

        // Empty actions on the override instead would end the evaluation silently.
        for (kind, rule_id) in category("mention").rules {
            rules.set_actions(kind.clone(), rule_id, Vec::new()).unwrap();
        }
        assert!(rules.get_actions(&event, &context).await.is_empty());
    }
}
