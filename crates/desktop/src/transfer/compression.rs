//! Per-chunk compression.
//!
//! A file is split into chunks (`transfer::chunker`), read one chunk at a time
//! (`transfer::upload`), and sealed (`transfer::session`). This module sits
//! between the read and the seal: the plaintext chunk is compressed first, and
//! what is encrypted is the compressed bytes.
//!
//! # Why compression is negotiated rather than always on
//!
//! The v1 wire is frozen (ADR-0010). An installed receiver that receives a
//! compressed body writes the compressed bytes into `<index>.part` and reports
//! success, so the corruption would only surface at the final integrity check —
//! as a transfer that can never complete. There is no safe way to "try it and
//! fall back", because the failure is silent.
//!
//! So the receiver advertises what it can decode on `GET /transfer/public-key`
//! ([`NEGOTIATION_HEADER`]), and the sender compresses only when that header says
//! `zstd`. A receiver that predates this module sends no header, receives
//! identity-encoded chunks exactly as before, and the compatibility fixture in
//! `tests/compat_v1.rs` is unaffected.
//!
//! Each chunk carries [`CHUNK_ENCODING_HEADER`] when it is compressed, so the
//! receiver knows per chunk. The header is absent for an identity chunk, which
//! keeps the common incompressible-media case at zero overhead.
//!
//! # What is compressed
//!
//! The chunk's plaintext, before encryption. That ordering matters:
//!
//! * the manifest records the hash and length of the **plaintext**, so resume
//!   queries and the sender's `chunk_digest` continue to agree;
//! * the sender's `ExpectedTransfer` digests are over plaintext, so the final
//!   integrity check is unchanged;
//! * progress counts the bytes the receiver can actually produce.
//!
//! Compressing after encryption would achieve nothing (ciphertext is
//! incompressible) and would leave the integrity metadata describing bytes
//! nobody holds.
//!
//! # Failure behaviour
//!
//! On the sender, compression is best-effort: a failure to compress, or a
//! result that is not smaller than the input, sends the chunk uncompressed. A
//! transfer must not fail because a compressor did.
//!
//! On the receiver, a chunk that declares `zstd` but does not decompress is
//! refused, because the alternative is writing bytes the sender never meant.

use crate::transfer::errors::{Result, TransferError};

/// The response header a receiver uses to advertise the chunk encodings it can
/// decode.
///
/// Sent on `GET /transfer/public-key`, which every session calls before its
/// first chunk on both a fresh transfer and a resume.
pub const NEGOTIATION_HEADER: &str = "vilsend-chunk-encoding";

/// The request header that names how one chunk body is encoded.
///
/// Absent means [`ChunkEncoding::Identity`].
pub const CHUNK_ENCODING_HEADER: &str = "chunk-encoding";

/// The wire name of the compressor this build speaks.
pub const ZSTD_NAME: &str = "zstd";

/// The zstd level used for chunk bodies.
///
/// Level 3 is zstd's default: it compresses 16 MiB chunks quickly enough that
/// eight in-flight chunks still saturate a typical link, while recovering most
/// of the ratio a higher level would give.
const ZSTD_LEVEL: i32 = 3;

/// The decompressed size a chunk is allowed to reach when the sender did not
/// describe its chunks.
///
/// Senders built alongside this receiver always include an
/// `ExpectedTransfer`, so the real bound is the expected chunk length. This is
/// the fallback, not the usual path. It exists so that a compressed chunk from a
/// peer that omitted the description cannot be used as an unbounded allocation
/// (a "zip bomb" of a few MiB expanding to gigabytes).
pub const MAX_DECOMPRESSED_CHUNK_BYTES: usize = 64 * 1024 * 1024;

/// How one chunk body is encoded on the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChunkEncoding {
    /// The chunk's plaintext, as it has always been sent.
    #[default]
    Identity,
    /// zstd-compressed plaintext.
    Zstd,
}

