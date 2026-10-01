pub mod atomic_file;
pub mod chunker;
pub mod cleanup;
pub mod constants;
// The frozen v1 crypto and the v2 crypto beside it. Both carry their own
// module docs; what matters here is that they are two modules and not one with
// a flag, so that neither can reach into the other's key derivation.
pub mod crypto;
#[cfg(feature = "protocol-v2")]
pub mod crypto_v2;
pub mod errors;
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
pub mod state;
pub mod upload;
pub mod writer;
