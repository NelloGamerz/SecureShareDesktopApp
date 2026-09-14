use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::models::Session;

/// An OAuth Authorization Code + PKCE flow that has been started but not yet
/// completed.
///
/// This lives in memory only. If the app exits mid-flow the verifier is lost
/// and the user simply starts again, which is preferable to persisting a
/// credential-bearing secret to disk.
#[derive(Debug, Clone)]
pub struct PendingLogin {
    /// Opaque `state` value echoed back by Clerk; compared on callback.
    pub state: String,
    /// PKCE `code_verifier`. Never logged.
    pub code_verifier: String,
    /// Token endpoint the code will be exchanged at.
    pub token_endpoint: String,
    /// Client id of the public OAuth application.
    pub client_id: String,
    /// Must match the value sent on the authorization request.
    pub redirect_uri: String,
}

#[derive(Default)]
pub struct AuthState {
    pub token: Arc<RwLock<Option<String>>>,
    pub user_id: Arc<RwLock<Option<String>>>,
    pub session: Arc<RwLock<Option<Session>>>,
    pub is_authenticated: Arc<RwLock<bool>>,

    /*
     * Desktop OAuth lifecycle.
     */
    /// Refresh token issued alongside the access token. Memory only.
    pub refresh_token: Arc<RwLock<Option<String>>>,
    /// Unix epoch seconds at which `token` expires, when the issuer reports it.
    pub token_expires_at: Arc<RwLock<Option<i64>>>,
    /// The single in-flight authorization request, if any.
    pub pending_login: Arc<RwLock<Option<PendingLogin>>>,
    /// Serializes refresh attempts so concurrent requests cannot each spend the
    /// same refresh token.
    pub refresh_lock: Arc<Mutex<()>>,
    /// Token endpoint of the active OAuth application, retained so a refresh
    /// can be performed without the frontend re-supplying configuration.
    pub token_endpoint: Arc<RwLock<Option<String>>>,
    /// Client id of the active OAuth application.
    pub client_id: Arc<RwLock<Option<String>>>,
}

impl AuthState {
    /// Clears every credential-bearing field. Used on logout and whenever a
    /// session is replaced.
    pub async fn clear_credentials(&self) {
        *self.token.write().await = None;
        *self.refresh_token.write().await = None;
        *self.token_expires_at.write().await = None;
        *self.user_id.write().await = None;
        *self.session.write().await = None;
        *self.is_authenticated.write().await = false;
    }
}
