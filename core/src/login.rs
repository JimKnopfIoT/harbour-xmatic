//! OAuth 2.0 authorization code login, the way Element X does it.
//!
//! matrix.org has moved to the Matrix Authentication Service, so there is no
//! password endpoint to talk to: the user authenticates in a browser and the
//! authorization server redirects back with a code.
//!
//! The redirect goes to a loopback listener the SDK spawns for us
//! (RFC 8252 §7.3) rather than to a custom `xmatic://` URI scheme. That avoids
//! registering a scheme handler and a D-Bus activation path on the device, and
//! it keeps the whole flow inside this crate — the front end only has to open
//! a URL. The port is picked from a small fixed range because every redirect
//! URI has to be declared up front during client registration.

use std::ops::Range;

use matrix_sdk::{
    authentication::oauth::{
        registration::{ApplicationType, ClientMetadata, Localized, OAuthGrantType},
        ClientRegistrationData, CsrfToken, OAuthSession, UserSession,
    },
    ruma::{serde::Raw, DeviceId, OwnedDeviceId, OwnedUserId},
    store::RoomLoadSettings,
    utils::{
        local_server::{LocalServerBuilder, LocalServerIpAddress, LocalServerRedirectHandle},
        UrlOrQuery,
    },
    Client, SessionMeta, SessionTokens,
};
use oauth2::{
    basic::BasicClient as OAuth2Client, ClientId, DeviceAuthorizationUrl, Scope,
    StandardDeviceAuthorizationResponse, TokenResponse, TokenUrl,
};
use oauth2_reqwest::ReqwestClient;
use url::Url;

/// Ports the loopback listener may bind to. All of them are declared as
/// redirect URIs, so any one of them is accepted by the authorization server.
const REDIRECT_PORTS: Range<u16> = 53182..53192;

/// Identifies the client to the authorization server; shown to the user on the
/// consent screen. It does not have to resolve.
const CLIENT_URI: &str = "https://github.com/JimKnopfIoT/harbour-xmatic";
const CLIENT_NAME: &str = "xmatic";

/// A login that has been started and is waiting for the user to finish in the
/// browser.
pub struct PendingLogin {
    /// The URL the front end has to open.
    pub url: Url,
    /// Resolves once the browser hits the loopback listener.
    pub redirect: LocalServerRedirectHandle,
    /// Identifies this authorization request; needed to clean up if the login
    /// is cancelled.
    pub state: CsrfToken,
    /// The homeserver this login was started against, needed when the session
    /// is persisted.
    pub homeserver: String,
}

/// Declares what this client is and where it may be redirected to.
fn client_metadata() -> Result<Raw<ClientMetadata>, serde_json::Error> {
    let redirect_uris = REDIRECT_PORTS
        .map(|port| {
            Url::parse(&format!("http://127.0.0.1:{port}/")).expect("loopback URI is well-formed")
        })
        .collect();

    let client_uri = Url::parse(CLIENT_URI).expect("client URI is well-formed");

    let mut metadata = ClientMetadata::new(
        ApplicationType::Native,
        vec![
            OAuthGrantType::AuthorizationCode { redirect_uris },
            OAuthGrantType::DeviceCode,
        ],
        Localized::new(client_uri, None),
    );
    metadata.client_name = Some(Localized::new(CLIENT_NAME.to_owned(), None));

    Raw::new(&metadata)
}

/// Full access to the client-server API (MSC2967).
const SCOPE_API: &str = "urn:matrix:org.matrix.msc2967.client:api:*";
/// Binds the device ID this login creates to the issued tokens (MSC2967).
const SCOPE_DEVICE_PREFIX: &str = "urn:matrix:org.matrix.msc2967.client:device:";

/// A device-code login (RFC 8628) waiting for the user to approve it
/// somewhere else.
///
/// This flow exists because the authorization-code flow needs a browser on
/// the device, and the browser of older Sailfish releases cannot render the
/// authentication service's pages. Here the app only displays a short URL and
/// a code; the sign-in happens on any other device while we poll the token
/// endpoint. The SDK implements this grant only inside its QR-code login, so
/// the flow is assembled here from the same public pieces it uses.
pub struct PendingDeviceLogin {
    /// Where the user has to go, shown by the front end.
    pub verification_uri: String,
    /// Same, with the code already embedded — for a QR code or a short link.
    pub verification_uri_complete: Option<String>,
    /// The code the user enters at `verification_uri`.
    pub user_code: String,
    /// The response the token polling needs.
    response: StandardDeviceAuthorizationResponse,
    /// The registered client this flow runs as.
    client_id: ClientId,
    /// The device ID baked into the requested scope; the session is built
    /// around it once the tokens arrive.
    device_id: OwnedDeviceId,
}

