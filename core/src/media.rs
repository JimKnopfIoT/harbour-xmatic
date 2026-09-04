//! Fetching and sending attachments. Media keys live in the event, not in the
//! URL, so the front end passes the source back as opaque JSON.

use std::path::{Path, PathBuf};
use std::time::Duration;

use matrix_sdk::{
    attachment::{
        AttachmentConfig as RoomAttachmentConfig, AttachmentInfo, BaseAudioInfo, BaseFileInfo,
        BaseImageInfo, BaseVideoInfo,
    },
    media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings, UniqueKey},
    ruma::{
        api::client::media::get_media_config,
        events::room::message::{RoomMessageEventContent, TextMessageEventContent},
        events::room::MediaSource,
        EventId, RoomId, UInt,
    },
    Client,
};
use matrix_sdk_ui::timeline::{AttachmentConfig, AttachmentSource, Timeline};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Size the timeline asks for. Generous enough for a phone screen, small
/// enough that scrolling does not pull megabytes per row.
const THUMBNAIL_EDGE: u32 = 800;

/// Largest attachment this app puts in memory to send: generous next to what a
/// homeserver accepts, far below what a phone loses to one allocation.
const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

/// Largest avatar. A profile picture is scaled down by everything that shows
/// it, so anything past this is a mistake rather than an intention.
pub const MAX_AVATAR_BYTES: u64 = 10 * 1024 * 1024;

/// Largest attachment this app will take in. Both halves of the check use it:
/// what the event declares, and what the download actually weighed.
const MAX_MEDIA_BYTES: u64 = 100 * 1024 * 1024;

/// How much disk the downloaded media may take before the oldest go. It is the
/// decrypted content of encrypted rooms, and dropping one costs a re-download.
const MAX_CACHE_BYTES: u64 = 256 * 1024 * 1024;

/// Downloads between two sweeps: the sweep reads the directory, and a handful
/// of files cannot cross the budget on their own.
const SWEEP_EVERY: usize = 16;

static SINCE_SWEEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Downloads in flight at once. One per row against a 429 is a queue this app
/// builds for itself, and the SDK nurses each for a quarter of an hour.
const FETCH_LANES: usize = 4;

static LANES: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(FETCH_LANES);

/// Drops the oldest files until the directory is inside its budget. Everything
/// here can be fetched again; what the user kept was copied out.
fn sweep_cache(directory: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, std::path::PathBuf)> = Vec::new();
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        let Ok(data) = entry.metadata() else { continue };
        if !data.is_file() {
            continue;
        }
        // A download in flight is not a cache entry: taking it makes the rename
        // that follows fail as "could not store media".
        if entry
            .file_name()
            .to_string_lossy()
            .contains(".part")
        {
            continue;
        }
        let age = data.modified().unwrap_or(std::time::UNIX_EPOCH);
        total = total.saturating_add(data.len());
        files.push((age, data.len(), entry.path()));
    }
    if total <= MAX_CACHE_BYTES {
        return;
    }
    files.sort_by_key(|(age, _, _)| *age);
    for (_, size, path) in files {
        if total <= MAX_CACHE_BYTES {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

/// How big a file is, without opening it.
pub fn file_size(path: &str) -> Result<u64, String> {
    std::fs::metadata(path)
        .map(|data| data.len())
        .map_err(|error| format!("could not read the file: {error}"))
}

/// Refuses a file too large to hold in memory, before it is read: every path
/// here reads its file in one piece.
pub fn check_size(size: u64, limit: u64, what: &str) -> Result<(), String> {
    if size > limit {
        return Err(format!(
            "{what} too large: {} MiB, the limit is {} MiB",
            size / (1024 * 1024),
            limit / (1024 * 1024)
        ));
    }
    Ok(())
}

/// A file name that survives a file system: sha256 of the SDK's media key. The
/// old cut key dropped the format suffix and merged thumbnail with original.
fn cache_name(request: &MediaRequestParameters) -> String {
    let digest = Sha256::digest(request.unique_key().as_bytes());
    format!("{}.bin", hex::encode(digest))
}

/// True for a name this module writes today.
fn is_cache_name(name: &str) -> bool {
    let Some(digest) = name.strip_suffix(".bin") else {
        return false;
    };
    digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit())
}

/// Drops what the old naming left behind: unreachable now, and the sweep would
/// only reach them once the whole directory is over budget.
fn drop_legacy_names(directory: &Path) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            if !entry.metadata().map(|data| data.is_file()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_cache_name(&name) && !name.contains(".part") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    });
}

