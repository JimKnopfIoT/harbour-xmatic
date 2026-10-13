//! UnifiedPush through Leghorn. See docs/PUSH.md.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use leghorn::matrix::{Notification, Pusher as LeghornPusher};
use leghorn::{Config as PushConfig, Event as PushEvent, Leghorn};
use matrix_sdk::config::RequestConfig;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use tokio::sync::watch;

/// Org and app must match the desktop file's [X-Sailjail] names. No fallback
/// gateway; the user picks one.
pub const PUSH: PushConfig = PushConfig::new("org.xmatic", "xmatic")
    .description("xmatic")
    .matrix(APP_ID)
    .matrix_without_fallback()
    .matrix_device("xmatic", "en");

/// Must not change: existing pushers on homeservers are filed under it.
const APP_ID: &str = "org.xmatic.xmatic";

/// aws-lc as rustls' provider, before anything speaks TLS: Leghorn's gateway
/// probe would otherwise install ring as a second one.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

pub async fn start() -> (
    Result<Leghorn, String>,
    UnboundedReceiver<PushEvent>,
) {
    install_crypto_provider();
    let (events, receiver) = unbounded_channel();
    let handler = move |push: PushEvent| {
        let events = events.clone();
        async move {
            let _ = events.send(push);
            Ok::<(), std::convert::Infallible>(())
        }
    };
    let started = leghorn::start(&PUSH, handler)
        .await
        .map_err(|error| crate::text::scrub_ids(&error.to_string()));
    (started, receiver)
}

/// Pusher requests: one retry, not the SDK's fifteen minutes. A 429's
/// Retry-After is still taken as given; `PUSHER_LIMIT` bounds the whole call.
fn bounded() -> RequestConfig {
    RequestConfig::new()
        .retry_limit(2)
        .timeout(Duration::from_secs(20))
}

/// One pusher request, its retry and any wait the server named included.
const PUSHER_LIMIT: Duration = Duration::from_secs(45);

fn timed_out(context: &str) -> PusherError {
    PusherError::failed(format!("{context}: no answer in time"))
}

static SYNC_RUNNING: OnceLock<watch::Sender<u64>> = OnceLock::new();

fn sync_running_sender() -> &'static watch::Sender<u64> {
    SYNC_RUNNING.get_or_init(|| watch::channel(0).0)
}

/// The sync reached `Running`; a pusher that failed to register tries again.
pub fn note_sync_running() {
    sync_running_sender().send_modify(|count| *count = count.wrapping_add(1));
}

pub fn sync_running_changes() -> watch::Receiver<u64> {
    sync_running_sender().subscribe()
}

/// What `push.state` says, from what is known. A distributor list that could
/// not be read is `None` and decides nothing.
pub struct Facts {
    pub pusher: bool,
    pub registered: bool,
    pub gateway: bool,
    pub endpoint: bool,
    pub distributors: Option<usize>,
}

pub fn derive_state(facts: &Facts) -> &'static str {
    // A registration with an uninstalled distributor receives nothing.
    if facts.distributors == Some(0) {
        return "no-distributor";
    }
    if facts.pusher {
        if facts.registered {
            "on"
        } else if !facts.gateway {
            "needs-gateway"
        } else {
            "registering"
        }
    } else if facts.endpoint {
        "registering"
    } else {
        "off"
    }
}

/// Whether `gateway` is an https URL.
pub fn gateway_is_sound(gateway: &str) -> bool {
    let gateway = gateway.trim();
    gateway.len() > 8
        && gateway
            .get(..8)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
}

/// The host of `url`, without scheme, port or path.
pub fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    authority.split(':').next().unwrap_or_default().to_owned()
}

/// The UnifiedPush project's gateway, used only where the user picked it.
pub const PUBLIC_GATEWAY: &str = "https://matrix.gateway.unifiedpush.org/_matrix/push/v1/notify";

/// Which gateway the user picked. Nothing registers before a pick.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Gateway {
    #[default]
    Unset,
    /// The push server's own, found by Leghorn.
    Server,
    Public,
    Other(String),
}

impl Gateway {
    pub fn parse(mode: &str, url: &str) -> Result<Self, &'static str> {
        match mode {
            "" => Ok(Self::Unset),
            "server" => Ok(Self::Server),
            "public" => Ok(Self::Public),
            "other" if gateway_is_sound(url) => Ok(Self::Other(url.trim().to_owned())),
            "other" => Err("the push gateway has to be an https address"),
            _ => Err("unknown gateway choice"),
        }
    }

    pub fn mode(&self) -> &'static str {
        match self {
            Self::Unset => "",
            Self::Server => "server",
            Self::Public => "public",
            Self::Other(_) => "other",
        }
    }

    /// The gateway to register, given what the push server offered.
    pub fn resolve(&self, discovered: Option<&str>) -> Option<String> {
        match self {
            Self::Unset => None,
            Self::Server => discovered.filter(|gateway| gateway_is_sound(gateway)).map(str::to_owned),
            Self::Public => Some(PUBLIC_GATEWAY.to_owned()),
            Self::Other(url) => Some(url.clone()),
        }
    }
}

