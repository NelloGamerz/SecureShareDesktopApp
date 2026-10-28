/// The smallest chunk a sender plans.
pub const MIN_CHUNK_SIZE: usize = 256 * 1024;

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