impl ChunkEncoding {
    /// The name this encoding carries on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Zstd => ZSTD_NAME,
        }
    }

    /// Parse a header value. Unknown values are `None`, never a guess.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "identity" => Some(Self::Identity),
            ZSTD_NAME => Some(Self::Zstd),
            _ => None,
        }
    }
}

/// Compress one chunk, or say that the result is not worth sending.
///
/// Returns `Ok(None)` when compression would not shrink the chunk. That is the
/// common answer for already-compressed media, and it keeps the sender from
/// paying zstd's frame overhead to send more bytes than it started with.
///
/// This is CPU-bound and callers on the async runtime should use
/// `tokio::task::spawn_blocking`; the transfer path does.
pub fn compress(plaintext: &[u8]) -> Result<Option<Vec<u8>>> {
    let compressed = zstd::bulk::compress(plaintext, ZSTD_LEVEL)
        .map_err(|error| TransferError::Compression(error.to_string()))?;

    if compressed.len() < plaintext.len() {
        Ok(Some(compressed))
    } else {
        Ok(None)
    }
}

/// Decompress one chunk, refusing anything that expands past `limit`.
///
/// `limit` is the plaintext length the sender said this chunk has. Passing it as
/// the scratch capacity is what makes the bound real rather than a check after
/// the allocation.
pub fn decompress(encoded: &[u8], limit: usize) -> Result<Vec<u8>> {
    let plaintext = zstd::bulk::decompress(encoded, limit)
        .map_err(|error| TransferError::Compression(error.to_string()))?;

    if plaintext.len() > limit {
        return Err(TransferError::Compression(format!(
            "the chunk expands past its {limit}-byte limit"
        )));
    }

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compressible_chunks_shrink_and_round_trip() {
        let plaintext = vec![b'a'; 64 * 1024];

        let compressed = compress(&plaintext)
            .expect("compression should succeed")
            .expect("a run of one byte must compress");

        assert!(compressed.len() < plaintext.len());

        let restored = decompress(&compressed, plaintext.len()).expect("it must decompress");

        assert_eq!(restored, plaintext);
    }

    /// Incompressible input must be sent as-is rather than made larger.
    #[test]
    fn incompressible_chunks_are_left_alone() {
        let mut state = 0x1234_5678_u32;
        let plaintext: Vec<u8> = (0..4096)
            .map(|_| {
                // xorshift: deterministic, no dependency, and incompressible.
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();

        assert_eq!(
            compress(&plaintext).expect("compression should succeed"),
            None
        );
    }

    #[test]
    fn an_unknown_encoding_is_not_guessed() {
        assert_eq!(ChunkEncoding::parse("zstd"), Some(ChunkEncoding::Zstd));
        assert_eq!(ChunkEncoding::parse(" ZSTD "), Some(ChunkEncoding::Zstd));
        assert_eq!(
            ChunkEncoding::parse("identity"),
            Some(ChunkEncoding::Identity)
        );
        assert_eq!(ChunkEncoding::parse("gzip"), None);
        assert_eq!(ChunkEncoding::parse(""), None);
    }

    /// A body that is not a zstd frame must be an error, not a panic or a
    /// truncated write.
    #[test]
    fn a_body_that_is_not_zstd_is_refused() {
        let error = decompress(b"not a zstd frame", 1024).expect_err("garbage must not decode");

        assert!(matches!(error, TransferError::Compression(_)));
    }

    /// The bound is enforced by the allocation scratch, before the bytes are
    /// materialised. A compressed chunk that claims a huge plaintext is the
    /// case this exists for.
    #[test]
    fn decompression_past_the_limit_is_refused() {
        let plaintext = vec![b'a'; 128 * 1024];
        let compressed = compress(&plaintext)
            .expect("compression should succeed")
            .expect("the input compresses");

        assert!(decompress(&compressed, 1024).is_err());
    }
}
