//! The user's own profile: display name and avatar.
//!
//! Small by design — reading is one request, writing is one or two. The
//! avatar upload reads the picked file from disk, uploads it and sets the
//! resulting `mxc:` URL in a single step via the SDK.

use matrix_sdk::Client;
use serde_json::{json, Value};

/// The user's display name and avatar URL, as the server reports them.
pub async fn get(client: &Client) -> Result<Value, String> {
    let profile = client
        .account()
        .fetch_user_profile()
        .await
        .map_err(|error| format!("profile unavailable: {error}"))?;

    // The response is a generic field map since profiles became extensible;
    // the two classic fields are all this client shows.
    let field = |name: &str| -> String {
        profile
            .get(name)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned()
    };

    Ok(json!({
        "displayName": field("displayname"),
        "avatarUrl": field("avatar_url"),
    }))
}

/// Changes the display name. An empty name removes it.
pub async fn set_display_name(client: &Client, name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    let value = if trimmed.is_empty() { None } else { Some(trimmed) };
    client
        .account()
        .set_display_name(value)
        .await
        .map_err(|error| format!("could not change the name: {error}"))
}

/// Uploads a picture from disk and makes it the avatar.
pub async fn set_avatar(client: &Client, path: &str) -> Result<Value, String> {
    let data = tokio::fs::read(path)
        .await
        .map_err(|error| format!("could not read the picture: {error}"))?;

    let mime = match path.rsplit('.').next().map(str::to_ascii_lowercase) {
        Some(ext) if ext == "png" => mime::IMAGE_PNG,
        Some(ext) if ext == "gif" => mime::IMAGE_GIF,
        Some(ext) if ext == "webp" => "image/webp".parse().expect("static mime is valid"),
        _ => mime::IMAGE_JPEG,
    };

    let url = client
        .account()
        .upload_avatar(&mime, data)
        .await
        .map_err(|error| format!("could not set the avatar: {error}"))?;

    Ok(json!({ "avatarUrl": url.to_string() }))
}
