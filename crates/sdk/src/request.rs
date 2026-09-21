//! The request types the two transfer verbs take.
//!
//! **Requests are data, not closures** (`04-sdk-cli-mobile-build-plan.md` §2.2):
//! every one of them derives `Serialize`/`Deserialize` so a binding can build
//! one in TypeScript, Swift or Kotlin and hand it across the FFI boundary
//! without a callback or a closure ever having to cross.
//!
//! Every field is public and every type is plain data. There is no builder for
//! a request — a struct literal is already the honest spelling of "here is what
//! I want", and a second builder for the builder would be ceremony.

use serde::{Deserialize, Serialize};

use vilsend_core::PeerRef;

use crate::ids::{Destination, FileRef};

/// The tunables a transfer runs under.
///
/// `Default` is the product's default, not an arbitrary one: the values match
/// `crates/desktop/src/transfer/constants.rs`, so a caller who does not care
/// gets what the shipped app does.
///
/// **Deliberately not `#[non_exhaustive`]**, unlike every public enum here.
/// `04-sdk-cli-mobile-build-plan.md` §2.5 asks for non-exhaustive *enums*; a
/// struct a caller has to fill in is a different thing, and marking one
/// non-exhaustive forbids the `..Policy::default()` update syntax that is the
/// whole reason the `Default` impl is useful. The cost is that adding a field
/// here is a breaking change, which is the right trade for a configuration
/// value: a new field is a decision for the caller anyway.
///
/// [`non_exhaustive`]: https://doc.rust-lang.org/reference/attributes/type_system.html
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    /// How many bytes are read, hashed and handed to the transport at a time.
    pub chunk_size: usize,
    /// How many chunks may be in flight at once.
    pub concurrency: u8,
    /// How many times a chunk that failed may be retried before the transfer
    /// fails. `0` means one attempt.
    pub max_retries: u32,
    /// Whether the receiving side verifies each chunk against the hash the
    /// sending side computed for it.
    ///
    /// Present so it can be *turned off* for a caller that has integrity at
    /// another layer, and so that the check being on is visible rather than
    /// implied. It is on by default and the in-memory backend's tests assert
    /// the failure path it guards.
    pub verify_chunks: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            chunk_size: 4 * 1024 * 1024,
            concurrency: 4,
            max_retries: 4,
            verify_chunks: true,
        }
    }
}

impl Policy {
    /// This policy with the request's explicit overrides applied on top.
    ///
    /// **Precedence, most specific first:** a field set directly on the
    /// request, then the request's `policy`, then the builder's policy.
    /// `04-sdk-cli-mobile-build-plan.md` §2.2 puts both the individual fields
    /// and a whole `Policy` on `SendRequest` without saying which wins; a field
    /// named on the request is the narrower statement, so it wins.
    pub(crate) fn merged_with(
        &self,
        chunk_size: Option<usize>,
        concurrency: Option<u8>,
        max_retries: Option<u32>,
        policy: Option<&Policy>,
    ) -> Policy {
        let base = policy.unwrap_or(self);

        Policy {
            chunk_size: chunk_size.unwrap_or(base.chunk_size),
            concurrency: concurrency.unwrap_or(base.concurrency),
            max_retries: max_retries.unwrap_or(base.max_retries),
            verify_chunks: base.verify_chunks,
        }
    }

    /// Rejects a policy the engine cannot honour.
    ///
    /// A zero chunk size is not a small chunk, it is a loop that never
    /// advances; a zero concurrency is a transfer that never starts. Both are
    /// [`vilsend_core::ErrorKind::InvalidInput`] rather than a panic, because
    /// the caller is another program.
    pub(crate) fn validate(&self) -> Result<(), vilsend_core::VilsendError> {
        if self.chunk_size == 0 {
            return Err(vilsend_core::VilsendError::InvalidInput(
                "policy.chunk_size must be at least 1".into(),
            ));
        }

        if self.concurrency == 0 {
            return Err(vilsend_core::VilsendError::InvalidInput(
                "policy.concurrency must be at least 1".into(),
            ));
        }

        Ok(())
    }
}

/// What to send, and to whom.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendRequest {
    /// Who receives it, as this side refers to them.
    pub peer: PeerRef,
    /// Opaque handles, resolved by the source the builder was given.
    pub files: Vec<FileRef>,
    /// Overrides the policy's chunk size. See [`Policy::merged_with`].
    pub chunk_size: Option<usize>,
    /// Overrides the policy's concurrency.
    pub concurrency: Option<u8>,
    /// Overrides the policy's retry budget.
    pub max_retries: Option<u32>,
    /// Per-transfer override of the builder's policy.
    pub policy: Option<Policy>,
}

impl SendRequest {
    /// A request for `files` to `peer`, with the builder's policy.
    pub fn to(peer: PeerRef, files: Vec<FileRef>) -> Self {
        Self {
            peer,
            files,
            chunk_size: None,
            concurrency: None,
            max_retries: None,
            policy: None,
        }
    }

