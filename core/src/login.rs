//! OAuth 2.0 authorization code login. The redirect goes to a loopback
//! listener (RFC 8252 §7.3), whose port is a request-time detail.

use std::{ops::Range, time::Duration};

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
    config::RequestConfig,
    Client, SessionMeta, SessionTokens,
};
use oauth2::{
    basic::BasicClient as OAuth2Client, ClientId, DeviceAuthorizationUrl, Scope,
    StandardDeviceAuthorizationResponse, TokenResponse, TokenUrl,
};
use oauth2_reqwest::ReqwestClient;
use url::Url;

/// Ports the loopback listener may bind to. Not part of the registration:
/// RFC 8252 §7.3 has the server ignore the port when matching a loopback URI.
const REDIRECT_PORTS: Range<u16> = 53182..53192;

/// The loopback redirect URI, declared without a port. A declared port is not
/// merely superfluous, some servers reject it for a native client.
const REDIRECT_URI: &str = "http://127.0.0.1/";

/// Identifies the client to the authorization server; shown to the user on the
/// consent screen. It does not have to resolve.
const CLIENT_URI: &str = "https://github.com/JimKnopfIoT/harbour-xmatic";
const CLIENT_NAME: &str = "xmatic";

/// The device-code login's own requests: bounded, named, and no redirects,
/// so a token or code never follows a 3xx to another host.
/// A 3xx is not an error to reqwest; here it is one, and says so.
fn refuse_redirect(response: reqwest::Response) -> Result<reqwest::Response, String> {
    if response.status().is_redirection() {
        return Err("the server redirected the request, which this sign-in does not follow".to_owned());
    }
    Ok(response)
}

fn login_http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(CLIENT_NAME)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

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
    let redirect_uris = vec![Url::parse(REDIRECT_URI).expect("loopback URI is well-formed")];

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

/// A device-code login (RFC 8628) for devices whose browser cannot render the
/// auth pages. The SDK wires this grant up only for QR, so it is assembled here.
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

    // Registration is a bare JSON POST by hand: the SDK's `register_client`
    // stores auth data on the client, and `restore_session` panics if any was set.
    let registration_endpoint = server_metadata
        .registration_endpoint
        .clone()
        .ok_or_else(|| "this server does not support client registration".to_owned())?;

    let metadata = client_metadata().map_err(|error| format!("client metadata: {error}"))?;

    #[derive(serde::Deserialize)]
    struct Registered {
        client_id: String,
    }

    let registered: Registered = login_http()
        .post(registration_endpoint)
        .json(&metadata)
        .send()
        .await
        .map_err(|error| format!("client registration failed: {error}"))
        .and_then(refuse_redirect)?
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

    let http: ReqwestClient = login_http().into();
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