/// Room and event of a Matrix push; both null for a count-only or foreign push.
pub fn message(body: &[u8]) -> Value {
    let note = Notification::from_slice(body).ok();
    let room = note.as_ref().and_then(|note| note.room_id.clone());
    let event = note.as_ref().and_then(|note| note.event_id.clone());
    match (room, event) {
        (Some(room), Some(event)) => json!({ "roomId": room, "eventId": event }),
        _ => json!({ "roomId": null, "eventId": null }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matrix_notification_names_its_room_and_event() {
        let body = br#"{"notification":{"event_id":"$abc","room_id":"!room:example.org",
                        "counts":{"unread":1}}}"#;
        assert_eq!(
            message(body),
            json!({ "roomId": "!room:example.org", "eventId": "$abc" })
        );
    }

    #[test]
    fn anything_else_is_not_ours() {
        for probe in [
            "",
            "not json at all",
            r#"{"notification":{}}"#,
            r#"{"notification":{"counts":{"unread":3}}}"#,
            r#"{"notification":{"room_id":"!room:example.org"}}"#,
            r#"{"hello":"world"}"#,
        ] {
            assert_eq!(
                message(probe.as_bytes()),
                json!({ "roomId": null, "eventId": null }),
                "accepted: {probe}"
            );
        }
    }

    #[test]
    fn only_https_gateways_are_sound() {
        assert!(gateway_is_sound("https://push.example.org/_matrix/push/v1/notify"));
        assert!(gateway_is_sound(" HTTPS://push.example.org "));
        assert!(!gateway_is_sound("http://push.example.org/_matrix/push/v1/notify"));
        assert!(!gateway_is_sound("https://"));
        assert!(!gateway_is_sound(""));
    }

    #[test]
    fn the_gateway_is_what_was_picked() {
        let found = "https://push.example.org/_matrix/push/v1/notify";
        let mine = "https://gateway.example.net/_matrix/push/v1/notify";
        assert_eq!(Gateway::Unset.resolve(Some(found)), None);
        assert_eq!(Gateway::Server.resolve(Some(found)).as_deref(), Some(found));
        assert_eq!(Gateway::Server.resolve(None), None);
        assert_eq!(Gateway::Server.resolve(Some("http://push.example.org/notify")), None);
        assert_eq!(Gateway::Public.resolve(None).as_deref(), Some(PUBLIC_GATEWAY));
        let other = Gateway::parse("other", mine).unwrap();
        assert_eq!(other.resolve(Some(found)).as_deref(), Some(mine));
    }

    #[test]
    fn only_the_host_is_kept() {
        assert_eq!(host("https://updates.push.services.mozilla.com/wpush/v2/gAAA"), "updates.push.services.mozilla.com");
        assert_eq!(host("https://ntfy.example:8443/upAbc?up=1"), "ntfy.example");
        assert_eq!(host("https://user:pw@ntfy.example/up"), "ntfy.example");
    }

    #[test]
    fn a_pick_is_parsed_strictly() {
        assert_eq!(Gateway::parse("", ""), Ok(Gateway::Unset));
        assert_eq!(Gateway::parse("server", "ignored"), Ok(Gateway::Server));
        assert!(Gateway::parse("other", "http://gateway.example.net").is_err());
        assert!(Gateway::parse("other", "").is_err());
        assert!(Gateway::parse("fallback", "").is_err());
        assert_eq!(Gateway::parse("public", "").map(|pick| pick.mode()), Ok("public"));
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "xmatic-push-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    #[test]
    fn another_devices_pusher_is_never_ours() {
        let mut owned = Owned::with_salt("salt".to_owned(), "@a:example.org");
        owned.note("https://push.example/up/mine-old");
        let server = [
            "https://push.example/up/mine-old",
            "https://push.example/up/mine-now",
            "https://push.example/up/other-phone",
        ];
        assert_eq!(
            to_remove(&server, &owned, &[], Some("https://push.example/up/mine-now")),
            vec!["https://push.example/up/mine-old"]
        );
        assert_eq!(
            to_remove(&server, &owned, &["https://push.example/up/mine-now"], None),
            vec!["https://push.example/up/mine-old", "https://push.example/up/mine-now"]
        );
        let empty = Owned::with_salt("salt".to_owned(), "@a:example.org");
        assert!(to_remove(&server, &empty, &[], None).is_empty());
    }

    #[test]
    fn the_record_keeps_no_pushkey_and_forgets_what_went() {
        let mut owned = Owned::with_salt("salt".to_owned(), "@a:example.org");
        owned.note("https://push.example/up/a");
        owned.note("https://push.example/up/a");
        owned.note("https://push.example/up/b");
        owned.note("https://push.example/up/gone");
        assert_eq!(owned.own().len(), 3);
        let raw = serde_json::to_string(&owned).unwrap();
        assert!(!raw.contains("push.example") && !raw.contains("example.org"));
        owned.settle(
            &["https://push.example/up/a", "https://push.example/up/b"],
            &["https://push.example/up/a"],
        );
        assert!(!owned.holds("https://push.example/up/a"));
        assert!(owned.holds("https://push.example/up/b"));
        assert!(!owned.holds("https://push.example/up/gone"));
    }

    #[test]
    fn the_salt_changes_the_marks() {
        let one = Owned::with_salt("one".to_owned(), "@a:example.org");
        let two = Owned::with_salt("two".to_owned(), "@a:example.org");
        assert_ne!(one.mark("https://push.example/up/a"), two.mark("https://push.example/up/a"));
        assert_ne!(one.account, two.account);
        assert_ne!(one.digest(PUSHKEY, "x"), one.digest(ACCOUNT, "x"));
        assert_eq!(fresh_salt().len(), 32);
    }

    #[test]
    fn the_record_survives_a_reload_and_an_empty_one_is_no_file() {
        let dir = scratch("owned");
        let path = dir.join("push-pushers.json");
        let user = "@a:example.org";
        assert!(!owns_any(&path, user));
        update_owned(&path, user, &|| true, |owned| owned.note("https://push.example/up/a"));
        assert!(path.exists());
        assert!(owns(&path, user, "https://push.example/up/a"));
        assert!(!owns(&path, user, "https://push.example/up/b"));
        update_owned(&path, user, &|| true, |owned| owned.settle(&[], &[]));
        assert!(!path.exists());
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(!owns_any(&path, user));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_accounts_settling_keeps_anothers_pushers() {
        let dir = scratch("accounts");
        let path = dir.join("push-pushers.json");
        let (a, b) = ("@a:example.org", "@b:example.org");
        update_owned(&path, a, &|| true, |owned| owned.note("https://push.example/up/a"));
        // B signs in, finds nothing of its own on its server, settles.
        assert!(!owns_any(&path, b));
        update_owned(&path, b, &|| true, |owned| owned.note("https://push.example/up/b"));
        update_owned(&path, b, &|| true, |owned| owned.settle(&[], &[]));
        assert!(!owns_any(&path, b));
        assert!(owns(&path, a, "https://push.example/up/a"));
        // A's next session removes it.
        update_owned(&path, a, &|| true, |owned| {
            owned.settle(&["https://push.example/up/a"], &["https://push.example/up/a"])
        });
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_session_that_ended_writes_nothing() {
        let dir = scratch("live");
        let path = dir.join("push-pushers.json");
        update_owned(&path, "@a:example.org", &|| false, |owned| {
            owned.note("https://push.example/up/a")
        });
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_old_pusher_is_claimed_by_its_auth_secret() {
        let raw = br#"{"distributor":"org.unifiedpush.Distributor.Foghorn",
                       "registrations":[{"token":"t1","instance":"default"}],
                       "keys":{"default":"c2VjcmV0|YXV0aC1vbmU","x":"broken"}}"#;
        assert_eq!(legacy_auths(raw), vec!["YXV0aC1vbmU".to_owned()]);
        assert!(legacy_auths(b"garbage").is_empty());

        let dir = scratch("legacy");
        let legacy = dir.join("push.json");
        let path = dir.join("push-pushers.json");
        std::fs::write(&legacy, raw).unwrap();
        note_legacy_pushers(&legacy, &path);
        let user = "@a:example.org";
        assert!(owns_any(&path, user));
        let mut owned = Owned::load(&path, user);
        owned.claim_legacy(&[
            ("https://push.example/up/old", Some("YXV0aC1vbmU")),
            ("https://push.example/up/other-phone", Some("b3RoZXI")),
            ("https://push.example/up/leghorn", None),
        ]);
        assert!(owned.holds("https://push.example/up/old"));
        assert!(!owned.holds("https://push.example/up/other-phone"));
        assert!(owned.legacy.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_refusal_is_told_from_a_hiccup() {
        assert!(refusing_status(400));
        assert!(refusing_status(403));
        assert!(!refusing_status(401));
        assert!(!refusing_status(429));
        assert!(!refusing_status(500));
        assert!(!refusing_status(502));
    }

    #[test]
    fn an_event_the_server_does_not_give_is_absent() {
        use matrix_sdk::ruma::api::error::ErrorKind;
        assert!(absent_answer(404, None));
        assert!(absent_answer(403, Some(&ErrorKind::Forbidden)));
        assert!(absent_answer(400, Some(&ErrorKind::NotFound)));
        assert!(!absent_answer(500, None));
        assert!(!absent_answer(429, None));
    }

    #[test]
    fn leghorns_files_go_whatever_the_distributor_said() {
        let dir = scratch("leghorn");
        let file = dir.join("leghorn.json");
        std::fs::write(&file, b"{}").unwrap();
        std::fs::write(dir.join("leghorn.json.tmp"), b"{").unwrap();
        remove_registration(&file).unwrap();
        assert!(!file.exists());
        assert!(!dir.join("leghorn.json.tmp").exists());
        remove_registration(&file).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_old_connectors_tokens_are_read() {
        let raw = br#"{"distributor":"org.unifiedpush.Distributor.Foghorn","distributor_ack":true,
                       "registrations":[{"token":"t1","instance":"default"},{"token":"","instance":"x"}],
                       "keys":{}}"#;
        assert_eq!(
            legacy_registration(raw),
            Some(("org.unifiedpush.Distributor.Foghorn".to_owned(), vec!["t1".to_owned()]))
        );
        assert_eq!(legacy_registration(br#"{"distributor":"d","registrations":[]}"#), None);
        assert_eq!(legacy_registration(br#"{"registrations":[{"token":"t"}]}"#), None);
        assert_eq!(legacy_registration(b"garbage"), None);
    }

    #[test]
    fn a_missing_distributor_is_said_even_with_a_pusher() {
        let on = Facts {
            pusher: true,
            registered: true,
            gateway: true,
            endpoint: true,
            distributors: Some(1),
        };
        assert_eq!(derive_state(&on), "on");
        assert_eq!(derive_state(&Facts { distributors: Some(0), ..on }), "no-distributor");
        let unknown = Facts { distributors: None, ..on };
        assert_eq!(derive_state(&unknown), "on");
        let off = Facts {
            pusher: false,
            registered: false,
            gateway: false,
            endpoint: false,
            distributors: Some(2),
        };
        assert_eq!(derive_state(&off), "off");
        assert_eq!(derive_state(&Facts { gateway: false, registered: false, ..on }), "needs-gateway");
    }

    #[test]
    fn only_a_failure_owes_a_banner() {
        assert!(Unshown::FilteredOut.silent());
        assert!(Unshown::Redacted.silent());
        assert!(Unshown::NotFound.silent());
        assert!(!Unshown::Failed("offline".to_owned()).silent());
    }

    #[test]
    fn aws_lc_is_the_provider_before_leghorn_asks() {
        install_crypto_provider();
        assert!(rustls::crypto::ring::default_provider().install_default().is_err());
    }

    #[test]
    fn leghorns_pusher_reads_as_ruma_s() {
        use matrix_sdk::ruma::api::client::push::{Pusher, PusherKind};
        let pusher: LeghornPusher = serde_json::from_value(json!({
            "pushkey": "https://push.example.org/up/abc",
            "app_id": APP_ID,
            "gateway": "https://push.example.org/_matrix/push/v1/notify",
            "app_display_name": "xmatic",
            "device_display_name": "phone",
            "lang": "en",
            "replaces": null,
        }))
        .unwrap();
        let ruma: Pusher = pusher.convert().unwrap();
        assert_eq!(ruma.ids.pushkey, "https://push.example.org/up/abc");
        assert_eq!(ruma.ids.app_id, APP_ID);
        let PusherKind::Http(data) = ruma.kind else {
            panic!("not an http pusher");
        };
        assert_eq!(data.url, "https://push.example.org/_matrix/push/v1/notify");
        assert_eq!(
            data.format,
            Some(matrix_sdk::ruma::push::PushFormat::EventIdOnly)
        );
    }
}

/// The pushkeys this installation set, as salted hashes, per account under a
/// salted hash of its user id: other devices and other accounts are not ours.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owned {
    salt: String,
    #[serde(default)]
    accounts: BTreeMap<String, Vec<String>>,
    /// Marks of 0.45's Web Push auth secrets; the next account to read its pushers claims them.
    #[serde(default)]
    legacy: Vec<String>,
    #[serde(skip)]
    account: String,
}

/// Separators of the three kinds of mark.
const PUSHKEY: u8 = 0;
const ACCOUNT: u8 = 1;
const LEGACY_AUTH: u8 = 2;

impl Owned {
    fn with_salt(salt: String, user: &str) -> Self {
        let mut owned = Self { salt, ..Self::default() };
        owned.account = owned.digest(ACCOUNT, user);
        owned
    }

    /// An unreadable record reads as empty: nothing is then deleted.
    pub fn load(path: &Path, user: &str) -> Self {
        let read = std::fs::read(path)
            .ok()
            .and_then(|raw| serde_json::from_slice::<Owned>(&raw).ok())
            .filter(|owned| !owned.salt.is_empty());
        match read {
            Some(mut owned) => {
                owned.account = owned.digest(ACCOUNT, user);
                owned
            }
            None => Self::with_salt(fresh_salt(), user),
        }
    }

    /// Through a temporary file; a record that keeps nothing removes the file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if self.accounts.values().all(Vec::is_empty) && self.legacy.is_empty() {
            return match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        let mut kept = self.clone();
        kept.accounts.retain(|_, marks| !marks.is_empty());
        let temporary = path.with_extension("json.new");
        std::fs::write(&temporary, serde_json::to_vec(&kept)?)?;
        std::fs::rename(&temporary, path)
    }

    fn digest(&self, kind: u8, value: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(self.salt.as_bytes());
        hash.update([kind]);
        hash.update(value.as_bytes());
        hex::encode(hash.finalize())
    }

    fn mark(&self, pushkey: &str) -> String {
        self.digest(PUSHKEY, pushkey)
    }

    fn own(&self) -> &[String] {
        self.accounts.get(&self.account).map_or(&[], Vec::as_slice)
    }

    pub fn holds(&self, pushkey: &str) -> bool {
        let mark = self.mark(pushkey);
        self.own().iter().any(|own| *own == mark)
    }

    /// Nothing for this account to remove, 0.45's marks included.
    pub fn is_empty(&self) -> bool {
        self.own().is_empty() && self.legacy.is_empty()
    }

    pub fn note(&mut self, pushkey: &str) {
        if !self.holds(pushkey) {
            let mark = self.mark(pushkey);
            self.accounts.entry(self.account.clone()).or_default().push(mark);
        }
    }

    /// This account's marks of `present` pushkeys that were not `removed`;
    /// other accounts' marks stay.
    pub fn settle(&mut self, present: &[&str], removed: &[&str]) {
        let keep: Vec<String> = present
            .iter()
            .filter(|pushkey| !removed.contains(pushkey))
            .map(|pushkey| self.mark(pushkey))
            .collect();
        if let Some(own) = self.accounts.get_mut(&self.account) {
            own.retain(|own| keep.contains(own));
        }
    }

    pub fn note_legacy(&mut self, auth: &str) {
        let mark = self.digest(LEGACY_AUTH, auth);
        if !self.legacy.contains(&mark) {
            self.legacy.push(mark);
        }
    }

    /// 0.45's pushers among `server` (pushkey, auth secret) become this account's.
    pub fn claim_legacy(&mut self, server: &[(&str, Option<&str>)]) {
        if self.legacy.is_empty() {
            return;
        }
        let found: Vec<&str> = server
            .iter()
            .filter(|(_, auth)| {
                auth.is_some_and(|auth| self.legacy.contains(&self.digest(LEGACY_AUTH, auth)))
            })
            .map(|(pushkey, _)| *pushkey)
            .collect();
        for pushkey in found {
            self.note(pushkey);
        }
        self.legacy.clear();
    }
}

/// Never fails: the salt keeps the marks from being looked up, it is no key.
fn fresh_salt() -> String {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    let read = std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .is_ok();
    if !read {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let mut hash = Sha256::new();
        hash.update(now.as_nanos().to_le_bytes());
        hash.update(std::process::id().to_le_bytes());
        bytes.copy_from_slice(&hash.finalize()[..16]);
    }
    hex::encode(bytes)
}

/// One read-modify-write of the record at a time in this process.
static OWNED_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Writes only while `live` holds: a session torn down meanwhile writes nothing.
fn update_owned(
    path: &Path,
    user: &str,
    live: &(dyn Fn() -> bool + Sync),
    change: impl FnOnce(&mut Owned),
) {
    let _serial = OWNED_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !live() {
        return;
    }
    let mut owned = Owned::load(path, user);
    change(&mut owned);
    if let Err(error) = owned.save(path) {
        log::warn!("the record of this device's pushers was not saved: {error}");
    }
}

/// Whether the record still names a pusher of `user` to delete.
pub fn owns_any(path: &Path, user: &str) -> bool {
    !Owned::load(path, user).is_empty()
}

pub fn owns(path: &Path, user: &str, pushkey: &str) -> bool {
    Owned::load(path, user).holds(pushkey)
}

/// The pushkeys on the server that are ours by the record or by name, `keep` excepted.
pub fn to_remove<'a>(
    server: &[&'a str],
    owned: &Owned,
    named: &[&str],
    keep: Option<&str>,
) -> Vec<&'a str> {
    server
        .iter()
        .copied()
        .filter(|pushkey| Some(*pushkey) != keep)
        .filter(|pushkey| owned.holds(pushkey) || named.contains(pushkey))
        .collect()
}

/// A pusher request that failed. `rejected`: the homeserver refused it, and
/// asking again unchanged gets the same answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PusherError {
    pub message: String,
    pub rejected: bool,
}

impl PusherError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self { message: message.into(), rejected: false }
    }

    fn http(context: &str, error: &matrix_sdk::HttpError) -> Self {
        let rejected = error
            .as_client_api_error()
            .is_some_and(|answer| refusing_status(answer.status_code.as_u16()));
        Self {
            message: crate::text::scrub_ids(&format!("{context}: {error}")),
            rejected,
        }
    }
}

impl From<PusherError> for String {
    fn from(error: PusherError) -> String {
        error.message
    }
}

/// A 4xx about the request itself: not a lapsed token, a timeout or a rate limit.
fn refusing_status(status: u16) -> bool {
    (400..500).contains(&status) && !matches!(status, 401 | 408 | 429)
}

/// The account whose pushers these are.
pub fn account_of(client: &matrix_sdk::Client) -> Result<String, PusherError> {
    client
        .user_id()
        .map(|user| user.to_string())
        .ok_or_else(|| PusherError::failed("no account for the pushers"))
}

/// One of this app's pushers on the server.
struct ServerPusher {
    pushkey: String,
    url: Option<String>,
    /// 0.45 put the Web Push auth secret into the pusher's data.
    auth: Option<String>,
}

async fn server_pushers(client: &matrix_sdk::Client) -> Result<Vec<ServerPusher>, PusherError> {
    use matrix_sdk::ruma::api::client::push::{get_pushers, PusherKind};

    let context = "could not read the pushers";
    let current = tokio::time::timeout(
        PUSHER_LIMIT,
        client.send(get_pushers::v3::Request::new()).with_request_config(bounded()),
    )
    .await
    .map_err(|_| timed_out(context))?
    .map_err(|error| PusherError::http(context, &error))?;
    Ok(current
        .pushers
        .into_iter()
        .filter(|existing| existing.ids.app_id == APP_ID)
        .map(|existing| {
            let (url, auth) = match &existing.kind {
                PusherKind::Http(data) => (
                    Some(data.url.clone()),
                    data.data.get("auth").and_then(Value::as_str).map(str::to_owned),
                ),
                _ => (None, None),
            };
            ServerPusher { pushkey: existing.ids.pushkey, url, auth }
        })
        .collect())
}

/// 0.45's pushers become this account's before anything is compared.
fn claim_legacy(
    owned_path: &Path,
    user: &str,
    live: &(dyn Fn() -> bool + Sync),
    ours: &[ServerPusher],
) {
    if Owned::load(owned_path, user).legacy.is_empty() {
        return;
    }
    let server: Vec<(&str, Option<&str>)> = ours
        .iter()
        .map(|pusher| (pusher.pushkey.as_str(), pusher.auth.as_deref()))
        .collect();
    update_owned(owned_path, user, live, |owned| owned.claim_legacy(&server));
}

async fn delete_pusher(client: &matrix_sdk::Client, pushkey: &str) -> Result<(), PusherError> {
    use matrix_sdk::ruma::api::client::push::{set_pusher, PusherIds};

    let request =
        set_pusher::v3::Request::delete(PusherIds::new(pushkey.to_owned(), APP_ID.to_owned()));
    let context = "could not remove the pusher";
    tokio::time::timeout(PUSHER_LIMIT, client.send(request).with_request_config(bounded()))
        .await
        .map_err(|_| timed_out(context))?
        .map(drop)
        .map_err(|error| PusherError::http(context, &error))
}

/// Sets the pusher with `gateway` unless already present, then deletes the ones
/// this installation set before for this account. `live` gates every write.
pub async fn register(
    client: &matrix_sdk::Client,
    pusher: &LeghornPusher,
    gateway: &str,
    owned_path: &Path,
    live: &(dyn Fn() -> bool + Sync),
) -> Result<(), PusherError> {
    use matrix_sdk::ruma::api::client::push::{set_pusher, Pusher};

    if !gateway_is_sound(gateway) || !gateway_is_sound(&pusher.pushkey) {
        return Err(PusherError {
            message: "the push gateway and address have to be https".to_owned(),
            rejected: true,
        });
    }
    let user = account_of(client)?;
    let ours = server_pushers(client).await?;
    claim_legacy(owned_path, &user, live, &ours);
    let in_place = ours.iter().any(|existing| {
        existing.pushkey == pusher.pushkey && existing.url.as_deref() == Some(gateway)
    });
    // Noted before the request: a set that lands unanswered is still ours.
    update_owned(owned_path, &user, live, |owned| {
        owned.note(&pusher.pushkey);
        if let Some(old) = &pusher.replaces {
            owned.note(old);
        }
    });
    if !in_place {
        let mut chosen = pusher.clone();
        chosen.gateway = Some(gateway.to_owned());
        let wanted: Pusher = chosen.convert().map_err(|error| {
            PusherError::failed(format!("Leghorn's pusher did not parse: {error}"))
        })?;
        // `append` stays false, as with the SDK's own setter.
        let context = "could not register the pusher";
        tokio::time::timeout(
            PUSHER_LIMIT,
            client.send(set_pusher::v3::Request::post(wanted)).with_request_config(bounded()),
        )
        .await
        .map_err(|_| timed_out(context))?
        .map_err(|error| PusherError::http(context, &error))?;
    }

    let mut present: Vec<&str> = ours.iter().map(|existing| existing.pushkey.as_str()).collect();
    present.push(&pusher.pushkey);
    let owned = Owned::load(owned_path, &user);
    let mut removed = Vec::new();
    for stale in to_remove(&present, &owned, &[], Some(&pusher.pushkey)) {
        if !live() {
            break;
        }
        // Kept in the record on failure; the next registration tries again.
        if delete_pusher(client, stale).await.is_ok() {
            removed.push(stale);
        }
    }
    update_owned(owned_path, &user, live, |owned| owned.settle(&present, &removed));
    Ok(())
}

/// Removes this account's pushers this installation set, `named` ones included.
/// What fails stays in the record for the account's next session.
pub async fn clear_own_pushers(
    client: &matrix_sdk::Client,
    owned_path: &Path,
    named: &[String],
    live: &(dyn Fn() -> bool + Sync),
) -> Result<(), PusherError> {
    let user = account_of(client)?;
    // Recorded first: an answer that never comes must not lose them.
    update_owned(owned_path, &user, live, |owned| {
        for pushkey in named {
            owned.note(pushkey);
        }
    });
    let ours = server_pushers(client).await?;
    claim_legacy(owned_path, &user, live, &ours);
    let present: Vec<&str> = ours.iter().map(|existing| existing.pushkey.as_str()).collect();
    let owned = Owned::load(owned_path, &user);
    let mut removed = Vec::new();
    let mut failure = None;
    for stale in to_remove(&present, &owned, &[], None) {
        if !live() {
            break;
        }
        match delete_pusher(client, stale).await {
            Ok(()) => removed.push(stale),
            Err(error) => failure = Some(error),
        }
    }
    update_owned(owned_path, &user, live, |owned| owned.settle(&present, &removed));
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The Web Push auth secrets of the old connector's registrations: 0.45's
/// pushers carry them in their data, and nothing else there names them.
pub fn legacy_auths(raw: &[u8]) -> Vec<String> {
    let Ok(state) = serde_json::from_slice::<Value>(raw) else {
        return Vec::new();
    };
    state
        .get("keys")
        .and_then(Value::as_object)
        .map(|keys| {
            keys.values()
                .filter_map(|key| key.as_str()?.split('|').nth(1))
                .filter(|auth| !auth.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Before `push.json` goes: its pushers' marks, for the next session to remove.
pub fn note_legacy_pushers(legacy_path: &Path, owned_path: &Path) {
    let auths = std::fs::read(legacy_path)
        .map(|raw| legacy_auths(&raw))
        .unwrap_or_default();
    if auths.is_empty() {
        return;
    }
    update_owned(owned_path, "", &|| true, |owned| {
        for auth in &auths {
            owned.note_legacy(auth);
        }
    });
}

/// Leghorn's file and its temporary sibling, whatever the distributor answered.
pub fn remove_registration(leghorn_file: &Path) -> std::io::Result<()> {
    for path in [leghorn_file.to_path_buf(), leghorn_file.with_extension("json.tmp")] {
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

/// Distributor and tokens of the old connector's push.json, if it has any.
pub fn legacy_registration(raw: &[u8]) -> Option<(String, Vec<String>)> {
    let state: Value = serde_json::from_slice(raw).ok()?;
    let distributor = state.get("distributor")?.as_str()?.to_owned();
    let tokens: Vec<String> = state
        .get("registrations")?
        .as_array()?
        .iter()
        .filter_map(|entry| entry.get("token")?.as_str().map(str::to_owned))
        .filter(|token| !token.is_empty())
        .collect();
    (!tokens.is_empty()).then_some((distributor, tokens))
}

/// Whether a `push.json` from the old connector holds a registration.
pub fn legacy_enabled(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .and_then(|state| state.get("registrations")?.as_array().map(|list| !list.is_empty()))
        .unwrap_or(false)
}

/// Gives the old connector's tokens back to its distributor. One no longer
/// installed has nothing to give back to.
pub async fn legacy_unregister(path: &Path) -> Result<(), String> {
    use std::collections::HashMap;
    use zbus::zvariant::Value as BusValue;

    let Some((distributor, tokens)) =
        std::fs::read(path).ok().and_then(|raw| legacy_registration(&raw))
    else {
        return Ok(());
    };
    let installed = leghorn::distributors()
        .await
        .map_err(|error| crate::text::scrub_ids(&error.to_string()))?;
    if !installed.contains(&distributor) {
        return Ok(());
    }
    let connection = zbus::Connection::session()
        .await
        .map_err(|error| format!("no session bus: {error}"))?;
    for token in &tokens {
        let mut args: HashMap<&str, BusValue<'_>> = HashMap::new();
        args.insert("token", BusValue::from(token.as_str()));
        let body = (args,);
        let call = connection.call_method(
            Some(distributor.as_str()),
            "/org/unifiedpush/Distributor",
            Some("org.unifiedpush.Distributor2"),
            "Unregister",
            &body,
        );
        match tokio::time::timeout(Duration::from_secs(5), call).await {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                return Err(format!("the old registration was not given back: {error}"))
            }
            Err(_) => return Err("the distributor did not answer for the old registration".to_owned()),
        }
    }
    Ok(())
}

/// Why a push gets no banner of its own. Only `Failed` still owes one.
#[derive(Debug, PartialEq, Eq)]
pub enum Unshown {
    /// The push rules say not to show it.
    FilteredOut,
    Redacted,
    /// Not on the server, or in a room this account does not have.
    NotFound,
    Failed(String),
}

impl Unshown {
    pub fn silent(&self) -> bool {
        !matches!(self, Unshown::Failed(_))
    }

    pub fn reason(&self) -> String {
        match self {
            Unshown::FilteredOut => "filtered out".to_owned(),
            Unshown::Redacted => "redacted".to_owned(),
            Unshown::NotFound => "event not found".to_owned(),
            Unshown::Failed(reason) => reason.clone(),
        }
    }
}

/// Fetches the message a push named. `NotificationClient` reaches an event no
/// sync brought in; `MultipleProcesses` only where no sync service runs.
pub async fn notification_for(
    client: &matrix_sdk::Client,
    room_id: &str,
    event_id: &str,
    sync: Option<std::sync::Arc<matrix_sdk_ui::sync_service::SyncService>>,
) -> Result<Value, Unshown> {
    use matrix_sdk::ruma::{EventId, RoomId};
    use matrix_sdk_ui::notification_client::{
        Error as NotificationError, NotificationClient, NotificationEvent,
        NotificationProcessSetup, NotificationStatus,
    };

    let room = RoomId::parse(room_id).map_err(|_| Unshown::NotFound)?;
    let event = EventId::parse(event_id).map_err(|_| Unshown::NotFound)?;

    // One process: `MultipleProcesses` mints a dummy permit and starts a second
    // encryption sync beside the running one, both claiming to-device events.
    let setup = match sync {
        Some(sync_service) => NotificationProcessSetup::SingleProcess { sync_service },
        None => NotificationProcessSetup::MultipleProcesses,
    };
    let notifications = NotificationClient::new(client.clone(), setup)
        .await
        .map_err(|error| {
            Unshown::Failed(crate::text::scrub_ids(&format!(
                "could not open the notification client: {error}"
            )))
        })?;

    match notifications.get_notification(&room, &event).await {
        Ok(NotificationStatus::Event(item)) => {
            let (kind, text) = notification_body(&item.event);
            let timestamp = match &item.event {
                NotificationEvent::Timeline(timeline) => {
                    u64::from(timeline.origin_server_ts().get())
                }
                NotificationEvent::Invite(_) => 0,
            };
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
                // A room tag, not a push rule; the banner decision reads it.
                "lowPriority": client.get_room(&room).is_some_and(|room| room.is_low_priority()),
                // Which message this banner announces, for the sync's banner.
                "timestamp": timestamp,
            }))
        }
        Ok(NotificationStatus::EventFilteredOut) => Err(Unshown::FilteredOut),
        Ok(NotificationStatus::EventRedacted) => Err(Unshown::Redacted),
        Ok(NotificationStatus::EventNotFound) => Err(Unshown::NotFound),
        Err(
            NotificationError::UnknownRoom
            | NotificationError::SlidingSyncEmptyRoom
            | NotificationError::ContextMissingEvent,
        ) => Err(Unshown::NotFound),
        // The SDK's `/context` fallback: a forged event id ends here, not above.
        Err(NotificationError::SdkError(error)) if absent_on_server(&error) => {
            Err(Unshown::NotFound)
        }
        Err(error) => Err(Unshown::Failed(crate::text::scrub_ids(&format!(
            "could not fetch the message: {error}"
        )))),
    }
}

/// The homeserver does not have the event, or not for this account.
fn absent_on_server(error: &matrix_sdk::Error) -> bool {
    error
        .as_client_api_error()
        .is_some_and(|answer| absent_answer(answer.status_code.as_u16(), answer.error_kind()))
}

fn absent_answer(
    status: u16,
    kind: Option<&matrix_sdk::ruma::api::error::ErrorKind>,
) -> bool {
    use matrix_sdk::ruma::api::error::ErrorKind;
    status == 404 || matches!(kind, Some(ErrorKind::NotFound | ErrorKind::Forbidden))
}

/// The preview a banner shows, in the same two fields the room list produces —
/// so a push and an ordinary arrival read identically.
fn notification_body(
    event: &matrix_sdk_ui::notification_client::NotificationEvent,
) -> (&'static str, String) {
    use matrix_sdk::ruma::events::poll::unstable_start::UnstablePollStartEventContent;
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
        // The question alone: the fallback text carries the answers as well and
        // would spill them into the banner.
        Some(AnyMessageLikeEventContent::UnstablePollStart(
            UnstablePollStartEventContent::New(started),
        )) => ("poll", crate::text::strip_bidi(&started.poll_start.question.text)),
        // Encrypted here means it could not be decrypted: the keys for it never
        // reached this device. The banner says so rather than counting it.
        Some(AnyMessageLikeEventContent::RoomEncrypted(_)) | None => ("encrypted", String::new()),
        _ => ("", String::new()),
    }
}
