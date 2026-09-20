//! Native desktop OAuth (Authorization Code + PKCE) against Clerk.
//!
//! The desktop app is a **public client**: it holds no client secret. Instead it
//! uses PKCE (RFC 7636) so that an authorization code intercepted on the deep
//! link cannot be exchanged without the in-process `code_verifier`.
//!
//! Flow:
//!   1. [`OAuthService::begin`] mints a `state` + `code_verifier`, stores them in
//!      `AuthState::pending_login`, and returns the authorization URL.
//!   2. The command layer opens that URL in the system browser.
//!   3. Clerk redirects to the registered custom-scheme redirect URI.
//!   4. The deep-link listener hands the URL to [`OAuthService::complete`], which
//!      validates `state`, exchanges the code, and stores the tokens.
//!
//! Nothing in this module logs an authorization code, access token, refresh
//! token, id token, or `code_verifier`.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Url};

use crate::error::AppError;
use crate::models::Session;
use crate::services::keyring_service::{KeyringService, StoredAuthSession};
use crate::state::auth_state::{AuthState, PendingLogin};
use std::sync::Arc;

/// Refresh this many seconds before the reported expiry, so a request in flight
/// cannot race the expiry boundary.
const EXPIRY_SKEW_SECS: i64 = 60;

/// PKCE `code_verifier` entropy (RFC 7636 allows 43–128 chars; 32 bytes of
/// base64url is 43 chars).
const VERIFIER_BYTES: usize = 32;
/// `state` entropy.
const STATE_BYTES: usize = 32;

const DEFAULT_SCOPES: &str = "openid profile email";

/// Configuration supplied by the frontend on each `start_desktop_auth` call so
/// that the app has a single source of truth for environment variables.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopAuthConfig {
    pub client_id: String,
    /// Clerk Frontend API origin, e.g. `https://clerk.example.com`.
    pub issuer: String,
    /// Must exactly match a redirect URI registered on the Clerk OAuth
    /// application, e.g. `vilsend://auth/callback`.
    pub redirect_uri: String,
    #[serde(default)]
    pub scopes: Option<String>,
    #[serde(default)]
    pub authorize_endpoint: Option<String>,
    #[serde(default)]
    pub token_endpoint: Option<String>,
    /// Optional extra OIDC `prompt` value sent only for the sign-up action, for
    /// Clerk instances that support steering the authorize page to registration.
    #[serde(default)]
    pub signup_prompt: Option<String>,
}

impl DesktopAuthConfig {
    fn authorize_endpoint(&self) -> String {
        self.authorize_endpoint
            .clone()
            .unwrap_or_else(|| self.endpoint("oauth/authorize"))
    }

    /// Resolved token endpoint, retained by the command layer so a later
    /// refresh does not need the frontend to resend configuration.
    pub fn token_endpoint(&self) -> String {
        self.token_endpoint
            .clone()
            .unwrap_or_else(|| self.endpoint("oauth/token"))
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.issuer.trim_end_matches('/'), path)
    }

    fn scopes(&self) -> String {
        self.scopes
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_SCOPES.to_string())
    }

    /// Rejects configuration that cannot produce a safe flow before anything is
    /// opened in a browser.
    fn validate(&self) -> Result<(), AppError> {
        if self.client_id.trim().is_empty() {
            return Err(AppError::auth("Clerk OAuth client ID is not configured"));
        }

        let issuer = Url::parse(&self.issuer)
            .map_err(|_| AppError::auth("Clerk issuer is not a valid URL"))?;

        if issuer.scheme() != "https" && !is_loopback_host(&issuer) {
            return Err(AppError::auth("Clerk issuer must use https"));
        }

        if Url::parse(&self.redirect_uri).is_err() {
            return Err(AppError::auth("OAuth redirect URI is not a valid URL"));
        }

        Ok(())
    }
}

/// Result of a completed exchange. Deliberately contains no token material —
/// callers hand this to the webview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthOutcome {
    pub user_id: Option<String>,
    pub expires_at: Option<i64>,
}

/// Tokens as returned by the Clerk OAuth token endpoint.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    token_type: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

