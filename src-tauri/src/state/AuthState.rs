pub struct AuthState {
    pub token: RwLock<Option<String>>,
    pub user_id: RwLock<Option<String>>,
}