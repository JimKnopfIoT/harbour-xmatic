//! Locations: a one-off `m.location` and a live share (MSC3489). The phone's
//! position comes from the Qt side; this module sends, reads rows and fetches
//! map tiles. Coordinates are never logged, not even rounded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use matrix_sdk::config::RequestConfig;
use matrix_sdk::ruma::events::beacon::BeaconEventContent;
use matrix_sdk::ruma::events::beacon_info::BeaconInfoEventContent;
use matrix_sdk::ruma::events::location::AssetType;
use matrix_sdk::ruma::events::room::message::{
    LocationMessageEventContent, MessageType, RoomMessageEventContent,
};
use matrix_sdk::ruma::{MilliSecondsSinceUnixEpoch, OwnedEventId, OwnedUserId, RoomId};
use matrix_sdk::Client;
use matrix_sdk_ui::timeline::{LiveLocationState, Timeline};
use serde_json::{json, Value};

use crate::protocol::{reply_error, reply_ok, Command};
use crate::runtime::Sink;
use crate::text::strip_bidi;

/// Zoom of the card: streets and their names, a few hundred metres across.
const ZOOM: u32 = 16;
/// The card's window onto the map, in map pixels. 2:1 fits a bubble.
const VIEW_WIDTH: i64 = 512;
const VIEW_HEIGHT: i64 = 256;
const TILE: i64 = 256;
/// OSM's tile policy: identify the app, keep tiles at least a week.
const TILE_URL: &str = "https://tile.openstreetmap.org";
const TILE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
const TILE_MAX_BYTES: usize = 256 * 1024;
/// A live share outside these bounds is a typo or an attack on the battery.
const LIVE_MIN_MS: u64 = 60 * 1000;
const LIVE_MAX_MS: u64 = 24 * 3600 * 1000;
/// Description texts are a stranger's words in a card.
const DESCRIPTION_CHARS: usize = 200;

/// A beacon holds the room's lane; a stop waits behind it, so keep it short.
const BEACON_RETRIES: usize = 2;

/// Two lanes to the tile server: a room full of locations must not become a burst.
static TILE_LANES: LazyLock<tokio::sync::Semaphore> =
    LazyLock::new(|| tokio::sync::Semaphore::new(2));

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(concat!(
            "harbour-xmatic/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/JimKnopfIoT/harbour-xmatic)"
        ))
        .timeout(Duration::from_secs(20))
        .build()
        .unwrap_or_default()
});

/// A share this run started: its event and content, not the store's copy,
/// which only a later sync brings.
struct Share {
    user: OwnedUserId,
    event_id: OwnedEventId,
    content: BeaconInfoEventContent,
}

/// Per room: start, stop and beacon run one at a time, in arrival order.
type Lane = Arc<tokio::sync::Mutex<Option<Share>>>;
static LANES: LazyLock<Mutex<HashMap<String, Lane>>> = LazyLock::new(Default::default);

fn lane(room_id: &str) -> Lane {
    let mut lanes = LANES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    lanes.entry(room_id.to_owned()).or_default().clone()
}

static PARTIAL: AtomicU64 = AtomicU64::new(0);

/// A point as `geo:` carries it. Anything outside the globe is refused, so a
/// row never draws a marker at NaN.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
    pub accuracy: Option<f64>,
}

impl Point {
    pub fn new(lat: f64, lon: f64, accuracy: Option<f64>) -> Option<Self> {
        if !lat.is_finite() || !lon.is_finite() || lat.abs() > 90.0 || lon.abs() > 180.0 {
            return None;
        }
        let accuracy = accuracy.filter(|metres| metres.is_finite() && *metres >= 0.0);
        Some(Self { lat, lon, accuracy })
    }

    /// RFC 5870. Six decimals is ten centimetres; more claims a precision no
    /// phone has.
    pub fn geo_uri(&self) -> String {
        match self.accuracy {
            Some(metres) => format!("geo:{:.6},{:.6};u={:.0}", self.lat, self.lon, metres),
            None => format!("geo:{:.6},{:.6}", self.lat, self.lon),
        }
    }
}

/// Reads `geo:lat,lon[,alt][;u=metres][;…]`. Unknown parameters are ignored,
/// a malformed number refuses the whole URI.
pub fn parse_geo(uri: &str) -> Option<Point> {
    let rest = uri.trim().strip_prefix("geo:").or_else(|| uri.trim().strip_prefix("GEO:"))?;
    let mut parts = rest.split(';');
    let coordinates = parts.next()?;
    let mut numbers = coordinates.split(',');
    let lat: f64 = numbers.next()?.trim().parse().ok()?;
    let lon: f64 = numbers.next()?.trim().parse().ok()?;
    let mut accuracy = None;
    for parameter in parts {
        if let Some(value) = parameter.trim().strip_prefix("u=") {
            accuracy = Some(value.trim().parse::<f64>().ok()?);
        }
    }
    Point::new(lat, lon, accuracy)
}

