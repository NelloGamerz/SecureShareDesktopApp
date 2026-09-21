//! The wire protocol version.
//!
//! ADR-0010, decision 5: the protocol version is a `u16` constant of its own,
//! **independent of the crate version.** They move at different rates — the
//! desktop app is at `1.0.4` and speaks wire protocol `1`, and nothing about
//! bumping the app to `1.0.5` may change a byte on the wire. Confusing the two
//! is how compatibility bugs happen, which is why the number lives here rather
//! than being derived from anything.
//!
//! # What "the version" means
//!
//! The v1 wire is `POST /transfer/start`, `POST /transfer/chunk` and
//! `GET /transfer/public-key`, with the AEAD and the KDF as they are in the
//! installed client. It is **frozen** — see ADR-0010 decision 1 — and this
//! repository never changes it, including the parts of it that are wrong.
//!
//! The v2 wire adds associated data to the AEAD, a salt to the KDF and a
//! repeat check to the nonce, and it is served from new paths so that a v1
//! receiver answers `404` rather than misparsing. Detection is by probing for
//! those paths, never by reading a version number off the wire (ADR-0010,
//! decision 3).
//!
//! # [`PROTOCOL_VERSION`] is the version this build *speaks by default*
//!
//! It is a compile-time choice gated on the `protocol-v2` feature, which is
//! **off** in a default build — so a default build of this crate reports
//! [`PROTOCOL_V1`], exactly as the installed base does. Turning the feature on
//! raises it to [`PROTOCOL_V2`] *for that build*; it does not change what the
//! build will do against a peer that only speaks v1, because that decision is
//! made by capability probing at session setup, not by this constant.
//!
//! Both numbers are always defined, so a v2-capable build can still name the
//! version it is falling back to.

/// The original wire format, shipped as v1.0.4 and frozen indefinitely
/// (ADR-0010, decision 1).
pub const PROTOCOL_V1: u16 = 1;

/// The hardened wire format: associated data on every chunk, a salted KDF, and
/// a bounded nonce-repeat window. Additive, served under new paths
/// (ADR-0010, decision 2).
pub const PROTOCOL_V2: u16 = 2;

/// The protocol version this build speaks by default.
///
/// [`PROTOCOL_V1`] unless the `protocol-v2` feature is enabled. The feature is
/// deliberately not in any crate's `default` list: every new wire behaviour
/// ships behind a flag that is off for the first release (cross-phase concern,
/// `05-migration-plan.md`).
#[cfg(feature = "protocol-v2")]
pub const PROTOCOL_VERSION: u16 = PROTOCOL_V2;

/// The protocol version this build speaks by default.
///
/// [`PROTOCOL_V1`] unless the `protocol-v2` feature is enabled.
#[cfg(not(feature = "protocol-v2"))]
pub const PROTOCOL_VERSION: u16 = PROTOCOL_V1;

#[cfg(test)]
mod tests {
    use super::*;

    /// The two version numbers are pinned by value, so that renumbering them is
    /// a deliberate act. `2` is not "the next integer" — it is what ADR-0010
    /// names, and a v1 peer that ever reads it must see `2` or nothing.
    ///
    /// It also pins that both constants exist in *every* configuration, which
    /// is what lets a v2 build name the version it falls back to.
    ///
    /// The table is a `const` rather than two `assert_eq!` calls because
    /// clippy's `assertions_on_constants` fires on a comparison of two named
    /// constants and asks for a const block — the loop is the honest spelling
    /// of "these are the values, check them".
    #[test]
    fn the_version_numbers_are_distinct_and_what_the_adr_names() {
        const NAMES: [(u16, u16); 2] = [(PROTOCOL_V1, 1), (PROTOCOL_V2, 2)];

        for (actual, expected) in NAMES {
            assert_eq!(actual, expected);
        }

        assert_ne!(PROTOCOL_V1, PROTOCOL_V2);
    }

    /// The gate is the whole point of the constant: a default build must claim
    /// v1, or every installed client breaks.
    #[cfg(not(feature = "protocol-v2"))]
    #[test]
    fn a_default_build_speaks_v1() {
        assert_eq!(PROTOCOL_VERSION, PROTOCOL_V1);
    }

    /// And enabling the feature is the only thing that moves it.
    #[cfg(feature = "protocol-v2")]
    #[test]
    fn a_protocol_v2_build_speaks_v2() {
        assert_eq!(PROTOCOL_VERSION, PROTOCOL_V2);
    }
}