/// Error body returned by the token endpoint.
#[derive(Debug, Deserialize)]
struct TokenErrorResponse {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Why a token request failed.
///
/// The distinction is load-bearing on the refresh path: a rejection means the
/// stored credentials are dead and the session must be cleared, whereas a
/// transport or server-side failure is transient and must leave it intact.
/// Collapsing the two would sign a user out for opening the app offline.
#[derive(Debug)]
enum TokenRequestError {
    /// The endpoint could not be reached at all — DNS, TLS, timeout.
    Transport(String),
    /// The endpoint answered with a definitive rejection (a 4xx), such as
    /// `invalid_grant` for an expired or revoked refresh token.
    Rejected {
        status: u16,
        detail: String,
        origin: String,
        client_id: String,
    },
    /// The endpoint answered with a server-side error (a 5xx).
    Server { status: u16 },
    /// The endpoint answered 2xx with a body this client cannot read.
    Malformed,
}

impl TokenRequestError {
    /// True only when the issuer has spoken about the credentials themselves.
    fn is_definitive(&self) -> bool {
        matches!(self, Self::Rejected { .. } | Self::Malformed)
    }

    /// The error to surface to someone actively signing in, where the request is
    /// the whole point of the interaction and every failure should be explained.
    fn into_app_error(self) -> AppError {
        match self {
            Self::Transport(error) => AppError::network(format!(
                "Could not reach Clerk to finish signing in: {error}"
            )),
            Self::Rejected {
                status,
                detail,
                origin,
                client_id,
            } => {
                // A public client sends no client secret, so Clerk answers 401
                // when the OAuth application still requires one. Lead with the
                // fix; the raw description is long and unhelpful on its own.
                AppError::auth(match status {
                    401 => format!(
                        "Clerk rejected the app's client credentials. Enable \"Public\" on \
                         this OAuth application in the Clerk Dashboard so that it does not \
                         require a client secret. (HTTP 401: {detail}. Issuer: {origin}, \
                         client ID: {client_id}.)"
                    ),
                    _ => format!(
                        "Sign-in failed at the token endpoint (HTTP {status}): {detail}. \
                         Issuer: {origin}, client ID: {client_id}."
                    ),
                })
            }
            Self::Server { status } => AppError::auth(format!(
                "Clerk could not complete the request (HTTP {status}). Please try again."
            )),
            Self::Malformed => {
                AppError::internal("Token endpoint returned an unexpected response shape")
            }
        }
    }
}

/// For logging only. Carries no token material, so it is safe in a log line.
impl std::fmt::Display for TokenRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(error) => {
                write!(formatter, "could not reach the token endpoint: {error}")
            }
            Self::Rejected {
                status,
                detail,
                origin,
                client_id,
            } => write!(
                formatter,
                "rejected with HTTP {status} ({detail}); issuer {origin}, client ID {client_id}"
            ),
            Self::Server { status } => {
                write!(formatter, "token endpoint returned HTTP {status}")
            }
            Self::Malformed => write!(formatter, "token endpoint returned an unexpected shape"),
        }
    }
}

pub struct OAuthService {
    auth_state: Arc<AuthState>,
    /// Needed to reach the OS keychain, where the session is persisted so a
    /// restart does not sign the user out.
    app: AppHandle,
}

impl OAuthService {
    pub fn new(auth_state: Arc<AuthState>, app: AppHandle) -> Self {
        Self { auth_state, app }
    }

    /// Resumes a session persisted by a previous run.
    ///
    /// Returns `true` when a session was restored. Call this once during startup
    /// **before** the webview can read the status, so an already-signed-in user
    /// lands in the app instead of on the sign-in screen.
    ///
    /// Deliberately performs no network I/O: a read of the local keychain cannot
    /// block startup on a slow or absent connection. An access token that has
    /// aged out is refreshed lazily by [`Self::token_for_request`] on the first
    /// authenticated request, which keeps the app usable offline.
    ///
    /// A session that has expired *and* carries no refresh token cannot be
    /// renewed and is dropped instead of restored — see [`Self::is_usable`].
    pub async fn restore(&self) -> bool {
        let stored = match KeyringService::load_auth_session(&self.app) {
            Ok(Some(stored)) => stored,
            Ok(None) => return false,
            Err(error) => {
                // A keychain that cannot be read is not fatal: the user simply
                // signs in again.
                tracing::warn!(
                    target: "oauth_service",
                    event = "session_restore_failed",
                    error = %error,
                    "could not read the persisted desktop session"
                );
                return false;
            }
        };

        if !Self::is_usable(&stored) {
            // Drop the unusable credentials rather than leaving them to be
            // re-read and re-evaluated on every future launch.
            tracing::info!(
                target: "oauth_service",
                event = "session_restore_expired",
                "persisted session had expired and could not be renewed; clearing it"
            );

            self.clear().await;
            return false;
        }

        self.adopt(stored).await;

        tracing::info!(
            target: "oauth_service",
            event = "session_restored",
            "resumed the desktop session from secure storage"
        );

        true
    }

