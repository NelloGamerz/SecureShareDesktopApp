use std::sync::Arc;

use tokio::sync::{RwLock};

use crate::models::Session;

#[derive(Default)]
pub struct AuthState {
    pub token: Arc<RwLock<Option<String>>>,
    pub user_id: Arc<RwLock<Option<String>>>,
    pub session: Arc<RwLock<Option<Session>>>,
    pub is_authenticated: Arc<RwLock<bool>>,
}