/// Registers the client and requests a device code from the server.
pub async fn start_device(client: &Client) -> Result<PendingDeviceLogin, String> {
    let oauth = client.oauth();

    let server_metadata = oauth
        .server_metadata()
        .await
        .map_err(|error| format!("server metadata unavailable: {error}"))?;

    // The registration goes over plain HTTP on purpose: the SDK's
    // `register_client` stores the resulting auth data on the client, and
    // `restore_session` — which activates the session at the end of this
    // flow — panics if anything set that data before it. Registration is a
    // bare unauthenticated JSON POST, so doing it by hand keeps the client
    // untouched.
    let registration_endpoint = server_metadata
        .registration_endpoint
        .clone()
        .ok_or_else(|| "this server does not support client registration".to_owned())?;

    let metadata = client_metadata().map_err(|error| format!("client metadata: {error}"))?;

    #[derive(serde::Deserialize)]
    struct Registered {
        client_id: String,
    }

    let registered: Registered = reqwest::Client::new()
        .post(registration_endpoint)
        .json(&metadata)
        .send()
        .await
        .map_err(|error| format!("client registration failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("client registration rejected: {error}"))?
        .json()
        .await
        .map_err(|error| format!("client registration reply unreadable: {error}"))?;

    let registration_client_id = ClientId::new(registered.client_id);
    let device_url = server_metadata
        .device_authorization_endpoint
        .clone()
        .map(DeviceAuthorizationUrl::from_url)
        .ok_or_else(|| "this server does not offer the device-code sign-in".to_owned())?;

    let device_id = DeviceId::new();
    let scopes = [
        Scope::new(SCOPE_API.to_owned()),
        Scope::new(format!("{SCOPE_DEVICE_PREFIX}{device_id}")),
    ];

    let http: ReqwestClient = reqwest::Client::new().into();
    let response = OAuth2Client::new(registration_client_id.clone())
        .set_device_authorization_url(device_url)
        .exchange_device_code()
        .add_scopes(scopes)
        .request_async(&http)
        .await
        .map_err(|error| format!("device authorization rejected: {error}"))?;

    Ok(PendingDeviceLogin {
        verification_uri: response.verification_uri().to_string(),
        verification_uri_complete: response
            .verification_uri_complete()
            .map(|uri| uri.secret().clone()),
        user_code: response.user_code().secret().clone(),
        response,
        client_id: registration_client_id,
        device_id,
    })
}

/// Polls the token endpoint until the user approves (or the code expires),
/// then activates the session on `client`.
///
/// The `oauth2` crate handles the polling interval, including the server's
/// `slow_down` responses, so this simply takes as long as the user takes.
pub async fn finish_device(client: &Client, pending: PendingDeviceLogin) -> Result<(), String> {
    let oauth = client.oauth();

    let server_metadata = oauth
        .server_metadata()
        .await
        .map_err(|error| format!("server metadata unavailable: {error}"))?;

    let http: ReqwestClient = reqwest::Client::new().into();
    let token = OAuth2Client::new(pending.client_id.clone())
        .set_token_uri(TokenUrl::from_url(server_metadata.token_endpoint.clone()))
        .exchange_device_access_token(&pending.response)
        .request_async(&http, tokio::time::sleep, None)
        .await
        .map_err(|error| format!("the sign-in was not completed: {error}"))?;

    // The tokens are not attached to the client yet, so the user ID has to be
    // fetched by hand before the session can be assembled.
    let whoami_url = client
        .homeserver()
        .join("_matrix/client/v3/account/whoami")
        .map_err(|error| format!("homeserver address cannot be extended: {error}"))?;

    #[derive(serde::Deserialize)]
    struct WhoAmI {
        user_id: OwnedUserId,
        device_id: Option<OwnedDeviceId>,
    }

    let who: WhoAmI = reqwest::Client::new()
        .get(whoami_url)
        .bearer_auth(token.access_token().secret())
        .send()
        .await
        .map_err(|error| format!("whoami request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("whoami request rejected: {error}"))?
        .json()
        .await
        .map_err(|error| format!("whoami reply unreadable: {error}"))?;

    let session = OAuthSession {
        client_id: pending.client_id,
        user: UserSession {
            meta: SessionMeta {
                user_id: who.user_id,
                device_id: who.device_id.unwrap_or(pending.device_id),
            },
            tokens: SessionTokens {
                access_token: token.access_token().secret().clone(),
                refresh_token: token.refresh_token().map(|t| t.secret().clone()),
            },
        },
    };

    oauth
        .restore_session(session, RoomLoadSettings::default())
        .await
        .map_err(|error| format!("the session could not be activated: {error}"))?;

    Ok(())
}

/// Where the homeserver lets people create an account.
///
/// Registration is deliberately not reimplemented here: on servers using the
/// Matrix Authentication Service it involves terms, a captcha and e-mail
/// confirmation, all of which change without notice. The authorization
/// server's own page handles that, and the normal sign-in works afterwards.
pub async fn registration_url(client: &Client) -> Result<String, String> {
    let metadata = client
        .oauth()
        .server_metadata()
        .await
        .map_err(|error| format!("this server does not advertise a sign-up page: {error}"))?;

    let mut url = metadata.issuer.clone();
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "the server's address cannot be extended".to_owned())?;
        segments.pop_if_empty().push("register");
    }

    Ok(url.to_string())
}

/// Registers the client if needed and builds the authorization URL.
pub async fn start(client: &Client, homeserver: String) -> Result<PendingLogin, String> {
    let metadata = client_metadata().map_err(|error| format!("client metadata: {error}"))?;
    let registration = ClientRegistrationData::new(metadata);

    let (redirect_uri, redirect) = LocalServerBuilder::new()
        .ip_address(LocalServerIpAddress::Localhostv4)
        .port_range(REDIRECT_PORTS)
        .spawn()
        .await
        .map_err(|error| format!("could not open the loopback listener: {error}"))?;

    let authorization = client
        .oauth()
        .login(redirect_uri, None, Some(registration), None)
        .build()
        .await
        .map_err(|error| format!("authorization request rejected: {error}"))?;

    Ok(PendingLogin {
        url: authorization.url,
        redirect,
        state: authorization.state,
        homeserver,
    })
}

/// Waits for the browser redirect and completes the login.
///
/// Returns `Ok(false)` when the listener was shut down without a redirect,
/// which is what an aborted login looks like.
pub async fn finish(client: &Client, redirect: LocalServerRedirectHandle) -> Result<bool, String> {
    let Some(query) = redirect.await else {
        return Ok(false);
    };

    client
        .oauth()
        .finish_login(UrlOrQuery::Query(query.0))
        .await
        .map_err(|error| format!("login could not be completed: {error}"))?;

    Ok(true)
}