    /// Whether a restored session can still be used or renewed.
    ///
    /// A session whose access token has already expired, and which holds no
    /// refresh token, is dead on arrival: restoring it would present a signed-in
    /// UI whose every request fails with a 401. It is better to treat it as
    /// signed out so the user is asked to sign in once, up front.
    fn is_usable(stored: &StoredAuthSession) -> bool {
        if stored.refresh_token.is_some() {
            return true;
        }

        match stored.expires_at {
            Some(expiry) => now_unix() + EXPIRY_SKEW_SECS < expiry,
            // No expiry recorded. Assume usable and let the first request decide.
            None => true,
        }
    }

    /// Mints a fresh PKCE challenge and records the pending flow.
    ///
    /// Returns the URL that should be opened in the system browser.
    pub async fn begin(&self, config: &DesktopAuthConfig, mode: &str) -> Result<String, AppError> {
        config.validate()?;

        let mut pending = self.auth_state.pending_login.write().await;

        // Guard against a double-click or a second window starting a competing
        // flow, which would make the callback `state` ambiguous.
        if pending.is_some() {
            return Err(AppError::auth(
                "A sign-in is already in progress. Finish it or cancel it first.",
            ));
        }

        let state = random_urlsafe(STATE_BYTES);
        let code_verifier = random_urlsafe(VERIFIER_BYTES);
        let code_challenge = code_challenge_for(&code_verifier);

        let mut url = Url::parse(&config.authorize_endpoint())
            .map_err(|_| AppError::internal("invalid Clerk authorization endpoint"))?;

        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &config.client_id)
                .append_pair("redirect_uri", &config.redirect_uri)
                .append_pair("scope", &config.scopes())
                .append_pair("state", &state)
                .append_pair("code_challenge", &code_challenge)
                .append_pair("code_challenge_method", "S256");

