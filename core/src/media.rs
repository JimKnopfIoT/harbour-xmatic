//! Fetching and sending attachments.
//!
//! Media in encrypted rooms is encrypted too, and its keys live in the event,
//! not in the URL. The front end therefore never handles an MXC URI directly:
//! it passes the event's media source back verbatim as opaque JSON, and this
//! module turns it into bytes on disk. That keeps decryption and caching in one
//! place and the Qt side free of crypto.

use std::path::{Path, PathBuf};

use matrix_sdk::{
    attachment::AttachmentConfig as RoomAttachmentConfig,
    media::{MediaFormat, MediaRequestParameters, MediaThumbnailSettings, UniqueKey},
    ruma::{
        api::client::media::get_media_config, events::room::message::RoomMessageEventContent,
        events::room::MediaSource, RoomId, UInt,
    },
    Client,
};
use matrix_sdk_ui::timeline::{AttachmentConfig, AttachmentSource, Timeline};
use serde_json::Value;

/// Size the timeline asks for. Generous enough for a phone screen, small
/// enough that scrolling does not pull megabytes per row.
const THUMBNAIL_EDGE: u32 = 800;

/// Turns a request into a file name that survives a file system.
fn cache_name(request: &MediaRequestParameters) -> String {
    let key = request.unique_key();
    let mut name: String = key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    // Keep it well below any path limit; the key stays unique enough because
    // the media id itself is a long random string.
    name.truncate(120);
    name
}

/// Downloads media and returns the path it was written to.
///
/// Already downloaded files are reused, so the timeline can ask repeatedly
/// while scrolling.
pub async fn fetch(
    client: &Client,
    cache_dir: &Path,
    source: Value,
    thumbnail: bool,
) -> Result<String, String> {
    // The parse error is deliberately not passed on: serde quotes the input it
    // choked on, and the input is a media address including the homeserver —
    // an identifier, and this project logs none of those in full.
    let source: MediaSource = serde_json::from_value(source)
        .map_err(|_| "media source is neither a plain address nor an encrypted file".to_owned())?;

    let format = if thumbnail {
        let edge = UInt::from(THUMBNAIL_EDGE);
        MediaFormat::Thumbnail(MediaThumbnailSettings::new(edge, edge))
    } else {
        MediaFormat::File
    };

    let request = MediaRequestParameters { source, format };

    std::fs::create_dir_all(cache_dir)
        .map_err(|error| format!("media cache unavailable: {error}"))?;
    let path: PathBuf = cache_dir.join(cache_name(&request));

    if path.is_file() {
        return Ok(path.to_string_lossy().into_owned());
    }

    let bytes = client
        .media()
        .get_media_content(&request, true)
        .await
        .map_err(|error| format!("download failed: {error}"))?;

    // A ceiling before the bytes ever reach a decoder. A remote party sets both
    // the size and the content of an attachment, so an unbounded download is a
    // memory-exhaustion lever; the image and video decoders on this old Qt are
    // the real target, and a hundred megabytes is already far past any genuine
    // thumbnail or clip.
    const MAX_MEDIA_BYTES: usize = 100 * 1024 * 1024;
    if bytes.len() > MAX_MEDIA_BYTES {
        return Err(format!(
            "attachment too large: {} MiB",
            bytes.len() / (1024 * 1024)
        ));
    }

    // Write beside the target and rename, so a cancelled download can never be
    // mistaken for a complete file.
    let partial = path.with_extension("part");
    std::fs::write(&partial, bytes).map_err(|error| format!("could not write media: {error}"))?;
    std::fs::rename(&partial, &path).map_err(|error| format!("could not store media: {error}"))?;

    Ok(path.to_string_lossy().into_owned())
}

/// Sends a file from disk to a room that is not the open one.
///
/// Forwarding re-uploads: the picture was decrypted for display, and the
/// target room encrypts under its own keys, so the bytes have to travel again.
pub async fn forward_file(
    client: &Client,
    room_id: &str,
    path: &str,
    mime_type: &str,
) -> Result<(), String> {
    let parsed = RoomId::parse(room_id).map_err(|_| "not a room identifier".to_owned())?;
    let room = client
        .get_room(&parsed)
        .ok_or_else(|| "room is not known".to_owned())?;

    let mime: mime::Mime = mime_type
        .parse()
        .map_err(|_| format!("not a media type: {mime_type}"))?;

    let local = path.trim_start_matches("file://");
    let data = std::fs::read(local).map_err(|error| format!("could not read the file: {error}"))?;
    let filename = std::path::Path::new(local)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".to_owned());

    room.send_attachment(filename, &mime, data, RoomAttachmentConfig::default())
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

    room.send(RoomMessageEventContent::text_plain(body))
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

/// Sends a file from disk as an attachment. The timeline shows it immediately
/// as a local echo.
pub async fn send(timeline: &Timeline, path: &str, mime_type: &str) -> Result<(), String> {
    let mime: mime::Mime = mime_type
        .parse()
        .map_err(|_| format!("not a media type: {mime_type}"))?;

    // Read here rather than handing the path over, so an unreadable file and a
    // rejected upload cannot produce the same message.
    let bytes = std::fs::read(path).map_err(|error| format!("could not read the file: {error}"))?;
    let size = bytes.len();

    // Ask the server what it accepts before spending minutes uploading. The
    // rejection itself arrives as a bare "failed sending attachment", which
    // tells the user nothing about why.
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

    timeline
        .send_attachment(
            AttachmentSource::Data { bytes, filename },
            mime,
            AttachmentConfig::default(),
        )
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
