//! The third verb.
//!
//! `04-sdk-cli-mobile-build-plan.md` §2.2 gives `Vilsend` four verbs and this
//! is one of them: `pub async fn auth(&self) -> AuthFacade`. It is deliberately
//! the thinnest of the four, for a reason that is worth stating in the source
//! rather than in a report.
//!
//! **Phase 6 owns authentication.** It introduces `AuthProvider` and
//! `CredentialStore`, moves issuer discovery, and adds revocation — in that
//! phase's words, "zero user-visible change". Phase 5 cannot implement any of
//! it: there is no provider to report on, and inventing a token lifecycle here
//! would give Phase 6 two things to reconcile instead of one thing to build.
//!
//! What Phase 5 *can* do is fix the verb's shape, so that a shell written
//! against `auth().state()` does not have to change when the provider arrives.
//! That is all this is.

use serde::{Deserialize, Serialize};

/// Whether this client has a usable session.
///
/// `#[non_exhaustive]`, like every other public enum in this crate: Phase 6
/// adds at least "signed in, refresh in flight" and "signed in, refresh
/// failed", and adding a variant must not be a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AuthState {
    /// There is no session. The caller must sign in before a transfer that
    /// needs one can run.
    SignedOut,
    /// There is a session.
    SignedIn,
}

/// What `Vilsend::auth()` hands back.
///
/// A facade rather than a set of methods on `Vilsend`, so that the auth surface
/// can grow — `login`, `logout`, `on_state_changed` — without widening the
/// object that also owns `send` and `receive`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthFacade {
    state: AuthState,
}

impl AuthFacade {
    pub(crate) fn new(state: AuthState) -> Self {
        Self { state }
    }

    /// The current state.
    ///
    /// Synchronous because it reports a decision that has already been made;
    /// nothing here goes to the network.
    pub fn state(&self) -> AuthState {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_facade_reports_the_state_it_was_built_with() {
        assert_eq!(
            AuthFacade::new(AuthState::SignedIn).state(),
            AuthState::SignedIn
        );
        assert_eq!(
            AuthFacade::new(AuthState::SignedOut).state(),
            AuthState::SignedOut
        );
    }

    #[test]
    fn the_two_states_are_distinct_and_serialise_in_the_wire_casing() {
        assert_ne!(AuthState::SignedIn, AuthState::SignedOut);

        assert_eq!(
            serde_json::to_string(&AuthState::SignedOut).expect("serialisable"),
            "\"SIGNED_OUT\""
        );
        assert_eq!(
            serde_json::to_string(&AuthState::SignedIn).expect("serialisable"),
            "\"SIGNED_IN\""
        );
    }
}
