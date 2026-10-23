//! The ports this crate defines and the adapters implement.
//!
//! A port is an interface the *domain* owns. The adapter that satisfies it
//! lives in another crate and depends on this one — never the other way round
//! (ADR-0002 rule 2). That is what lets the desktop shell, and later the CLI,
//! the SDK and the mobile shells, share one definition of what a transport *is*
//! without sharing an implementation of one.
//!
//! The types are re-exported at the crate root, so the public path is
//! `vilsend_core::Transport`. The `ports` module exists to group them, not to
//! lengthen the path a caller writes.
//!
//! Deliberately *not* moved in here: [`crate::event`]. The event port predates
//! this module, and moving it would change the module path of a type the SDK's
//! committed public-API snapshot names — churn with no behaviour behind it.

pub mod transport;