/// Downloads media and returns the path. Already downloaded files are reused,
/// so the timeline can ask repeatedly while scrolling.
pub async fn fetch(
    client: &Client,
    cache_dir: &Path,
    source: Value,
    thumbnail: bool,
    declared: u64,
) -> Result<String, String> {
    // The parse error is not passed on: serde quotes the input, and the input is a
    // media address including the homeserver.
    let source: MediaSource = serde_json::from_value(source)
        .map_err(|_| "media source is neither a plain address nor an encrypted file".to_owned())?;

    // Encrypted media has no server-side thumbnail and cannot have one. The SDK
    // ignores the request, so asking filed the same bytes under a second name.
    let encrypted = matches!(source, MediaSource::Encrypted(_));
    let format = if thumbnail && !encrypted {
        let edge = UInt::from(THUMBNAIL_EDGE);
        MediaFormat::Thumbnail(MediaThumbnailSettings::new(edge, edge))
    } else {
        MediaFormat::File
    };

    let request = MediaRequestParameters { source, format };

    std::fs::create_dir_all(cache_dir)
        .map_err(|error| format!("media cache unavailable: {error}"))?;
    drop_legacy_names(cache_dir);
    let path: PathBuf = cache_dir.join(cache_name(&request));

    if path.is_file() {
        return Ok(path.to_string_lossy().into_owned());
    }

    // The event's own figure, before anything is asked for - the SDK has no way
    // to stream. A gate, not a guarantee: what arrives is weighed below.
    check_size(declared, MAX_MEDIA_BYTES, "attachment")?;

    // Held for the request and released with it. A closed semaphore never
    // happens here - nothing closes it - so the error is mapped, not expected.
    let _lane = LANES
        .acquire()
        .await
        .map_err(|_| "the download queue is closed".to_owned())?;

    let bytes = client
        .media()
        .get_media_content(&request, false)
        .await
        .map_err(|error| format!("download failed: {error}"))?;

    // A ceiling before the bytes reach a decoder: the decoders on this old Qt are
    // the real target, and a hundred megabytes is past any genuine thumbnail.
    check_size(bytes.len() as u64, MAX_MEDIA_BYTES, "attachment")?;

    // Write beside the target and rename, so a cancelled download is never
    // mistaken for a complete file. The counter: the same media, asked twice.
    static ATTEMPT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let attempt = ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial = path.with_extension(format!("part{attempt}"));
    std::fs::write(&partial, bytes).map_err(|error| format!("could not write media: {error}"))?;
    // Decrypted content of an end-to-end encrypted room: owner-only, like the
    // session file. The umask alone would leave it 0644.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&partial, &path).map_err(|error| format!("could not store media: {error}"))?;

    // Every so often, and only after the file this call was asked for is in
    // place: the sweep must never drop the one the caller is about to open.
    if SINCE_SWEEP.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % SWEEP_EVERY == 0 {
        if let Some(directory) = path.parent() {
            sweep_cache(directory);
        }
    }

    Ok(path.to_string_lossy().into_owned())
}

/// What this app can say about a file it sends. The SDK writes an empty `info`
/// when handed none, and the variant has to match the media type or it is silence.
fn attachment_info(
    mime: &mime::Mime,
    size: usize,
    dimensions: Option<(u64, u64)>,
    voice: Option<u64>,
) -> AttachmentInfo {
    let size = UInt::try_from(size as u64).ok();
    let (width, height) = match dimensions {
        Some((width, height)) => (UInt::try_from(width).ok(), UInt::try_from(height).ok()),
        None => (None, None),
    };

    // A recording of one's own is a voice message, not an audio file: one marker
    // (MSC3245) plus the length, and the bridges make a native note only of that.
    if let Some(duration) = voice {
        return AttachmentInfo::Voice(BaseAudioInfo {
            duration: Some(Duration::from_millis(duration)),
            size,
            waveform: None,
        });
    }

    match mime.type_() {
        mime::IMAGE => AttachmentInfo::Image(BaseImageInfo {
            height,
            width,
            size,
            blurhash: None,
            is_animated: None,
        }),
        mime::VIDEO => AttachmentInfo::Video(BaseVideoInfo {
            duration: None,
            height,
            width,
            size,
            blurhash: None,
        }),
        mime::AUDIO => AttachmentInfo::Audio(BaseAudioInfo {
            duration: None,
            size,
            waveform: None,
        }),
        _ => AttachmentInfo::File(BaseFileInfo { size }),
    }
}

/// Sends a file to a room that is not the open one. Forwarding re-uploads: the
/// target room encrypts under its own keys.
pub async fn forward_file(
    client: &Client,
    room_id: &str,
    path: &str,
    mime_type: &str,
    dimensions: Option<(u64, u64)>,
) -> Result<(), String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())?;

    let mime: mime::Mime = mime_type
        .parse()
        .map_err(|_| format!("not a media type: {mime_type}"))?;

    let local = path.trim_start_matches("file://");
    check_size(file_size(local)?, MAX_ATTACHMENT_BYTES, "attachment")?;
    let data = std::fs::read(local).map_err(|error| format!("could not read the file: {error}"))?;
    let filename = std::path::Path::new(local)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_owned());

    let config = RoomAttachmentConfig::default()
        .info(attachment_info(&mime, data.len(), dimensions, None));

    room.send_attachment(filename, &mime, data, config)
        .await
        .map(|_| ())
        .map_err(|error| format!("could not forward the attachment: {error}"))
}

