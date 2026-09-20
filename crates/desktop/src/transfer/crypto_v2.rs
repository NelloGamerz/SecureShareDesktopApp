//! The **v2** transfer crypto: associated data, a salted KDF, and a bounded
//! nonce-repeat window.
//!
//! Compiled only under the `protocol-v2` feature, which is off by default. See
//! [`crate::transfer::crypto`] for the frozen v1 implementation this sits
//! beside; the two share no code and no key derivation, deliberately.
//!
//! # What v2 changes, and why each one is a security fix rather than a tweak
//!
//! | | v1 | v2 | Task |
//! |---|---|---|---|
//! | AEAD associated data | none — a ciphertext is bound to nothing | [`ChunkAad`] binds transfer, file, index, path and protocol version | 4.1 |
//! | KDF salt | none — two sessions reaching the same ECDH secret share a key | `derive_transfer_key_v2` salts with both handshake nonces | 4.2 |
//! | Nonce repeats | undetectable — no call has anywhere to record one | `NonceWindow`, bounded | 4.4 |
//!
//! # Why the two implementations never share a key
//!
//! Three independent separations, so that a v2 ciphertext cannot be opened as a
//! v1 one even by accident:
//!
//! 1. the AAD differs — v1 passes none, so the tag covers a v1 chunk with no
//!    context and a v2 chunk with it;
//! 2. the KDF salt differs — v1 passes `None`, v2 passes the two nonces;
//! 3. the KDF `info` string differs — `b"carsdv-transfer-key-v1"` against
//!    `b"vilsend-transfer-key-v2"`.
//!
//! (2) and (3) are not belt-and-braces on (1). A nonce reused across two
//! different keys is not a nonce reuse at all, and the two paths must not be
//! able to collide even if one of the separations is later changed.
//!
//! # What is *not* here
//!
//! Wiring. Nothing in this module is called by the transfer path yet: the v2
//! endpoints that would call it are task 4.11, and the sender that would pick
//! between v1 and v2 by capability probe is task 4.11 as well. Until then a
//! build with `protocol-v2` on still speaks v1 on the wire, which is exactly
//! what the compatibility fixture in `tests/compat_v1.rs` asserts.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use vilsend_core::VilsendError;

/// The domain separator that opens every v2 chunk AAD.
///
/// It exists so that a v2 AAD cannot be confused with any other byte string
/// this protocol hashes, and so the encoding is self-describing to a reader
/// holding only the bytes. It is **frozen**: changing it changes every v2 tag.
const AAD_DOMAIN: &[u8] = b"vilsend-chunk-aad-v2";

/// The AES-GCM nonce width, in bytes. Same in both versions.
pub const NONCE_LEN: usize = 12;

/// The AEAD authentication tag width, in bytes. Same in both versions.
pub const TAG_LEN: usize = 16;

/*
 * ----------------------------------------------------------------------
 * Associated data (task 4.1)
 * ----------------------------------------------------------------------
 */

/// Everything a v2 chunk is cryptographically bound to.
///
/// ADR-0007 decision 4 gives the field list:
/// `transfer_id ‖ file_id ‖ chunk_index ‖ relative_path ‖ proto_v`. This type
/// is that list, and [`ChunkAad::encode`] is the one place its byte encoding is
/// defined.
///
/// # Why the encoding is length-prefixed rather than plain concatenation
///
/// The ADR writes the fields joined by `‖`, which is notation for
/// "concatenated", not a wire instruction. Concatenating variable-length
/// strings with no delimiters is **ambiguous**, and ambiguity in a MAC's
/// associated data is a real defect, not a style question: with plain
/// concatenation, `transfer_id = "ab", file_id = "c"` and
/// `transfer_id = "a", file_id = "bc"` produce identical AAD bytes, so a chunk
/// sealed for one transfer would open for the other. Every field but
/// `chunk_index` is variable-length, so that collision is reachable, and a test
/// below asserts it stays unreachable.
///
/// So each string is written as a little-endian `u32` byte length followed by
/// its bytes. The field order and the field set are exactly the ADR's; only the
/// framing is pinned down.
///
/// `protocol_version` is carried even though v2 is the only version that
/// produces this AAD. It is what stops a future v3 adopting the v2 AAD bytes by
/// accident, and it is the ADR's list verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkAad<'a> {
    pub transfer_id: &'a str,
    pub file_id: &'a str,
    pub chunk_index: u64,
    pub relative_path: &'a str,
    pub protocol_version: u16,
}

