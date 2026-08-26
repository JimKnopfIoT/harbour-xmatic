//! Fetching and sending attachments.
//!
//! Media in encrypted rooms is encrypted too, and its keys live in the event,
//! not in the URL. The front end therefore never handles an MXC URI directly:
//! it passes the event's media source back verbatim as opaque JSON, and this
//! module turns it into bytes on disk. That keeps decryption and caching in one
//! place and the Qt side free of crypto.

use std::path::{Path, PathBuf};
use std::time::Duration;

use matrix_sdk::{
    attachment::{AttachmentConfig as RoomAttachmentConfig, AttachmentInfo, BaseAudioInfo},
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

/// Size the timeline asks for. Generous enough for a phone screen, small
/// enough that scrolling does not pull megabytes per row.
const THUMBNAIL_EDGE: u32 = 800;

/// Largest attachment this app will put in memory to send. Generous next to
/// what a homeserver accepts (matrix.org allows 50 MiB), and far below what a
/// phone can lose to a single allocation.
const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

/// Largest avatar. A profile picture is scaled down by everything that shows
/// it, so anything past this is a mistake rather than an intention.
pub const MAX_AVATAR_BYTES: u64 = 10 * 1024 * 1024;

/// Largest attachment this app will take in. Both halves of the check use it:
/// what the event declares, and what the download actually weighed.
const MAX_MEDIA_BYTES: u64 = 100 * 1024 * 1024;

/// How big a file is, without opening it.
pub fn file_size(path: &str) -> Result<u64, String> {
    std::fs::metadata(path)
        .map(|data| data.len())
        .map_err(|error| format!("could not read the file: {error}"))
}

/// Refuses a file that is too large to hold in memory, before it is read.
///
/// Every one of these paths reads its file in one piece — the SDK's attachment
/// API wants the bytes, not a path — so the only place to stop an outsized file
/// is before the read, not after it.
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
    declared: u64,
) -> Result<String, String> {
    // The parse error is deliberately not passed on: serde quotes the input it
    // choked on, and the input is a media address including the homeserver —
    // an identifier, and this project logs none of those in full.
    let source: MediaSource = serde_json::from_value(source)
        .map_err(|_| "media source is neither a plain address nor an encrypted file".to_owned())?;

    // Encrypted media has no server-side thumbnail and cannot have one — the
    // homeserver only ever sees ciphertext. The SDK honours
    // `MediaFormat::Thumbnail` on the plain arm alone and silently ignores it
    // for an encrypted file, downloading and decrypting the original either
    // way. Asking for a thumbnail regardless did not shrink a single byte; it
    // only filed the very same bytes under a second cache name, so every
    // encrypted picture was stored twice. Saying so here keeps the request and
    // the file that comes back describing the same thing.
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
    let path: PathBuf = cache_dir.join(cache_name(&request));

    if path.is_file() {
        return Ok(path.to_string_lossy().into_owned());
    }

    // The event's own figure, before anything is asked for. The SDK hands
    // media over as one `Vec<u8>` and offers no way to stream it - its
    // `get_media_file` calls the same function and writes the result - so the
    // only place a download can be stopped without holding it in memory first
    // is here, before the request goes out.
    //
    // The sender writes that figure, so this is a gate and not a guarantee:
    // whoever lies about it still runs into the check below, which weighs what
    // actually arrived. Two halves of one protection - the first costs the
    // attacker the download, the second costs us the memory.
    check_size(declared, MAX_MEDIA_BYTES, "attachment")?;

    let bytes = client
        .media()
        .get_media_content(&request, false)
        .await
        .map_err(|error| format!("download failed: {error}"))?;

    // A ceiling before the bytes ever reach a decoder. A remote party sets both
    // the size and the content of an attachment, so an unbounded download is a
    // memory-exhaustion lever; the image and video decoders on this old Qt are
    // the real target, and a hundred megabytes is already far past any genuine
    // thumbnail or clip.
    check_size(bytes.len() as u64, MAX_MEDIA_BYTES, "attachment")?;

    // Write beside the target and rename, so a cancelled download can never be
    // mistaken for a complete file.
    let partial = path.with_extension("part");
    std::fs::write(&partial, bytes).map_err(|error| format!("could not write media: {error}"))?;
    // Decrypted content of an end-to-end encrypted room: owner-only, like the
    // session file. The umask alone would leave it 0644.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600));
    }
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
    check_size(file_size(local)?, MAX_ATTACHMENT_BYTES, "attachment")?;
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
///
/// `caption` and `reply_to` are both optional and both belong to this call:
/// the caption travels inside the media event, and a reply relation cannot be
/// added to an event that was already sent. An empty string means "not set"
/// for either.
pub async fn send(
    timeline: &Timeline,
    path: &str,
    mime_type: &str,
    caption: &str,
    reply_to: &str,
    voice: Option<u64>,
) -> Result<(), String> {
    let mime: mime::Mime = mime_type
        .parse()
        .map_err(|_| format!("not a media type: {mime_type}"))?;

    // Size first, contents second. Reading before asking how big the file is
    // means a picked file of any size lands in memory in one piece, and on a
    // phone that is the app being killed rather than an error the user can act
    // on. The server's own limit is asked for below, but only after this: that
    // question needs the network, and there is no reason to hold a gigabyte in
    // memory while waiting for the answer.
    let size = file_size(path)?;
    check_size(size, MAX_ATTACHMENT_BYTES, "attachment")?;

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

    let mut config = AttachmentConfig::default();

    // A recording of one's own is a voice message, not an audio file. The
    // difference is one marker (MSC3245) plus the length, and it is not
    // cosmetic: clients draw a voice message differently, and the bridges to
    // other networks make a native voice note only out of a message that
    // carries it - one of them refuses everything else as an unsupported
    // format, which reads as if the recording were broken.
    if let Some(duration) = voice {
        config.info = Some(AttachmentInfo::Voice(BaseAudioInfo {
            duration: Some(Duration::from_millis(duration)),
            size: UInt::try_from(size as u64).ok(),
            waveform: None,
        }));
    }

    let caption = caption.trim();
    if !caption.is_empty() {
        config.caption = Some(TextMessageEventContent::plain(caption));
    }
    if !reply_to.is_empty() {
        // Refused rather than silently sent bare: an attachment that lost its
        // reply looks like an answer to nothing, and the user cannot tell that
        // from a working one.
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
