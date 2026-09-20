//! Transfer and connection lifecycle enums.
//!
//! Both are moved verbatim from the desktop shell
//! (`crates/desktop/src/models/transfer.rs` and `crates/desktop/src/models/mod.rs`).
//! The serde attributes *are* the wire format the webview reads, so they stay
//! exactly as they were.

use serde::{Deserialize, Serialize};

/// The lifecycle of one transfer, from either side of it.
///
/// `SCREAMING_SNAKE_CASE` is the existing wire encoding; the frontend compares
/// against these strings.
///
/// `#[non_exhaustive]` because `vilsend-sdk` **re-exports this type** as part
/// of its public API, and `04-sdk-cli-mobile-build-plan.md` §2.5 requires every
/// public enum in that API to be open to new variants. The attribute is a
/// Rust-side guarantee only: it changes nothing on the wire, and the golden
/// fixtures in `tests/golden_payloads.rs` still pin the eight variants exactly.
///
/// Phase 5 added it. `ConnectionStatus` is deliberately *not* marked: the SDK
/// does not re-export it, and nothing outside the shell sees it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransferStatus {
    Queued,
    Uploading,
    Paused,
    Completed,
    Failed,
    Cancelled,
    Pending,
    Downloading,
}

/// The lifecycle of the control-plane websocket.
///
/// Deliberately has no `rename_all`: the variants are emitted in Rust casing
/// (`"Connected"`, not `"connected"`) and an `Error` becomes
/// `{"Error": "..."}`. The frontend was corrected to match this in Phase 1
/// (task 1.12); changing it now would break it again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Error(String),
}