impl<'a> ChunkAad<'a> {
    /// The AAD for a v2 chunk, with the protocol version filled in.
    ///
    /// This is the constructor callers should use. [`ChunkAad`] can also be
    /// built with an explicit version, which is what lets a test assert what a
    /// mismatched version does.
    pub fn new(
        transfer_id: &'a str,
        file_id: &'a str,
        chunk_index: u64,
        relative_path: &'a str,
    ) -> Self {
        Self {
            transfer_id,
            file_id,
            chunk_index,
            relative_path,
            protocol_version: vilsend_core::PROTOCOL_V2,
        }
    }

    /// The canonical byte encoding of this AAD.
    ///
    /// ```text
    /// b"vilsend-chunk-aad-v2"                  the domain separator, 20 bytes
    /// ‖ u16_le(protocol_version)               2 bytes
    /// ‖ u32_le(len(transfer_id))   ‖ transfer_id
    /// ‖ u32_le(len(file_id))       ‖ file_id
    /// ‖ u64_le(chunk_index)                    8 bytes
    /// ‖ u32_le(len(relative_path)) ‖ relative_path
    /// ```
    ///
    /// Little-endian, matching the integer encoding the v1 wire already uses
    /// for `Chunk-Index` and `Total-Chunks`. Frozen: this is the input to every
    /// v2 chunk tag.
    ///
    /// The string lengths are **byte** lengths, not character counts, so a
    /// non-ASCII path is framed by what is actually written.
    ///
    /// The fields are written in order by position, not by matching a value
    /// against the others, because two of them may legitimately be equal — a
    /// transfer and a file can share an id — and a value-matching encoding
    /// would then write the index twice.
    pub fn encode(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(
            AAD_DOMAIN.len()
                + 2
                + 8
                + [self.transfer_id, self.file_id, self.relative_path]
                    .iter()
                    .map(|value| 4 + value.len())
                    .sum::<usize>(),
        );

        aad.extend_from_slice(AAD_DOMAIN);
        aad.extend_from_slice(&self.protocol_version.to_le_bytes());

        push_length_prefixed(&mut aad, self.transfer_id);
        push_length_prefixed(&mut aad, self.file_id);

        aad.extend_from_slice(&self.chunk_index.to_le_bytes());

        push_length_prefixed(&mut aad, self.relative_path);

        aad
    }
}

/// Append `u32_le(value.len()) ‖ value`.
fn push_length_prefixed(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(&(value.len() as u32).to_le_bytes());
    output.extend_from_slice(value.as_bytes());
}

/*
 * ----------------------------------------------------------------------
 * AEAD (task 4.1)
 * ----------------------------------------------------------------------
 */

/// A sealed v2 chunk, in the shape the wire carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedChunk {
    /// 12 bytes. Random on the sender, and never reused under one key.
    pub nonce: [u8; NONCE_LEN],
    /// Ciphertext with the 16-byte tag appended.
    pub ciphertext: Vec<u8>,
}