fn point_json(point: &Point) -> Value {
    json!({ "lat": point.lat, "lon": point.lon, "accuracy": point.accuracy })
}

fn description(text: Option<&str>) -> Option<String> {
    let text = text?.trim();
    if text.is_empty() {
        return None;
    }
    Some(strip_bidi(&text.chars().take(DESCRIPTION_CHARS).collect::<String>()))
}

/// The row field for a one-off location; null where the URI is not a point.
pub fn from_message(content: &LocationMessageEventContent) -> Value {
    match parse_geo(content.geo_uri()) {
        Some(point) => {
            let mut value = point_json(&point);
            value["live"] = json!(false);
            // The sender's own position; absent means that too (MSC3488).
            value["self"] = json!(content
                .asset
                .as_ref()
                .is_none_or(|asset| asset.type_ == AssetType::Self_));
            value
        }
        None => Value::Null,
    }
}

/// The row field for a live share: the newest point, when it was taken and
/// until when the share runs. `active` is the SDK's reading at row time; the
/// UI compares `until` with its own clock, since no diff marks the expiry.
pub fn from_live(state: &LiveLocationState) -> Value {
    let latest = state.latest_location();
    let point = latest.and_then(|location| parse_geo(location.geo_uri()));
    let started = u64::from(state.ts().get());
    json!({
        "live": true,
        "self": state.asset_type() == AssetType::Self_,
        "active": state.is_live(),
        "lat": point.map(|point| point.lat),
        "lon": point.map(|point| point.lon),
        "accuracy": point.and_then(|point| point.accuracy),
        "updated": latest.map(|location| u64::from(location.ts().get())),
        "until": started.saturating_add(state.timeout().as_millis() as u64),
        "description": description(state.description()),
    })
}

/// The five location commands. Sending into the open room goes through its
/// timeline for the local echo; a live share outlives the room page, so it
/// names its room.
pub async fn handle(
    command: Command,
    client: Option<Client>,
    timeline: Option<Arc<Timeline>>,
    tile_dir: PathBuf,
    sink: &Arc<Sink>,
) {
    let id = command.id();
    let outcome = match command {
        Command::LocationSend { lat, lon, accuracy, .. } => match timeline {
            Some(timeline) => send(&timeline, lat, lon, accuracy).await,
            None => Err("no timeline is open".to_owned()),
        },
        Command::LocationLiveStart { room_id, duration_ms, .. } => {
            live_start(client, &room_id, duration_ms).await
        }
        Command::LocationBeacon { room_id, lat, lon, accuracy, .. } => {
            beacon(client, &room_id, lat, lon, accuracy).await
        }
        Command::LocationLiveStop { room_id, .. } => live_stop(client, &room_id).await,
        Command::LocationTiles { key, lat, lon, .. } => {
            sink.emit(reply_ok(id, tiles(&tile_dir, &key, lat, lon).await));
            return;
        }
        _ => Err("not a location command".to_owned()),
    };
    match outcome {
        Ok(value) => sink.emit(reply_ok(id, value)),
        Err(message) => sink.emit(reply_error(id, message)),
    }
}

async fn send(timeline: &Timeline, lat: f64, lon: f64, accuracy: Option<f64>) -> Result<Value, String> {
    let point = Point::new(lat, lon, accuracy).ok_or_else(|| "not a position".to_owned())?;
    let uri = point.geo_uri();
    // What clients without location support show: the URI, which a phone opens.
    let content = LocationMessageEventContent::new(format!("Location {uri}"), uri)
        .with_asset_type(AssetType::Self_)
        .with_ts(MilliSecondsSinceUnixEpoch::now());
    timeline
        .send(RoomMessageEventContent::new(MessageType::Location(content)).into())
        .await
        .map(|_| json!({ "done": true }))
        .map_err(|error| format!("the location could not be sent: {error}"))
}

fn joined_room(client: Option<Client>, room_id: &str) -> Result<matrix_sdk::Room, String> {
    let client = client.ok_or_else(|| "not signed in".to_owned())?;
    let id = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    client.get_room(&id).ok_or_else(|| "this room is not known".to_owned())
}

