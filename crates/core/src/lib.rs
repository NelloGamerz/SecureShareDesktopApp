//! The platform-agnostic core of VilSend.
//!
//! Nothing in this crate knows about Tauri, the filesystem, the network or the
//! keychain. It holds the domain types that the desktop shell, and later the
//! CLI, the SDK and the mobile shells, all share:
//!
//! - the domain identifiers ([`TransferId`], [`DeviceId`], [`PeerRef`]),
//! - the transfer and connection lifecycle enums ([`TransferStatus`],
//!   [`ConnectionStatus`]),
//! - transfer progress and its arithmetic ([`TransferProgress`]),
//! - the boundary error model ([`VilsendError`], [`ErrorKind`], ADR-0003),
//! - the event port ([`DomainEvent`], [`EventSink`], ADR-0012),
//! - the wire protocol version ([`PROTOCOL_VERSION`], ADR-0010).
//!
//! `cargo tree -p vilsend-core` is held in CI to `serde` and `thiserror` alone;
//! the dependency direction in ADR-0002 is enforced by an architecture lint
//! rather than by convention.

pub mod error;
pub mod event;
pub mod ids;
pub mod progress;
pub mod protocol;
pub mod status;

pub use error::{ErrorKind, VilsendError};
pub use event::{DomainEvent, EventSink, RecordingEventSink};
pub use ids::{DeviceId, PeerRef, TransferId};
pub use progress::{download_progress, upload_progress, TransferProgress};
pub use protocol::{PROTOCOL_V1, PROTOCOL_V2, PROTOCOL_VERSION};
pub use status::{ConnectionStatus, TransferStatus};
