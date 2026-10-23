pub mod atomic_file;
/// The durable chunk-write core, shared by the v1 HTTP handler and the v2
/// framed session. One implementation of the write order, so the two protocols
/// cannot drift apart on it.
pub mod chunk_sink;
pub mod chunker;
pub mod cleanup;
/// Per-chunk compression, applied after the chunk is read and before it is
/// sealed. Negotiated, so a peer that does not advertise it keeps the frozen v1
/// wire.
pub mod compression;
pub mod constants;
// The frozen v1 crypto and the v2 crypto beside it. Both carry their own
// module docs; what matters here is that they are two modules and not one with
// a flag, so that neither can reach into the other's key derivation.
pub mod crypto;
#[cfg(feature = "protocol-v2")]
pub mod crypto_v2;
pub mod errors;
/// The v2 wire framing. Gated with the rest of v2: it names the v2 AEAD's
/// nonce and tag widths, so it has nothing to say in a build that cannot
/// speak v2.
#[cfg(feature = "protocol-v2")]
pub mod frame;
pub mod http_client;
pub mod integrity;
pub mod manager;
pub mod manifest;
pub mod merger;
pub mod persist;
pub mod progress;
/// The end-to-end suite: the real sender against the real receiver, interrupted
/// every way it can be.
#[cfg(test)]
mod resumability_tests;
pub mod resume;
pub mod retry;
pub mod scanner;
pub mod scheduler;
/// The seam between the transfer engine and whichever protocol carries a
/// transfer. `LegacyV1Session` is the frozen v1 wire behind that seam.
pub mod session;
pub mod state;
pub mod upload;
pub mod writer;
