use std::time::Duration;

/// The smallest chunk a sender plans.
pub const MIN_CHUNK_SIZE: usize = 256 * 1024;

/// How long a handshake request may wait for the receiver's answer.
///
/// A route that accepts the connection but never answers would otherwise hold
/// `start` for the life of the process. That is worse than a slow transfer:
/// commands run on the WebSocket receive task, so a start that never returns
/// means every later `START_TRANSFER` is never read. Bounding it is also what
/// lets `FailoverV1Session` treat the route as failed and move on.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// How long opening a session may take. Generous, because a receiver
/// re-opening a transfer checks the chunks it already holds against its disk
/// before answering, which for a large resume is real work.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(120);

/// The longest a TCP connect may take before the route counts as unreachable.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The largest chunk a sender plans, whatever it is configured with.
///
/// This is **not** [`MAX_CHUNK_SIZE`]: the receiver still accepts bodies up to
/// that limit so installed senders keep working. The cap is a sender policy —
/// a chunk this size fails and is retried in well under a second on a slow
/// link, whereas a 16 MiB chunk made one stall cost twenty.
pub const MAX_PLANNED_CHUNK_SIZE: usize = 512 * 1024;

/// The largest chunk any sender may configure.
///
/// The receiver's request-body limit and the v2 frame cap are both derived from
/// this, so a chunk size above it would be accepted by the planner and refused
/// by the wire. It exists so the two cannot disagree.
pub const MAX_CHUNK_SIZE: usize = 32 * 1024 * 1024;

/// The chunk size a sender falls back to when the receiver does not say what it
/// can accept.
///
/// This is a *maximum*, so an installed receiver — whose request-body limit is
/// 10 MiB — accepts it comfortably. The sender never plans above
/// [`MAX_PLANNED_CHUNK_SIZE`] anyway.
pub const LEGACY_CHUNK_SIZE: usize = MAX_PLANNED_CHUNK_SIZE;

/// The response header a receiver uses to state the largest chunk body it will
/// accept.
pub const MAX_CHUNK_BYTES_HEADER: &str = "vilsend-max-chunk-bytes";

/// How many chunks may be in flight at once.
///
/// This is the width of the scheduler's sliding window and the pool size the
/// HTTP client keeps warm. With chunks capped at 512 KiB, 16 in flight is at
/// most 8 MiB of chunk buffers per process.
pub const DEFAULT_CONCURRENCY: usize = 16;

pub const DEFAULT_MAX_RETRIES: u32 = 4;

pub const RECEIVER_PORT: u16 = 7878;

/// How long an active download may go without a chunk before the receiver
/// treats it as interrupted.
///
/// Silence is the only signal a receiver has that the sender is gone: there is
/// no session-close request in the protocol. The window is generous because a
/// large chunk on a slow link takes real time to encrypt, send and write —
/// marking that interrupted would fight the transfer it is meant to protect.
pub const DOWNLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(120);

/// How often the receiver looks for stalled downloads.
pub const DOWNLOAD_STALL_CHECK_INTERVAL: Duration = Duration::from_secs(15);