async fn live_start(client: Option<Client>, room_id: &str, duration_ms: u64) -> Result<Value, String> {
    if !(LIVE_MIN_MS..=LIVE_MAX_MS).contains(&duration_ms) {
        return Err("the sharing time is out of range".to_owned());
    }
    let room = joined_room(client, room_id)?;
    let lane = lane(room_id);
    let mut share = lane.lock().await;
    let user = room.own_user_id().to_owned();
    let content = BeaconInfoEventContent::new(None, Duration::from_millis(duration_ms), true, None);
    let response = room
        .send_state_event_for_key(&user, content.clone())
        .await
        .map_err(|error| format!("live location could not be started: {error}"))?;
    let until = u64::from(content.ts.get()).saturating_add(duration_ms);
    *share = Some(Share { user, event_id: response.event_id, content });
    Ok(json!({ "roomId": room_id, "until": until }))
}

/// The share this run started in the room, while it runs and for this account.
fn current<'a>(share: &'a Option<Share>, room: &matrix_sdk::Room) -> Option<&'a Share> {
    share
        .as_ref()
        .filter(|share| share.user == room.own_user_id() && share.content.is_live())
}

async fn beacon(
    client: Option<Client>,
    room_id: &str,
    lat: f64,
    lon: f64,
    accuracy: Option<f64>,
) -> Result<Value, String> {
    let point = Point::new(lat, lon, accuracy).ok_or_else(|| "not a position".to_owned())?;
    let room = joined_room(client, room_id)?;
    // Held across the send: a stop behind it lands after this point, never before.
    let lane = lane(room_id);
    let share = lane.lock().await;
    let Some(share) = current(&share, &room) else {
        return Err("no live share runs in this room".to_owned());
    };
    let content = BeaconEventContent::new(share.event_id.clone(), point.geo_uri(), None);
    // The error names no coordinates: it goes to the journal.
    room.send(content)
        .with_request_config(RequestConfig::new().retry_limit(BEACON_RETRIES))
        .await
        .map(|_| json!({ "roomId": room_id }))
        .map_err(|error| format!("the live location update was not sent: {error}"))
}

async fn live_stop(client: Option<Client>, room_id: &str) -> Result<Value, String> {
    let room = joined_room(client, room_id)?;
    let lane = lane(room_id);
    let mut share = lane.lock().await;
    let result = match current(&share, &room) {
        Some(running) => {
            let mut content = running.content.clone();
            content.stop();
            room.send_state_event_for_key(&running.user, content)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        // Left over from before a restart: only the store knows it.
        None => room.stop_live_location_share().await.map(|_| ()).map_err(|error| error.to_string()),
    };
    match result {
        Ok(()) => {
            *share = None;
            Ok(json!({ "roomId": room_id }))
        }
        Err(error) => Err(format!("live location could not be stopped: {error}")),
    }
}

/// Where a point lies on the map at `zoom`, in map pixels (Web Mercator).
fn world_pixel(lat: f64, lon: f64, zoom: u32) -> (f64, f64) {
    let size = (TILE as f64) * f64::from(1u32 << zoom);
    let lat = lat.clamp(-85.051_128, 85.051_128).to_radians();
    let x = (lon + 180.0) / 360.0 * size;
    let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * size;
    (x, y)
}

/// A tile the card needs: where it sits in the view, and which one it is.
#[derive(Debug, PartialEq)]
struct Placed {
    x: u32,
    y: u32,
    left: i64,
    top: i64,
}

/// The tiles under a view centred on the point. Wraps across the date line;
/// rows past the poles are left out.
fn layout(lat: f64, lon: f64, zoom: u32) -> Vec<Placed> {
    let count = 1i64 << zoom;
    let (px, py) = world_pixel(lat, lon, zoom);
    let left = px.round() as i64 - VIEW_WIDTH / 2;
    let top = py.round() as i64 - VIEW_HEIGHT / 2;
    let mut placed = Vec::new();
    for ty in top.div_euclid(TILE)..=(top + VIEW_HEIGHT - 1).div_euclid(TILE) {
        if ty < 0 || ty >= count {
            continue;
        }
        for tx in left.div_euclid(TILE)..=(left + VIEW_WIDTH - 1).div_euclid(TILE) {
            placed.push(Placed {
                x: tx.rem_euclid(count) as u32,
                y: ty as u32,
                left: tx * TILE - left,
                top: ty * TILE - top,
            });
        }
    }
    placed
}

/// Never an error reply: a card without a map shows the coordinates, and must
/// not spin. `retry` separates "the server could not be asked" from the rest.
async fn tiles(dir: &Path, key: &str, lat: f64, lon: f64) -> Value {
    let unavailable = |retry: bool| json!({ "key": key, "available": false, "retry": retry });
    if Point::new(lat, lon, None).is_none() {
        return unavailable(false);
    }
    let mut out = Vec::new();
    for tile in layout(lat, lon, ZOOM) {
        match tile_file(dir, ZOOM, tile.x, tile.y).await {
            Some(path) => out.push(json!({
                "path": path.to_string_lossy(),
                "left": tile.left,
                "top": tile.top,
            })),
            None => return unavailable(true),
        }
    }
    json!({
        "key": key,
        "available": true,
        "width": VIEW_WIDTH,
        "height": VIEW_HEIGHT,
        "tile": TILE,
        "tiles": out,
    })
}

/// The tile from the cache while it is younger than a week, else fetched.
async fn tile_file(dir: &Path, zoom: u32, x: u32, y: u32) -> Option<PathBuf> {
    let path = dir.join(zoom.to_string()).join(x.to_string()).join(format!("{y}.png"));
    if fresh(&path) {
        return Some(path);
    }
    let _lane = TILE_LANES.acquire().await.ok()?;
    // Another lane may have fetched it while this one waited.
    if fresh(&path) {
        return Some(path);
    }
    let response = HTTP.get(format!("{TILE_URL}/{zoom}/{x}/{y}.png")).send().await.ok()?;
    if !response.status().is_success() {
        tracing::warn!("map tile refused: {}", response.status());
        return None;
    }
    let is_png = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("image/png"));
    let bytes = response.bytes().await.ok()?;
    if !is_png || bytes.len() > TILE_MAX_BYTES || !bytes.starts_with(b"\x89PNG") {
        return None;
    }
    let parent = path.parent()?;
    std::fs::create_dir_all(parent).ok()?;
    // Own name per fetch: two cards may want the same tile at once.
    let partial = path.with_extension(format!(
        "{}.{}.part",
        std::process::id(),
        PARTIAL.fetch_add(1, Ordering::Relaxed)
    ));
    if std::fs::write(&partial, &bytes).is_err() {
        let _ = std::fs::remove_file(&partial);
        return None;
    }
    if std::fs::rename(&partial, &path).is_err() {
        let _ = std::fs::remove_file(&partial);
        return fresh(&path).then_some(path);
    }
    Some(path)
}