/// Polls the token endpoint until the user approves or the code expires. The
/// `oauth2` crate handles the interval, `slow_down` included.
pub async fn finish_device(client: &Client, pending: PendingDeviceLogin) -> Result<(), String> {
    let oauth = client.oauth();

    let server_metadata = oauth
        .server_metadata()
        .await
        .map_err(|error| format!("server metadata unavailable: {error}"))?;

    let http: ReqwestClient = login_http().into();
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

    let who: WhoAmI = login_http()
        .get(whoami_url)
        .bearer_auth(token.access_token().secret())
        .send()
        .await
        .map_err(|error| format!("whoami request failed: {error}"))
        .and_then(refuse_redirect)?
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

/// Where the homeserver lets people create an account. Registration is not
/// reimplemented: terms, captcha and e-mail confirmation change without notice.
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

/// Waits for the browser redirect. `Ok(false)` is a listener shut down without
/// one, which is what an aborted login looks like.
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

/// True if the server offers `m.login.password`. Only consulted after OAuth
/// answered `NotSupported`; a transport error is an error, never a fallback.
pub async fn password_offered(client: &Client) -> Result<bool, String> {
    Ok(login_flows(client).await?.iter().any(|flow| flow == "password"))
}

/// Every sign-in method the server offers. The point is the negative case: the
/// user needs to hear what the server wanted, not just that this app cannot.
pub async fn login_flows(client: &Client) -> Result<Vec<String>, String> {
    use matrix_sdk::ruma::api::client::session::get_login_types::v3::LoginType;

    let response = client
        .matrix_auth()
        .get_login_types()
        .await
        .map_err(|error| {
            format!(
                "could not ask for the sign-in methods: {}",
                crate::text::scrub_ids(&error.to_string())
            )
        })?;

    Ok(response
        .flows
        .iter()
        .map(|flow| {
            match flow {
                LoginType::Password(_) => "password",
                LoginType::Sso(_) => "sso",
                LoginType::Token(_) => "token",
                LoginType::ApplicationService(_) => "appservice",
                _ => "other",
            }
            .to_owned()
        })
        .collect())
}

/// Signs in with `m.login.password`. The refresh token is requested (MSC2918)
/// so the stored token rotates; the password is borrowed and never copied.
pub async fn password(client: &Client, user: &str, password: &str) -> Result<(), String> {
    client
        .matrix_auth()
        .login_username(user, password)
        .initial_device_display_name(CLIENT_NAME)
        .request_refresh_token()
        .send()
        .await
        .map_err(|error| format!("sign-in failed: {error}"))?;

    Ok(())
}

/// The interactive-auth session the server opened for a cross-signing setup,
/// kept until the user answers it with the password. Not a secret.
static CROSS_SIGNING_UIA: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// How long the password may wait on the setup request.
const CROSS_SIGNING_LIMIT: Duration = Duration::from_secs(60);

/// One setup at a time: two answers to the same check must not upload twice.
static CROSS_SIGNING_SETUP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn forget_cross_signing() {
    if let Ok(mut guard) = CROSS_SIGNING_UIA.lock() {
        *guard = None;
    }
}

/// Whether the server holds a master key for this account - asked, not read
/// from the store, which keeps the key a refused setup created locally.
async fn server_has_identity(client: &Client) -> Result<bool, String> {
    use matrix_sdk::ruma::api::client::keys::get_keys;

    let user = client.user_id().ok_or("not signed in")?.to_owned();
    let mut request = get_keys::v3::Request::new();
    request.device_keys.insert(user.clone(), Vec::new());
    // Bounded: the password waits on this answer.
    let config = RequestConfig::new().disable_retry().timeout(Duration::from_secs(20));
    let response = client
        .send(request)
        .with_request_config(config)
        .await
        .map_err(|error| crate::text::scrub_ids(&format!("could not ask for the keys: {error}")))?;
    Ok(response.master_keys.contains_key(&user))
}

/// This device signed by the account's identity, where this device holds the
/// signing key: a setup cut off between the two uploads leaves it unsigned.
async fn sign_own_device(client: &Client) {
    let encryption = client.encryption();
    let has_key = encryption
        .cross_signing_status()
        .await
        .is_some_and(|status| status.has_self_signing);
    if !has_key {
        return;
    }
    if let Ok(Some(device)) = encryption.get_own_device().await {
        if !device.is_cross_signed_by_owner() {
            if let Err(error) = device.verify().await {
                log::warn!("{}", crate::text::scrub_ids(&format!("signing this device failed: {error}")));
            }
        }
    }
}

/// `true` where the server wants the password to create the account's identity.
/// Decided by the server each time, never replacing an identity it has.
pub async fn cross_signing_without_password(client: &Client) -> Result<bool, String> {
    let _setup = CROSS_SIGNING_SETUP.lock().await;
    if server_has_identity(client).await? {
        sign_own_device(client).await;
        return Ok(false);
    }
    let Err(error) = client.encryption().bootstrap_cross_signing(None).await else {
        return Ok(false);
    };
    let Some(response) = error.as_uiaa_response() else {
        return Err(crate::text::scrub_ids(&format!("cross-signing setup failed: {error}")));
    };
    if let Ok(mut guard) = CROSS_SIGNING_UIA.lock() {
        *guard = response.session.clone();
    }
    Ok(true)
}

/// Answers the server's check with the password the user just typed; one
/// command, one request. Asks the server again first: an identity another
/// client created meanwhile is kept, never overwritten.
pub async fn cross_signing_with_password(
    client: &Client,
    password: crate::protocol::Secret,
) -> Result<bool, String> {
    use matrix_sdk::ruma::api::client::uiaa;

    // Never queued behind another setup with the password in hand.
    let Ok(_setup) = CROSS_SIGNING_SETUP.try_lock() else {
        return Err("a setup is already running".to_owned());
    };
    if server_has_identity(client).await? {
        drop(password);
        forget_cross_signing();
        // A setup cut off earlier: its keys arrived, its signature did not.
        sign_own_device(client).await;
        return Ok(false);
    }
    let user = client.user_id().ok_or("not signed in")?.to_string();
    let session = CROSS_SIGNING_UIA.lock().ok().and_then(|guard| guard.clone());
    let mut auth = uiaa::Password::new(
        uiaa::UserIdentifier::Matrix(uiaa::MatrixUserIdentifier::new(user)),
        password.as_str().to_owned(),
    );
    auth.session = session;
    // The SDK's retry has no ceiling; a setup cut here is signed by the next start.
    let outcome = tokio::time::timeout(
        CROSS_SIGNING_LIMIT,
        client.encryption().bootstrap_cross_signing(Some(uiaa::AuthData::Password(auth))),
    )
    .await;
    drop(password);
    let Ok(outcome) = outcome else {
        return Err("the server did not answer in time".to_owned());
    };
    match outcome {
        Ok(()) => {
            forget_cross_signing();
            sign_own_device(client).await;
            Ok(true)
        }
        Err(error) => {
            if let Some(response) = error.as_uiaa_response() {
                if let Ok(mut guard) = CROSS_SIGNING_UIA.lock() {
                    *guard = response.session.clone();
                }
                return Err("the password was not accepted".to_owned());
            }
            Err(crate::text::scrub_ids(&format!("cross-signing setup failed: {error}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A declared port is what a strict server rejects for a native client;
    /// the listener's actual port is matched by the server ignoring it.
    #[test]
    fn redirect_uri_carries_no_port() {
        let raw = client_metadata().expect("metadata serialises");
        let json: serde_json::Value =
            serde_json::from_str(raw.json().get()).expect("metadata is JSON");

        let uris = json["redirect_uris"].as_array().expect("redirect_uris is a list");
        assert_eq!(uris.len(), 1);

        let uri = Url::parse(uris[0].as_str().expect("redirect URI is a string"))
            .expect("redirect URI parses");
        assert_eq!(uri.port(), None);
        assert_eq!(uri.host_str(), Some("127.0.0.1"));
        assert_eq!(uri.path(), "/");
    }

    /// The listener binds inside the declared-free port range; what the server
    /// sees has to reduce to the registered URI.
    #[test]
    fn listener_uri_reduces_to_the_registered_one() {
        for port in REDIRECT_PORTS {
            let mut uri = Url::parse(&format!("http://127.0.0.1:{port}/")).expect("listener URI");
            uri.set_port(None).expect("port can be dropped");
            assert_eq!(uri.as_str(), REDIRECT_URI);
        }
    }
}