    /// The effective policy for this request. See [`Policy::merged_with`].
    pub(crate) fn effective_policy(
        &self,
        default: &Policy,
    ) -> Result<Policy, vilsend_core::VilsendError> {
        let merged = default.merged_with(
            self.chunk_size,
            self.concurrency,
            self.max_retries,
            self.policy.as_ref(),
        );

        merged.validate()?;

        Ok(merged)
    }
}

/// What to receive, and where to put it.
///
/// `04-sdk-cli-mobile-build-plan.md` sketches `SendRequest` but never defines
/// this one, so its shape is a Phase 5 decision: it carries the same three
/// pieces of information the sending side does — who, where, and under what
/// policy — and nothing else.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveRequest {
    /// Who the bytes come from, as this side refers to them.
    pub peer: PeerRef,
    /// Where the received files are written, resolved by the sink the builder
    /// was given.
    pub destination: Destination,
    /// Per-transfer override of the builder's policy.
    pub policy: Option<Policy>,
}

impl ReceiveRequest {
    /// A request to receive from `peer` into `destination`.
    pub fn from(peer: PeerRef, destination: Destination) -> Self {
        Self {
            peer,
            destination,
            policy: None,
        }
    }

    pub(crate) fn effective_policy(
        &self,
        default: &Policy,
    ) -> Result<Policy, vilsend_core::VilsendError> {
        let merged = default.merged_with(None, None, None, self.policy.as_ref());

        merged.validate()?;

        Ok(merged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> PeerRef {
        PeerRef::from("memory")
    }

    #[test]
    fn the_default_policy_is_the_one_the_shipped_app_runs() {
        // Pinned by value so that "the default changed" is a deliberate commit
        // rather than a side effect of editing `Default`.
        let policy = Policy::default();

        assert_eq!(policy.chunk_size, 4 * 1024 * 1024);
        assert_eq!(policy.concurrency, 4);
        assert_eq!(policy.max_retries, 4);
        assert!(policy.verify_chunks);
    }

    #[test]
    fn a_field_named_on_the_request_beats_the_requests_policy() {
        let with_everything = Policy {
            chunk_size: 10,
            concurrency: 1,
            max_retries: 0,
            verify_chunks: false,
        };

        let merged = Policy::default().merged_with(Some(64), None, None, Some(&with_everything));

        assert_eq!(merged.chunk_size, 64, "the request field is narrower");
        assert_eq!(merged.concurrency, 1, "from the request's policy");
        assert_eq!(merged.max_retries, 0, "from the request's policy");
        assert!(!merged.verify_chunks, "from the request's policy");
    }

    #[test]
    fn a_requests_policy_beats_the_builders_policy() {
        let builder = Policy {
            chunk_size: 1,
            concurrency: 1,
            max_retries: 1,
            verify_chunks: true,
        };
        let request = Policy {
            chunk_size: 2,
            concurrency: 2,
            max_retries: 2,
            verify_chunks: false,
        };

        let merged = builder.merged_with(None, None, None, Some(&request));

        assert_eq!(merged.chunk_size, 2);
        assert_eq!(merged.max_retries, 2);
    }

    #[test]
    fn the_builders_policy_is_the_floor() {
        let builder = Policy {
            chunk_size: 7,
            concurrency: 3,
            max_retries: 2,
            verify_chunks: false,
        };

        assert_eq!(builder.merged_with(None, None, None, None), builder);
    }

    #[test]
    fn a_request_with_no_policy_at_all_resolves_to_the_default() {
        let request = SendRequest::to(peer(), vec![FileRef::from("a")]);

        assert_eq!(
            request.effective_policy(&Policy::default()).expect("valid"),
            Policy::default()
        );
    }

    #[test]
    fn a_zero_chunk_size_is_rejected_rather_than_accepted_as_a_loop() {
        let request = SendRequest {
            chunk_size: Some(0),
            ..SendRequest::to(peer(), vec![FileRef::from("a")])
        };

        let error = request
            .effective_policy(&Policy::default())
            .expect_err("a zero chunk size cannot be honoured");

        assert_eq!(error.kind(), vilsend_core::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_zero_concurrency_is_rejected() {
        let request = ReceiveRequest {
            policy: Some(Policy {
                concurrency: 0,
                ..Policy::default()
            }),
            ..ReceiveRequest::from(peer(), Destination::root())
        };

        let error = request
            .effective_policy(&Policy::default())
            .expect_err("a zero concurrency cannot be honoured");

        assert_eq!(error.kind(), vilsend_core::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_send_request_round_trips_through_json() {
        // The FFI promise of §2.2: a request built in another language arrives
        // intact, including the fields it left unset.
        let request = SendRequest {
            chunk_size: Some(1024),
            ..SendRequest::to(peer(), vec![FileRef::from("a.txt"), FileRef::from("b.txt")])
        };

        let json = serde_json::to_string(&request).expect("serialisable");

        assert_eq!(
            serde_json::from_str::<SendRequest>(&json).expect("deserialisable"),
            request
        );
    }

    #[test]
    fn a_receive_request_round_trips_through_json() {
        let request = ReceiveRequest::from(peer(), Destination::named("downloads"));

        let json = serde_json::to_string(&request).expect("serialisable");

        assert_eq!(
            serde_json::from_str::<ReceiveRequest>(&json).expect("deserialisable"),
            request
        );
    }
}
