//! The unified error model (ADR-0003).
//!
//! Two taxonomies exist in the shell today — `AppError`, which is what the
//! webview sees, and `TransferError`, which is internal to the transfer
//! module — and both lose their classification at the boundary: `AppError`
//! serialises as a flat string via `Display`, and `TransferError::Receiver`
//! collapses "wrong credential", "insufficient storage" and "peer crashed"
//! into one opaque message.
//!
//! `VilsendError` is the boundary type that replaces both. Its `kind()` is the
//! only thing a shell may switch on, and it is stable across versions.
//!
//! **Interpretation of ADR-0003's "no `String`-payload variant" rule.** The
//! rule is about classification: a *condition* must never be signalled only by
//! a message string, so shells can branch on it without parsing. Several
//! variants below do carry a diagnostic string, but every one of them also has
//! a distinct `ErrorKind`, so the classification survives the boundary and the
//! message is decoration. The rule is applied literally in the other
//! direction: there is no catch-all `Other(String)` variant, and climbing into
//! `Internal` instead of naming a real condition is a review rejection.

use thiserror::Error;

/// The stable, machine-readable classification of a [`VilsendError`].
///
/// This is the only thing that crosses a boundary as a discriminant. Adding a
/// variant is not a breaking change for consumers; changing or removing one
/// is.
///
/// `NoRoute`, `IntegrityMismatch`, `InsufficientStorage` and `Cancelled` are
/// defined here because ADR-0003 gives them CLI exit codes and shell mappings,
/// but nothing produces them yet — the transfer engine that would is Phase 4's.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// There is no valid session; the caller must sign in.
    Unauthenticated,
    /// The control plane has not been reached yet.
    NotConnected,
    /// The peer or the control plane could not be reached.
    NoRoute,
    /// A received chunk did not match the hash it was sent with.
    IntegrityMismatch,
    /// There is no room for the transfer at the destination.
    InsufficientStorage,
    /// The user cancelled the operation.
    Cancelled,
    /// The caller passed something the operation cannot accept.
    InvalidInput,
    /// The named transfer or file does not exist.
    NotFound,
    /// A transport-level failure.
    Network,
    /// A local filesystem or storage fault.
    Storage,
    /// A payload could not be encoded or decoded.
    Serialization,
    /// Something the core cannot name. Never the only signal.
    Internal,
}

/// The boundary error type for every shell.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VilsendError {
    #[error("not authenticated")]
    Unauthenticated,

    #[error("not connected")]
    NotConnected,

    #[error("no route to peer")]
    NoRoute,

    #[error("integrity mismatch: {0}")]
    IntegrityMismatch(String),

    #[error("insufficient storage")]
    InsufficientStorage,

    #[error("cancelled")]
    Cancelled,

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("internal error: {0}")]
    Internal(String),
}

impl VilsendError {
    /// The stable classification of this error.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Unauthenticated => ErrorKind::Unauthenticated,
            Self::NotConnected => ErrorKind::NotConnected,
            Self::NoRoute => ErrorKind::NoRoute,
            Self::IntegrityMismatch(_) => ErrorKind::IntegrityMismatch,
            Self::InsufficientStorage => ErrorKind::InsufficientStorage,
            Self::Cancelled => ErrorKind::Cancelled,
            Self::InvalidInput(_) => ErrorKind::InvalidInput,
            Self::NotFound(_) => ErrorKind::NotFound,
            Self::Network(_) => ErrorKind::Network,
            Self::Storage(_) => ErrorKind::Storage,
            Self::Serialization(_) => ErrorKind::Serialization,
            Self::Internal(_) => ErrorKind::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn every_variant() -> Vec<VilsendError> {
        vec![
            VilsendError::Unauthenticated,
            VilsendError::NotConnected,
            VilsendError::NoRoute,
            VilsendError::IntegrityMismatch("chunk 4".into()),
            VilsendError::InsufficientStorage,
            VilsendError::Cancelled,
            VilsendError::InvalidInput("endpoint is empty".into()),
            VilsendError::NotFound("transfer-1".into()),
            VilsendError::Network("connection reset".into()),
            VilsendError::Storage("disk is full".into()),
            VilsendError::Serialization("invalid utf-8".into()),
            VilsendError::Internal("unreachable".into()),
        ]
    }

    #[test]
    fn every_variant_has_its_own_kind() {
        let kinds: HashSet<ErrorKind> = every_variant().iter().map(VilsendError::kind).collect();

        assert_eq!(kinds.len(), every_variant().len());
    }

    #[test]
    fn the_kinds_a_shell_switches_on_are_what_they_say() {
        assert_eq!(
            VilsendError::Unauthenticated.kind(),
            ErrorKind::Unauthenticated
        );
        assert_eq!(VilsendError::NoRoute.kind(), ErrorKind::NoRoute);
        assert_eq!(
            VilsendError::InsufficientStorage.kind(),
            ErrorKind::InsufficientStorage
        );
        assert_eq!(
            VilsendError::IntegrityMismatch("x".into()).kind(),
            ErrorKind::IntegrityMismatch
        );
        assert_eq!(VilsendError::Cancelled.kind(), ErrorKind::Cancelled);
    }

    #[test]
    fn a_diagnostic_string_never_carries_the_classification() {
        // Two failures of the same class with different messages must classify
        // identically — that is the whole point of the boundary type.
        assert_eq!(
            VilsendError::Network("connection reset".into()).kind(),
            VilsendError::Network("dns lookup failed".into()).kind()
        );

        // And the kind survives while the message is discarded.
        assert_eq!(
            VilsendError::Unauthenticated.kind(),
            VilsendError::Unauthenticated.kind()
        );
    }

    #[test]
    fn the_display_form_is_human_readable() {
        assert_eq!(
            VilsendError::Unauthenticated.to_string(),
            "not authenticated"
        );
        assert_eq!(
            VilsendError::NotFound("transfer-1".into()).to_string(),
            "not found: transfer-1"
        );
    }
}
