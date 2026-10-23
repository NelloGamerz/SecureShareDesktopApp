/// The size of one chunk, in bytes.
///
/// 16 MiB is the default because the per-request cost — a round trip, a header
/// set, an AEAD tag, a manifest read-modify-write on the receiver — is paid once
/// per chunk. Larger chunks amortise that better over a fast link; past this
/// point the win is small and a chunk that fails has to be re-read and re-sent
/// whole, so retry latency grows with it.
pub const DEFAULT_CHUNK_SIZE: usize = 16 * 1024 * 1024;

/// The largest chunk any sender may configure.
///
/// The receiver's request-body limit and the v2 frame cap are both derived from
/// this, so a chunk size above it would be accepted by the planner and refused
/// by the wire. It exists so the two cannot disagree.
pub const MAX_CHUNK_SIZE: usize = 32 * 1024 * 1024;

/// The chunk size a sender falls back to when the receiver does not say what it
/// can accept.
///
/// This is what installed receivers have always been sent: their request-body
/// limit is 10 MiB, so it is the largest value that is safe without negotiation.
/// A sender that assumed the new 16 MiB default would get `413` from every
/// installed peer.
pub const LEGACY_CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// The response header a receiver uses to state the largest chunk body it will
/// accept.
pub const MAX_CHUNK_BYTES_HEADER: &str = "vilsend-max-chunk-bytes";

/// How many chunks may be in flight at once.
///
/// Each one holds a full chunk buffer on both sides, so this is a memory bound
/// as much as a throughput setting: 8 × 16 MiB is 128 MiB of chunk buffers per
/// process at the default chunk size.
pub const DEFAULT_CONCURRENCY: usize = 8;

pub const DEFAULT_MAX_RETRIES: u32 = 4;

pub const RECEIVER_PORT: u16 = 7878;
