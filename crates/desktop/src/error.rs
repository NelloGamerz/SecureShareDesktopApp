use thiserror::Error;
use vilsend_core::VilsendError;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not authenticated")]
    NotAuthenticated,
    /// A user-facing authentication failure.
    ///
    /// Rendered verbatim in the UI, so the message must already be safe to show
    /// and must never contain tokens, authorization codes, or client secrets.
    #[error("{0}")]
    Auth(String),
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

    pub fn auth(message: impl Into<String>) -> Self {
        Self::Auth(message.into())
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

/*
 * The bridge to `vilsend_core::VilsendError` (ADR-0003).
 *
 * `AppError` keeps its variant set and its flat-string serialisation: it is
 * what the webview parses today, and Phase 2 does not change a command or
 * event payload. What it gains is a way in and out of the boundary type, so
 * that Phase 3 can move `?` sites across one at a time without a big-bang
 * rewrite of the error handling.
 */
impl From<VilsendError> for AppError {
    fn from(error: VilsendError) -> Self {
        match error {
            VilsendError::Unauthenticated => Self::NotAuthenticated,
            VilsendError::Authentication(message) => Self::Auth(message),
            VilsendError::NotConnected => Self::NotConnected,
            VilsendError::NoRoute => Self::Network("no route to peer".into()),
            VilsendError::Network(detail) => Self::Network(detail),
            VilsendError::Internal(detail) => Self::Internal(detail),
            VilsendError::Serialization(detail) => {
                Self::Internal(format!("serialization error: {detail}"))
            }
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<AppError> for VilsendError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::NotAuthenticated => Self::Unauthenticated,
            /*
             * `AppError::Auth` carries the message the UI renders verbatim, and
             * `VilsendError::Authentication` carries it across. Both report
             * `ErrorKind::Unauthenticated`, so a shell that only cares about
             * the class is unaffected, and one that wants to show the reason
             * still has it. The round trip is lossless in both directions.
             */
            AppError::Auth(message) => Self::Authentication(message),
            AppError::NotConnected => Self::NotConnected,
            AppError::Internal(detail) => Self::Internal(detail),
            AppError::Network(detail) => Self::Network(detail),
            AppError::Serialization(error) => Self::Serialization(error.to_string()),
            AppError::Tauri(detail) => Self::Internal(format!("tauri error: {detail}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vilsend_core::ErrorKind;

    #[test]
    fn app_error_still_serialises_as_a_flat_string() {
        // This is the wire contract the webview already parses. Phase 2 must
        // not change it, whatever the error model grows into.
        let json = serde_json::to_string(&AppError::NotAuthenticated).expect("serialisable");

        assert_eq!(json, "\"not authenticated\"");
        assert_eq!(
            serde_json::to_string(&AppError::Auth("session expired".into())).unwrap(),
            "\"session expired\""
        );
    }

    #[test]
    fn a_core_error_keeps_its_class_through_app_error() {
        // Compared through `Display` because `AppError::Serialization` holds a
        // `serde_json::Error`, which is not `PartialEq`.
        assert!(matches!(
            AppError::from(VilsendError::Unauthenticated),
            AppError::NotAuthenticated
        ));
        assert_eq!(
            AppError::from(VilsendError::Network("reset".into())).to_string(),
            "network error: reset"
        );
        assert_eq!(
            AppError::from(VilsendError::NotFound("transfer-1".into())).to_string(),
            "internal error: not found: transfer-1"
        );
    }

    #[test]
    fn an_auth_failure_keeps_its_message_through_vilsend_error() {
        // The message on `AppError::Auth` is rendered verbatim in the UI. The
        // boundary type must not be a one-way door that swallows it.
        let original = AppError::Auth("your session expired".into());

        let bridged: VilsendError = original.into();

        assert_eq!(bridged.kind(), ErrorKind::Unauthenticated);
        assert_eq!(bridged.to_string(), "your session expired");

        let back: AppError = bridged.into();

        assert_eq!(back.to_string(), "your session expired");
        // And `AppError` still crosses the IPC boundary as the flat string the
        // webview already parses.
        assert_eq!(
            serde_json::to_string(&back).expect("serialisable"),
            "\"your session expired\""
        );
    }

    #[test]
    fn an_app_error_keeps_its_class_through_vilsend_error() {
        let authenticated: VilsendError = AppError::NotAuthenticated.into();
        let network: VilsendError = AppError::Network("reset".into()).into();

        assert_eq!(authenticated.kind(), ErrorKind::Unauthenticated);
        assert_eq!(network.kind(), ErrorKind::Network);
        assert_eq!(network.to_string(), "network error: reset");
    }
}