            if mode == "sign_up" {
                if let Some(prompt) = config
                    .signup_prompt
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    query.append_pair("prompt", prompt);
                }
            }
        }

        *pending = Some(PendingLogin {
            state,
            code_verifier,
            token_endpoint: config.token_endpoint(),
            client_id: config.client_id.clone(),
            redirect_uri: config.redirect_uri.clone(),
        });

        Ok(url.to_string())
    }

    /// Abandons any in-flight flow. Safe to call when nothing is pending.
    pub async fn cancel(&self) -> Result<(), AppError> {
        *self.auth_state.pending_login.write().await = None;
        Ok(())
    }

    /// Validates and consumes a deep-link callback, then exchanges the
    /// authorization code for tokens.
    ///
    /// Returns `Ok(None)` when the URL is not an authorization callback at all
    /// (so the caller can ignore unrelated deep links rather than erroring).
    pub async fn complete(&self, callback: &Url) -> Result<Option<AuthOutcome>, AppError> {
        // Take the pending flow up-front: consuming it before any I/O makes a
        // replayed callback fail rather than spend the code twice.
        let pending = self.auth_state.pending_login.write().await.take();

        let Some(pending) = pending else {
            // No flow to complete. If a session already exists this is a late
            // duplicate of a callback that already succeeded (the OS or the
            // browser can deliver one twice); ignoring it avoids replacing a
            // valid session with a spurious error.
            if *self.auth_state.is_authenticated.read().await {
                return Ok(None);
            }

            // Otherwise the app was very likely restarted while the user was in
            // the browser, which discards the in-memory PKCE verifier.
            return Err(AppError::auth(
                "No sign-in is in progress. Start sign-in again from the app.",
            ));
        };

        if !callback_matches_redirect(callback, &pending.redirect_uri) {
            return Err(AppError::auth("Unexpected OAuth callback URL"));
        }

        let params: std::collections::HashMap<_, _> = callback.query_pairs().collect();

        // Clerk reports user-facing failures (denied consent, cancelled, etc.)
        // on the callback rather than by failing the exchange.
        if let Some(error) = params.get("error") {
            let description = params
                .get("error_description")
                .map(|value| value.to_string())
                .unwrap_or_else(|| error.to_string());

            return Err(AppError::auth(format!(
                "Sign-in was not completed: {description}"
            )));
        }

        let Some(returned_state) = params.get("state") else {
            return Err(AppError::auth("OAuth callback is missing the state"));
        };

        if !constant_time_eq(returned_state, &pending.state) {
            return Err(AppError::auth(
                "OAuth callback state did not match this sign-in attempt",
            ));
        }

        let Some(code) = params.get("code") else {
            return Err(AppError::auth(
                "OAuth callback is missing the authorization code",
            ));
        };

        let tokens = self.exchange_code(&pending, code).await?;

        // Clerk issues `Bearer` tokens; anything else would produce an
        // `Authorization` header the backend cannot understand.
        if let Some(token_type) = tokens.token_type.as_deref() {
            if !token_type.eq_ignore_ascii_case("bearer") {
                return Err(AppError::auth(
                    "Authorization server returned an unsupported token type",
                ));
            }
        }

        let expires_at = tokens.expires_in.map(|seconds| now_unix() + seconds);

        // Prefer the id_token for identity; fall back to the access token when
        // it is a JWT. Both are treated as display data only — authorization is
        // always the backend's decision.
        let user_id = tokens
            .id_token
            .as_deref()
            .and_then(subject_of)
            .or_else(|| subject_of(&tokens.access_token));

        self.store_tokens(
            tokens.access_token,
            tokens.refresh_token,
            expires_at,
            user_id.clone(),
        )
        .await;

        Ok(Some(AuthOutcome {
            user_id,
            expires_at,
        }))
    }

    /// Returns a token suitable for an `Authorization` header, refreshing it
    /// first when it is at or past its expiry skew.
    ///
    /// On a failed refresh the local session is cleared so the UI can return to
    /// the unauthenticated screen instead of looping on 401s.
    pub async fn token_for_request(&self) -> Result<Option<String>, AppError> {
        let (token, expires_at) = {
            let token = self.auth_state.token.read().await.clone();
            let expires_at = *self.auth_state.token_expires_at.read().await;
            (token, expires_at)
        };

        let Some(current) = token else {
            return Ok(None);
        };

        let needs_refresh = expires_at
            .map(|expiry| now_unix() + EXPIRY_SKEW_SECS >= expiry)
            .unwrap_or(false);

        if !needs_refresh {
            return Ok(Some(current));
        }

        let refresh_token = self.auth_state.refresh_token.read().await.clone();

        let Some(refresh_token) = refresh_token else {
            // Without a refresh token the stored session can no longer be
            // renewed, so it is genuinely over. An issuer only grants one when
            // `offline_access` was requested.
            tracing::warn!(
                target: "oauth_service",
                event = "token_expired_without_refresh",
                "access token expired and no refresh token was issued"
            );
            self.clear().await;
            return Err(AppError::NotAuthenticated);
        };

        // Serialize refreshes: a second caller waits, then re-reads the token
        // the first caller already installed.
        let _guard = self.auth_state.refresh_lock.lock().await;

        {
            let token = self.auth_state.token.read().await.clone();
            let expires_at = *self.auth_state.token_expires_at.read().await;

            if token.is_some()
                && expires_at
                    .map(|expiry| now_unix() + EXPIRY_SKEW_SECS < expiry)
                    .unwrap_or(false)
            {
                return Ok(token);
            }
        }

        match self.refresh(&refresh_token).await {
            Ok(tokens) => {
                let expires_at = tokens.expires_in.map(|seconds| now_unix() + seconds);
                let user_id = self.auth_state.user_id.read().await.clone();

                self.store_tokens(
                    tokens.access_token,
                    tokens.refresh_token,
                    expires_at,
                    user_id,
                )
                .await;

                tracing::info!(
                    target: "oauth_service",
                    event = "token_refreshed",
                    "desktop session token refreshed"
                );

                Ok(self.auth_state.token.read().await.clone())
            }
            // The issuer rejected the refresh token: it has expired, been
            // revoked, or been spent. The session is over, so clear it and send
            // the user back to the sign-in screen.
            Err(error) if error.is_definitive() => {
                tracing::warn!(
                    target: "oauth_service",
                    event = "token_refresh_rejected",
                    error = %error,
                    "desktop session refresh was rejected; clearing session"
                );

                self.clear().await;
                Err(AppError::NotAuthenticated)
            }
            // A transport or server-side failure says nothing about whether the
            // credentials are still good. Keep the session: signing the user out
            // because the machine was briefly offline would defeat persisted
            // sign-in, and the next request retries the refresh.
            Err(error) => {
                tracing::warn!(
                    target: "oauth_service",
                    event = "token_refresh_deferred",
                    error = %error,
                    "desktop session refresh could not be completed; keeping the session"
                );

                Ok(self.auth_state.token.read().await.clone())
            }
        }
    }

    /// Reads the current authentication state without touching the network.
    pub async fn snapshot(&self) -> (bool, Option<String>, bool) {
        let is_authenticated = *self.auth_state.is_authenticated.read().await;
        let user_id = self.auth_state.user_id.read().await.clone();
        let has_pending_login = self.auth_state.pending_login.read().await.is_some();

        (is_authenticated, user_id, has_pending_login)
    }

    async fn exchange_code(
        &self,
        pending: &PendingLogin,
        code: &str,
    ) -> Result<TokenResponse, AppError> {
        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", pending.redirect_uri.as_str()),
            ("client_id", pending.client_id.as_str()),
            ("code_verifier", pending.code_verifier.as_str()),
        ];

        self.post_token(
            pending.token_endpoint.as_str(),
            pending.client_id.as_str(),
            &params,
        )
        .await
        .map_err(TokenRequestError::into_app_error)
    }

    async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse, TokenRequestError> {
        let (token_endpoint, client_id) = {
            // The endpoints are no longer in `pending_login` once the flow has
            // completed, so recover them from the active session's config by
            // re-reading what the frontend last supplied.
            let endpoint = self.auth_state.token_endpoint.read().await.clone();

            let client_id = self.auth_state.client_id.read().await.clone();

            (endpoint, client_id)
        };

        let Some(token_endpoint) = token_endpoint else {
            return Err(TokenRequestError::Transport(
                "token endpoint is not configured".to_string(),
            ));
        };

        let Some(client_id) = client_id else {
            return Err(TokenRequestError::Transport(
                "client id is not configured".to_string(),
            ));
        };

        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id.as_str()),
        ];

        self.post_token(token_endpoint.as_str(), client_id.as_str(), &params)
            .await
    }

    async fn post_token(
        &self,
        endpoint: &str,
        client_id: &str,
        params: &[(&str, &str)],
    ) -> Result<TokenResponse, TokenRequestError> {
        let response = reqwest::Client::new()
            .post(endpoint)
            .header("Accept", "application/json")
            .form(params)
            .send()
            .await
            .map_err(|error| {
                // reqwest errors can embed the request URL; they never contain
                // the code or verifier (those travel in the body).
                TokenRequestError::Transport(error.to_string())
            })?;

        let status = response.status();

        if !status.is_success() {
            // Clerk returns `{ error, error_description }`. Surface the
            // description only — never the body wholesale, and never the code.
            let detail = response
                .json::<TokenErrorResponse>()
                .await
                .ok()
                .and_then(|body| body.error_description.or(body.error))
                .unwrap_or_else(|| "the authorization server rejected the request".to_string());

            let origin = origin_of(endpoint);

            // Both the issuer and the client ID are public identifiers, so
            // naming them here is safe and makes a misconfiguration obvious:
            // they reveal immediately whether this client ID belongs to the
            // same Clerk instance as the configured publishable key.
            tracing::warn!(
                target: "oauth_service",
                event = "token_request_rejected",
                status = status.as_u16(),
                issuer = %origin,
                client_id = %client_id,
                "the token endpoint rejected the desktop client's request"
            );

            // A 5xx is the issuer's problem, not the credentials': report it as
            // transient so a refresh does not clear a valid session.
            return Err(if status.is_server_error() {
                TokenRequestError::Server {
                    status: status.as_u16(),
                }
            } else {
                TokenRequestError::Rejected {
                    status: status.as_u16(),
                    detail,
                    origin,
                    client_id: client_id.to_string(),
                }
            });
        }

        response
            .json::<TokenResponse>()
            .await
            .map_err(|_| TokenRequestError::Malformed)
    }

    async fn store_tokens(
        &self,
        access_token: String,
        refresh_token: Option<String>,
        expires_at: Option<i64>,
        user_id: Option<String>,
    ) {
        let session = Session::new(access_token.clone());

        *self.auth_state.token.write().await = Some(access_token.clone());
        *self.auth_state.session.write().await = Some(session);
        *self.auth_state.token_expires_at.write().await = expires_at;
        *self.auth_state.is_authenticated.write().await = true;
        *self.auth_state.user_id.write().await = user_id;

        // A refresh response may omit the refresh token; keep the existing one
        // in that case so the session can still be renewed.
        if let Some(refresh_token) = refresh_token {
            *self.auth_state.refresh_token.write().await = Some(refresh_token);
        }

        // Persist after the in-memory state is settled, so the keychain copy
        // always matches what this process is actually using.
        self.persist_session(&access_token).await;
    }

    /// Installs a session loaded from secure storage, retaining the endpoints a
    /// later refresh needs.
    async fn adopt(&self, stored: StoredAuthSession) {
        // The endpoints are written before `store_tokens` so that its persist
        // step captures them; otherwise a session restored and killed before its
        // first refresh would lose the ability to refresh at all.
        {
            *self.auth_state.token_endpoint.write().await = stored.token_endpoint.clone();
            *self.auth_state.client_id.write().await = stored.client_id.clone();
        }

        self.store_tokens(
            stored.access_token,
            stored.refresh_token,
            stored.expires_at,
            stored.user_id,
        )
        .await;
    }

    /// Mirrors the current in-memory session into the OS keychain.
    ///
    /// Best-effort by design: a keychain that refuses the write must not fail a
    /// sign-in that has already succeeded. The user stays signed in for this
    /// run and is asked to sign in again only after a restart.
    async fn persist_session(&self, access_token: &str) {
        let stored = StoredAuthSession {
            access_token: access_token.to_string(),
            refresh_token: self.auth_state.refresh_token.read().await.clone(),
            expires_at: *self.auth_state.token_expires_at.read().await,
            user_id: self.auth_state.user_id.read().await.clone(),
            token_endpoint: self.auth_state.token_endpoint.read().await.clone(),
            client_id: self.auth_state.client_id.read().await.clone(),
        };

        if let Err(error) = KeyringService::save_auth_session(&self.app, &stored) {
            tracing::warn!(
                target: "oauth_service",
                event = "session_persist_failed",
                error = %error,
                "could not persist the desktop session; the user will have to sign in again after a restart"
            );
        }
    }

    /// Records the endpoints needed by a later refresh. Stored in memory only.
    pub async fn remember_endpoints(&self, token_endpoint: String, client_id: String) {
        *self.auth_state.token_endpoint.write().await = Some(token_endpoint);
        *self.auth_state.client_id.write().await = Some(client_id);
    }

    /// Ends the session, dropping both the in-memory credentials and the
    /// persisted copy. A logout that left the keychain entry behind would sign
    /// the user straight back in on the next launch.
    pub async fn clear(&self) {
        *self.auth_state.pending_login.write().await = None;
        *self.auth_state.token_endpoint.write().await = None;
        *self.auth_state.client_id.write().await = None;
        self.auth_state.clear_credentials().await;

        if let Err(error) = KeyringService::delete_auth_session(&self.app) {
            tracing::warn!(
                target: "oauth_service",
                event = "session_delete_failed",
                error = %error,
                "could not remove the persisted desktop session"
            );
        }
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs() as i64
}

