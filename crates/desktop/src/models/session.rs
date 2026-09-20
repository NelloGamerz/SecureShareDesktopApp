#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct Session {
    pub token: String,
    // pub user_id: Option<String>,
}

impl Session {
    pub fn new(token: String) -> Self {
        Self { token }
    }
}