/// Sends text to a room that is not the open one.
pub async fn forward_text(client: &Client, room_id: &str, body: String) -> Result<(), String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())?;

    // The same markers a message in the open room carries; a forward is not a
    // different kind of message.
    let content = match crate::compose::to_formatted_body(&body) {
        Some(html) => RoomMessageEventContent::text_html(body, html),
        None => RoomMessageEventContent::text_plain(body),
    };

    room.send(content)
        .await
        .map(|_| ())
        .map_err(|error| format!("could not forward the message: {error}"))
}

/// The server's maximum upload size in bytes, if it discloses one.
async fn upload_limit(client: &Client) -> Option<u64> {
    let request = get_media_config::v3::Request::new();
    let response = client.send(request).await.ok()?;
    Some(u64::from(response.upload_size))
}

/// Sends a file as an attachment, shown at once as a local echo. `caption` and
/// `reply_to` belong to this call - neither can be added afterwards.
pub async fn send(
    timeline: &Timeline,
    path: &str,
    mime_type: &str,
    caption: &str,
    reply_to: &str,
    voice: Option<u64>,
    dimensions: Option<(u64, u64)>,
) -> Result<(), String> {
    let mime: mime::Mime = mime_type
        .parse()
        .map_err(|_| format!("not a media type: {mime_type}"))?;

    // Size first, contents second: reading first means a picked file of any size
    // lands in memory whole, which on a phone is the app being killed.
    let size = file_size(path)?;
    check_size(size, MAX_ATTACHMENT_BYTES, "attachment")?;

    let bytes = std::fs::read(path).map_err(|error| format!("could not read the file: {error}"))?;
    let size = bytes.len();

    // Ask the server what it accepts before spending minutes uploading: the
    // rejection arrives as a bare "failed sending attachment".
    if let Some(limit) = upload_limit(&timeline.room().client()).await {
        if size as u64 > limit {
            return Err(format!(
                "too large for this server: {} MiB, limit is {} MiB",
                size / (1024 * 1024),
                limit / (1024 * 1024)
            ));
        }
    }
    let filename = std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_owned());

    let mut config = AttachmentConfig::default();
    config.info = Some(attachment_info(&mime, size, dimensions, voice));

    let caption = caption.trim();
    if !caption.is_empty() {
        config.caption = Some(TextMessageEventContent::plain(caption));
    }
    if !reply_to.is_empty() {
        // Refused rather than silently sent bare: an attachment that lost its reply
        // looks like an answer to nothing.
        config.in_reply_to = Some(
            EventId::parse(reply_to)
                .map_err(|_| "the message being answered is not known".to_owned())?,
        );
    }

    timeline
        .send_attachment(AttachmentSource::Data { bytes, filename }, mime, config)
        .await
        .map(|_| ())
        .map_err(|error| {
            // The SDK's Display is one short phrase; Debug carries the cause,
            // and the size matters because servers cap uploads.
            format!(
                "upload of {} KiB rejected: {error:?}",
                size / 1024
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::{events::room::MediaSource, OwnedMxcUri};

    fn plain(uri: &str, thumbnail: bool) -> MediaRequestParameters {
        let edge = UInt::from(THUMBNAIL_EDGE);
        MediaRequestParameters {
            source: MediaSource::Plain(OwnedMxcUri::from(uri)),
            format: if thumbnail {
                MediaFormat::Thumbnail(MediaThumbnailSettings::new(edge, edge))
            } else {
                MediaFormat::File
            },
        }
    }

    #[test]
    fn the_name_has_one_length_whatever_goes_in() {
        let short = cache_name(&plain("mxc://example.org/abc", false));
        let long = cache_name(&plain(&format!("mxc://example.org/{}", "a".repeat(4000)), false));
        assert_eq!(short.len(), 68);
        assert_eq!(long.len(), short.len());
        assert!(is_cache_name(&short) && is_cache_name(&long));
    }

    #[test]
    fn a_long_id_does_not_merge_thumbnail_and_file() {
        // What the old name did: the format sat at the end of the key and the
        // cut to 120 characters took it off, so both of these were one file.
        let uri = format!("mxc://example.org/{}", "b".repeat(200));
        assert_ne!(cache_name(&plain(&uri, true)), cache_name(&plain(&uri, false)));
    }

    #[test]
    fn a_shared_prefix_is_not_a_shared_name() {
        let mine = format!("mxc://example.org/{}", "c".repeat(200));
        let theirs = format!("{mine}x");
        assert_ne!(cache_name(&plain(&mine, false)), cache_name(&plain(&theirs, false)));
    }

    #[test]
    fn the_old_names_are_not_taken_for_current_ones() {
        assert!(!is_cache_name("mxc___example_org_abc_file"));
        assert!(!is_cache_name(&"a".repeat(64)));
        assert!(!is_cache_name(&format!("{}.bin", "z".repeat(64))));
        assert!(is_cache_name(&format!("{}.bin", "0".repeat(64))));
    }
}
