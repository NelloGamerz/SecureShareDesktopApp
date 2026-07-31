use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not authenticated")]
    NotAuthenticated,
    #[error("not connected")]
    NotConnected,
    #[error("internal error: {0}")]
    Internal(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("tauri error: {0}")]
    Tauri(String),
}

impl AppError {
    pub fn not_authenticated() -> Self {
        Self::NotAuthenticated
    }

    pub fn not_connected() -> Self {
        Self::NotConnected
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self::Network(message.into())
    }
}

impl From<tauri::Error> for AppError {
    fn from(value: tauri::Error) -> Self {
        Self::Tauri(value.to_string())
    }
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