fn random_urlsafe(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buffer);
    URL_SAFE_NO_PAD.encode(buffer)
}

fn code_challenge_for(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Compares two `state` values without an early exit, so an attacker cannot
/// learn a matching prefix from response timing.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());

    if a.len() != b.len() {
        return false;
    }

    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        difference |= x ^ y;
    }

    difference == 0
}

/// Confirms the callback landed on the redirect URI we actually requested.
fn callback_matches_redirect(callback: &Url, redirect_uri: &str) -> bool {
    let Ok(expected) = Url::parse(redirect_uri) else {
        return false;
    };

    let normalize = |value: &str| value.trim_end_matches('/').to_ascii_lowercase();

    callback.scheme().eq_ignore_ascii_case(expected.scheme())
        && callback
            .host_str()
            .map(normalize)
            .eq(&expected.host_str().map(normalize))
        && normalize(callback.path()) == normalize(expected.path())
}

/// Reads the `sub` claim out of a JWT payload **without verifying the
/// signature**. The value is used only to label local session state; every
/// authorization decision remains the backend's, based on the token itself.
/// Returns `None` for opaque tokens.
fn subject_of(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    claims
        .get("sub")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
}

/// Scheme + host of a URL, for diagnostics. The token endpoint carries no
/// secrets, so quoting its origin is safe.
fn origin_of(endpoint: &str) -> String {
    match Url::parse(endpoint) {
        Ok(url) => match url.host_str() {
            Some(host) => format!("{}://{}", url.scheme(), host),
            None => url.scheme().to_string(),
        },
        Err(_) => endpoint.to_string(),
    }
}

fn is_loopback_host(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("[::1]")
    )
}