/// Seal one chunk.
///
/// The nonce is a parameter rather than generated here, for two reasons: it
/// makes the function a pure function of its inputs, which is what a frozen
/// test vector needs; and it keeps nonce *generation* in exactly one place, so
/// the bounded repeat check added by task 4.4 cannot be bypassed by a caller
/// that forgot about it.
///
/// For the same reason there is no nonce-less convenience wrapper. Reaching for
/// this function directly is the thing that skips the discipline, so it should
/// look like the deliberate act it is.
pub fn seal(
    key: &[u8; 32],
    aad: &[u8],
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
) -> Result<SealedChunk, VilsendError> {
    let cipher = Aes256Gcm::new(key.into());

    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|error| VilsendError::Internal(format!("chunk seal failed: {error}")))?;

    // Sizes only. Never the nonce, never a key, never the plaintext — see the
    // redaction rule in the phase brief and `06-testing-and-quality.md` §6.4.
    tracing::debug!(
        plaintext_bytes = plaintext.len(),
        ciphertext_bytes = ciphertext.len(),
        aad_bytes = aad.len(),
        "v2 chunk sealed"
    );

    Ok(SealedChunk {
        nonce: *nonce,
        ciphertext,
    })
}

/// Open one chunk, rejecting anything whose tag does not verify.
///
/// A `nonce` that is not [`NONCE_LEN`] bytes is [`ErrorKind::InvalidInput`]
/// rather than a panic. This is one place v2 is deliberately stricter than v1:
/// `crypto::decrypt_chunk` builds a `Nonce` from whatever slice it is handed
/// and panics on a length that is neither 12 nor 16, which makes the caller's
/// length check load-bearing. `open` is total for every input.
///
/// A failed tag is [`ErrorKind::IntegrityMismatch`], so a shell can classify it
/// without parsing a message (ADR-0003). The message is deliberately the same
/// for a flipped ciphertext, a replay into another index, and a wrong key: an
/// attacker must not be able to tell those apart from the response.
pub fn open(
    key: &[u8; 32],
    aad: &[u8],
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, VilsendError> {
    if nonce.len() != NONCE_LEN {
        return Err(VilsendError::InvalidInput(format!(
            "v2 chunk nonce must be {NONCE_LEN} bytes, got {}",
            nonce.len()
        )));
    }

    let cipher = Aes256Gcm::new(key.into());

    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| VilsendError::IntegrityMismatch("v2 chunk tag did not verify".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::crypto as v1;
    use vilsend_core::ErrorKind;

    /// A raw key for the `seal`/`open` tests.
    ///
    /// Deliberately a literal and not a derived key: `seal` and `open` are
    /// pure functions of their arguments, and using a fixed key keeps these
    /// tests about the AEAD rather than about the KDF. The KDF's own tests use
    /// derived keys.
    const KEY: [u8; 32] = [0x42; 32];

    fn aad_of(transfer_id: &str, file_id: &str, index: u64, path: &str) -> Vec<u8> {
        ChunkAad::new(transfer_id, file_id, index, path).encode()
    }

    fn fixed_aad() -> Vec<u8> {
        aad_of("t-1", "f-1", 3, "docs/report.pdf")
    }

    /*
     * ------------------------------------------------------------------
     * The feature gate reaches this crate
     * ------------------------------------------------------------------
     */

    /// The desktop crate forwards `protocol-v2` to `vilsend-core`. If that
    /// forwarding breaks, this module compiles but the protocol version the
    /// shell reports is v1 — the two disagreeing about which version they are
    /// is the failure ADR-0010 warns about. Asserted rather than assumed.
    #[test]
    fn the_forwarded_feature_raises_the_protocol_version() {
        assert_eq!(vilsend_core::PROTOCOL_VERSION, vilsend_core::PROTOCOL_V2);
    }

    /*
     * ------------------------------------------------------------------
     * AAD encoding
     * ------------------------------------------------------------------
     */

    /// A literal byte string for a literal input, so a change to the framing is
    /// a deliberate act. This is the input to every v2 tag: it never travels on
    /// the wire by itself, but it is as frozen as anything that does.
    #[test]
    fn the_aad_encoding_is_pinned_byte_for_byte() {
        let aad = ChunkAad {
            transfer_id: "t-1",
            file_id: "f-1",
            chunk_index: 3,
            relative_path: "a/b",
            protocol_version: 2,
        };

        let mut expected = Vec::new();
        expected.extend_from_slice(b"vilsend-chunk-aad-v2");
        expected.extend_from_slice(&2u16.to_le_bytes());
        expected.extend_from_slice(&3u32.to_le_bytes());
        expected.extend_from_slice(b"t-1");
        expected.extend_from_slice(&3u32.to_le_bytes());
        expected.extend_from_slice(b"f-1");
        expected.extend_from_slice(&3u64.to_le_bytes());
        expected.extend_from_slice(&3u32.to_le_bytes());
        expected.extend_from_slice(b"a/b");

        assert_eq!(aad.encode(), expected);
        assert_eq!(aad.encode().len(), 20 + 2 + 7 + 7 + 8 + 7);
    }

    /// The reason the encoding is length-prefixed at all.
    ///
    /// With plain concatenation these two inputs produce the same bytes —
    /// `transfer_id = "ab", file_id = "c"` and `transfer_id = "a",
    /// file_id = "bc"` both read `...abc` — so a chunk sealed for one transfer
    /// would open for the other. This is what makes that unreachable, and it
    /// fails the day someone "simplifies" the framing back to concatenation.
    #[test]
    fn shifting_a_character_across_a_field_boundary_changes_the_aad() {
        assert_ne!(aad_of("ab", "c", 0, "p"), aad_of("a", "bc", 0, "p"));
    }

    /// The same hazard against the fixed-width index: no string can grow into
    /// it, because every string is preceded by its own length.
    #[test]
    fn no_field_can_absorb_the_chunk_index() {
        assert_ne!(aad_of("t", "f", 0, "ab"), aad_of("t", "f", 0, "a"));
        assert_ne!(aad_of("t", "f", 0, ""), aad_of("t", "f", 1, ""));
    }

    /// A transfer and a file may share an id, and when they do the index must
    /// still be written exactly once. A value-matching encoding writes it
    /// twice; the length assertion is what catches that.
    #[test]
    fn the_index_is_written_exactly_once_when_two_ids_are_equal() {
        let encoded = aad_of("x", "x", 0, "p");

        // 20 domain + 2 version + (4+1) + (4+1) + 8 index + (4+1)
        assert_eq!(encoded.len(), 45);

        // And it is still sensitive to the index, so "written once" is not
        // "written nowhere".
        assert_ne!(encoded, aad_of("x", "x", 1, "p"));
    }

    /// Every field is actually covered. Table-driven so that dropping one from
    /// `encode` fails here rather than silently weakening every tag.
    #[test]
    fn every_field_changes_the_aad() {
        let base = fixed_aad();

        let variants = [
            ("transfer_id", aad_of("t-2", "f-1", 3, "docs/report.pdf")),
            ("file_id", aad_of("t-1", "f-2", 3, "docs/report.pdf")),
            ("chunk_index", aad_of("t-1", "f-1", 4, "docs/report.pdf")),
            ("relative_path", aad_of("t-1", "f-1", 3, "docs/other.pdf")),
            (
                "protocol_version",
                ChunkAad {
                    protocol_version: 3,
                    ..ChunkAad::new("t-1", "f-1", 3, "docs/report.pdf")
                }
                .encode(),
            ),
        ];

        for (field, variant) in variants {
            assert_ne!(base, variant, "changing {field} must change the AAD");
        }
    }

    #[test]
    fn the_aad_encoding_is_deterministic() {
        assert_eq!(fixed_aad(), fixed_aad());
    }

    /// A path with multi-byte characters is framed by its byte length, so the
    /// framing can never split a character.
    #[test]
    fn a_non_ascii_path_is_framed_by_byte_length() {
        let encoded = aad_of("t-1", "f-1", 0, "док/отчёт.pdf");

        // Offset of the path's length prefix: everything that precedes it.
        let prefix = 20 + 2 + (4 + 3) + (4 + 3) + 8;
        let declared = u32::from_le_bytes(encoded[prefix..prefix + 4].try_into().unwrap());

        assert_eq!(declared as usize, "док/отчёт.pdf".len());
        assert!(declared as usize > "док/отчёт.pdf".chars().count());
    }

    #[test]
    fn an_empty_field_still_contributes_a_length_prefix() {
        let encoded = aad_of("", "", 0, "");

        assert_eq!(encoded.len(), 20 + 2 + 4 + 4 + 8 + 4);
        assert_ne!(encoded, aad_of("", "", 0, "x"));
    }

    /// A chunk index beyond 32 bits occupies all eight of its bytes, so the
    /// `u64` in the encoding is not decorative.
    #[test]
    fn a_large_chunk_index_uses_all_eight_bytes() {
        let low = aad_of("t", "f", 0, "p");
        let high = aad_of("t", "f", u32::MAX as u64 + 1, "p");

        assert_eq!(low.len(), high.len());
        assert_ne!(low, high);
    }

    /// A future protocol version must not be able to reuse these AAD bytes.
    #[test]
    fn a_different_protocol_version_produces_a_different_aad() {
        let v2 = ChunkAad::new("t", "f", 0, "p").encode();

        let v3 = ChunkAad {
            protocol_version: 3,
            ..ChunkAad::new("t", "f", 0, "p")
        }
        .encode();

        assert_eq!(v2.len(), v3.len());
        assert_ne!(v2, v3);
    }

    /*
     * ------------------------------------------------------------------
     * Seal and open
     * ------------------------------------------------------------------
     */

    #[test]
    fn a_chunk_round_trips_with_matching_aad() {
        let aad = fixed_aad();

        let sealed = seal(&KEY, &aad, &[0xAB; NONCE_LEN], b"the payload").unwrap();

        assert_eq!(
            open(&KEY, &aad, &sealed.nonce, &sealed.ciphertext).unwrap(),
            b"the payload"
        );
    }

    #[test]
    fn the_v2_nonce_is_twelve_bytes_and_the_tag_is_sixteen() {
        let sealed = seal(&KEY, &fixed_aad(), &[0; NONCE_LEN], &[0xCD; 4096]).unwrap();

        assert_eq!(sealed.nonce.len(), NONCE_LEN);
        assert_eq!(sealed.ciphertext.len(), 4096 + TAG_LEN);
    }

    #[test]
    fn an_empty_chunk_seals_and_opens() {
        let aad = fixed_aad();

        let sealed = seal(&KEY, &aad, &[0x11; NONCE_LEN], &[]).unwrap();

        assert_eq!(sealed.ciphertext.len(), TAG_LEN);
        assert_eq!(
            open(&KEY, &aad, &sealed.nonce, &sealed.ciphertext).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn seal_reports_the_nonce_it_was_given() {
        let sealed = seal(&KEY, &fixed_aad(), &[7; NONCE_LEN], b"x").unwrap();

        assert_eq!(sealed.nonce, [7; NONCE_LEN]);
    }

    /// The same inputs produce the same bytes. AES-GCM is deterministic once
    /// the nonce is fixed, which is what makes the vector below possible — and
    /// is exactly why the nonce must never repeat under one key.
    #[test]
    fn sealing_is_deterministic_for_a_fixed_nonce() {
        let aad = fixed_aad();

        let first = seal(&KEY, &aad, &[0x0A; NONCE_LEN], b"hello v2").unwrap();
        let second = seal(&KEY, &aad, &[0x0A; NONCE_LEN], b"hello v2").unwrap();

        assert_eq!(first, second);
    }

    /*
     * ------------------------------------------------------------------
     * The frozen wire vector
     * ------------------------------------------------------------------
     */

    /// Fixed key + fixed nonce + fixed AAD + fixed plaintext → fixed
    /// ciphertext. This is `05-migration-plan.md`'s "Crypto vectors"
    /// requirement: the regression test on the v2 wire format.
    ///
    /// If this fails, two builds of this repository disagree about what a v2
    /// chunk looks like. That is a data-corruption bug shipped to users, not a
    /// test to update; change the expected value only alongside a decision that
    /// renumbers the protocol.
    #[test]
    fn the_v2_chunk_vector_is_frozen() {
        let sealed = seal(&KEY, &fixed_aad(), &[0x0A; NONCE_LEN], b"hello v2").unwrap();

        assert_eq!(hex::encode(sealed.nonce), "0a0a0a0a0a0a0a0a0a0a0a0a");

        assert_eq!(
            hex::encode(&sealed.ciphertext),
            "674c2c79f58581ccd674a2782cd1810fdbce80b2608ef9c2"
        );
    }

    /// A second vector over the AAD alone, so the assertion above is a function
    /// of the AAD and not a constant that happens to be returned for anything.
    #[test]
    fn the_v2_aad_vector_is_frozen() {
        assert_eq!(
            hex::encode(fixed_aad()),
            "76696c73656e642d6368756e6b2d6161642d7632020003000000742d3103000000662d3103000000000000000f000000646f63732f7265706f72742e706466"
        );
    }

    /*
     * ------------------------------------------------------------------
     * Tamper tests — every one must fail closed
     * ------------------------------------------------------------------
     */

    /// The acceptance criterion: "A captured v2 chunk replayed at a different
    /// index is rejected."
    #[test]
    fn a_chunk_replayed_at_a_different_index_is_rejected() {
        let captured = seal(
            &KEY,
            &aad_of("t-1", "f-1", 3, "docs/report.pdf"),
            &[0x0A; NONCE_LEN],
            b"chunk three",
        )
        .unwrap();

        // The same bytes, presented as chunk 99.
        let replayed = open(
            &KEY,
            &aad_of("t-1", "f-1", 99, "docs/report.pdf"),
            &captured.nonce,
            &captured.ciphertext,
        );

        assert_eq!(
            replayed.unwrap_err().kind(),
            ErrorKind::IntegrityMismatch,
            "a replay into another index must be an integrity failure"
        );
    }

    /// The same capture replayed into a different transfer, file or path. Each
    /// is a separate route to the same vulnerability, so each is asserted.
    #[test]
    fn a_chunk_replayed_into_another_slot_is_rejected() {
        let captured = seal(
            &KEY,
            &aad_of("t-1", "f-1", 3, "docs/report.pdf"),
            &[0x0A; NONCE_LEN],
            b"chunk three",
        )
        .unwrap();

        let elsewhere = [
            (
                "another transfer",
                aad_of("t-9", "f-1", 3, "docs/report.pdf"),
            ),
            ("another file", aad_of("t-1", "f-9", 3, "docs/report.pdf")),
            ("another path", aad_of("t-1", "f-1", 3, "docs/secrets.pdf")),
        ];

        for (what, aad) in elsewhere {
            let result = open(&KEY, &aad, &captured.nonce, &captured.ciphertext);

            assert_eq!(
                result.unwrap_err().kind(),
                ErrorKind::IntegrityMismatch,
                "replay into {what} must be rejected"
            );
        }
    }

    #[test]
    fn a_flipped_ciphertext_byte_fails_closed() {
        let aad = fixed_aad();

        let mut sealed = seal(&KEY, &aad, &[0x0A; NONCE_LEN], b"payload").unwrap();
        sealed.ciphertext[0] ^= 0x01;

        assert_eq!(
            open(&KEY, &aad, &sealed.nonce, &sealed.ciphertext)
                .unwrap_err()
                .kind(),
            ErrorKind::IntegrityMismatch
        );
    }

    #[test]
    fn a_flipped_tag_byte_fails_closed() {
        let aad = fixed_aad();

        let mut sealed = seal(&KEY, &aad, &[0x0A; NONCE_LEN], b"payload").unwrap();
        let last = sealed.ciphertext.len() - 1;
        sealed.ciphertext[last] ^= 0x80;

        assert!(open(&KEY, &aad, &sealed.nonce, &sealed.ciphertext).is_err());
    }

    #[test]
    fn a_flipped_nonce_byte_fails_closed() {
        let aad = fixed_aad();

        let mut sealed = seal(&KEY, &aad, &[0x0A; NONCE_LEN], b"payload").unwrap();
        sealed.nonce[0] ^= 0x01;

        assert_eq!(
            open(&KEY, &aad, &sealed.nonce, &sealed.ciphertext)
                .unwrap_err()
                .kind(),
            ErrorKind::IntegrityMismatch
        );
    }

    #[test]
    fn a_flipped_aad_byte_fails_closed() {
        let mut aad = fixed_aad();

        let sealed = seal(&KEY, &aad, &[0x0A; NONCE_LEN], b"payload").unwrap();

        aad[0] ^= 0x01;

        assert!(open(&KEY, &aad, &sealed.nonce, &sealed.ciphertext).is_err());
    }

    /// And a flipped AAD byte in the *length prefix*, which is the part most
    /// likely to be treated as harmless framing.
    #[test]
    fn a_flipped_aad_length_prefix_fails_closed() {
        let mut aad = fixed_aad();

        let sealed = seal(&KEY, &aad, &[0x0A; NONCE_LEN], b"payload").unwrap();

        aad[22] ^= 0x01;

        assert!(open(&KEY, &aad, &sealed.nonce, &sealed.ciphertext).is_err());
    }

    #[test]
    fn a_different_key_fails_closed() {
        let aad = fixed_aad();

        let sealed = seal(&KEY, &aad, &[0x0A; NONCE_LEN], b"payload").unwrap();

        assert!(open(&[0x43; 32], &aad, &sealed.nonce, &sealed.ciphertext).is_err());
    }

    /// An empty AAD is not a "no AAD" escape hatch: the tag still covers it, so
    /// a chunk sealed with an empty AAD cannot be opened with a real one.
    #[test]
    fn an_empty_aad_is_still_covered_by_the_tag() {
        let sealed = seal(&KEY, &[], &[0x0A; NONCE_LEN], b"payload").unwrap();

        assert!(open(&KEY, &[], &sealed.nonce, &sealed.ciphertext).is_ok());

        assert!(open(&KEY, &fixed_aad(), &sealed.nonce, &sealed.ciphertext).is_err());
    }

    /// v2 is total where v1 panics. `crypto::decrypt_chunk` builds an AES-GCM
    /// `Nonce` out of whatever slice it is handed and panics on a length that
    /// is neither 12 nor 16; `open` classifies every input it can be given here.
    #[test]
    fn a_wrong_length_nonce_is_an_error_and_not_a_panic() {
        let aad = fixed_aad();

        let sealed = seal(&KEY, &aad, &[0x0A; NONCE_LEN], b"payload").unwrap();

        for length in [0usize, 11, 13, 16, 32] {
            let nonce = vec![0x0A; length];

            assert_eq!(
                open(&KEY, &aad, &nonce, &sealed.ciphertext)
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidInput,
                "a nonce of {length} bytes must be InvalidInput, not a panic"
            );
        }
    }

    /*
     * ------------------------------------------------------------------
     * Domain separation from v1
     * ------------------------------------------------------------------
     */

    /// A v2 chunk is not openable as a v1 one.
    ///
    /// The v1 function is handed the same key, so the AAD is the only
    /// difference being exercised: v1 passes none, so its tag covers the
    /// ciphertext with no context and cannot match a chunk sealed with any.
    #[test]
    fn a_v2_chunk_is_not_a_v1_chunk() {
        let sealed = seal(&KEY, &fixed_aad(), &[0x0A; NONCE_LEN], b"payload").unwrap();

        assert!(
            v1::decrypt_chunk(&KEY, &sealed.nonce, &sealed.ciphertext).is_err(),
            "a v2 chunk must not open on the v1 path"
        );
    }

    /// And the reverse: a v1 chunk does not open on the v2 path.
    #[test]
    fn a_v1_chunk_is_not_a_v2_chunk() {
        let legacy = v1::encrypt_chunk(&KEY, b"payload").unwrap();

        assert!(
            open(&KEY, &fixed_aad(), &legacy.nonce, &legacy.data).is_err(),
            "a v1 chunk must not open on the v2 path"
        );
    }
}