fn fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < TILE_MAX_AGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geo_uris_parse() {
        let point = parse_geo("geo:12.345678,23.456789;u=35").unwrap();
        assert_eq!((point.lat, point.lon, point.accuracy), (12.345678, 23.456789, Some(35.0)));
        let point = parse_geo("geo:-12.5,-123.5,40;crs=wgs84").unwrap();
        assert_eq!((point.lat, point.lon, point.accuracy), (-12.5, -123.5, None));
    }

    #[test]
    fn nonsense_is_refused() {
        for uri in [
            "geo:",
            "geo:91,0",
            "geo:0,181",
            "geo:NaN,0",
            "geo:inf,0",
            "geo:1",
            "geo:1,2;u=x",
            "https://example.org",
        ] {
            assert!(parse_geo(uri).is_none(), "{uri}");
        }
        assert!(Point::new(0.0, 0.0, Some(f64::NAN)).unwrap().accuracy.is_none());
    }

    #[test]
    fn the_uri_round_trips() {
        let point = Point::new(34.567891, 45.678912, Some(12.4)).unwrap();
        assert_eq!(point.geo_uri(), "geo:34.567891,45.678912;u=12");
        assert_eq!(parse_geo(&point.geo_uri()).unwrap().lat, 34.567891);
    }

    #[test]
    fn the_view_is_covered_and_centred() {
        let tiles = layout(0.0, 0.0, 2);
        // The equator at zoom 2 sits on a tile edge: 2 x 2 tiles, point in the middle.
        assert_eq!(tiles.len(), 4);
        assert!(tiles.iter().all(|tile| tile.left > -TILE && tile.left < VIEW_WIDTH));
        assert!(tiles.iter().all(|tile| tile.top > -TILE && tile.top < VIEW_HEIGHT));
        let (px, py) = world_pixel(0.0, 0.0, 2);
        assert_eq!((px, py.round()), (512.0, 512.0));
    }

    #[test]
    fn the_date_line_wraps() {
        let tiles = layout(0.0, 179.99, 3);
        assert!(tiles.iter().any(|tile| tile.x == 0));
        assert!(tiles.iter().all(|tile| tile.x < 8));
    }

    #[test]
    fn the_pole_drops_rows() {
        let tiles = layout(85.05, 0.0, 3);
        assert!(tiles.iter().all(|tile| tile.y < 8));
        assert!(!tiles.is_empty());
    }
}
