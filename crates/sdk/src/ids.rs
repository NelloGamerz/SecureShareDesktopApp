//! Handles that name a party to a transfer without exposing what is behind it.
//!
//! [`FileRef`] and [`Destination`] are deliberately opaque strings, exactly as
//! `SendRequest.files` is in `04-sdk-cli-mobile-build-plan.md` §2.2: "opaque
//! handles resolved by the injected `ChunkSource`". The SDK never asks a caller
//! for a `PathBuf`, because a caller on iOS, in a browser or inside a sandboxed
//! host has no such thing — it has a handle its own storage layer understands.
//!
//! Both types live here rather than in `vilsend-core` because nothing outside
//! the SDK's request types has a use for them. If Phase 8's `Transport` port
//! needs to name a file, that is the point to move them.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A caller-supplied handle for one input file.
///
/// The string is meaningful only to whoever supplied the source. The SDK
/// passes it back untouched; it never parses it, and it never assumes it is a
/// path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileRef(String);

impl FileRef {
    /// Wraps a caller's handle.
    pub fn new(handle: impl Into<String>) -> Self {
        Self(handle.into())
    }

    /// The handle, verbatim.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FileRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for FileRef {
    fn from(handle: &str) -> Self {
        Self(handle.to_owned())
    }
}

impl From<String> for FileRef {
    fn from(handle: String) -> Self {
        Self(handle)
    }
}

/// Where a receiving transfer puts what it receives.
///
/// Also opaque, and also for the same reason: `vilsend receive --to ./downloads`
/// is a *shell's* spelling of a destination it constructs from its own
/// filesystem, not something the SDK may assume.
///
/// The one shape the SDK does require is that a destination can be empty —
/// [`Destination::root`] — because a host that has already decided where the
/// bytes go should not have to invent a name for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Destination(String);

impl Destination {
    /// The destination with no name of its own: the whole store.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// A destination with a name.
    pub fn named(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The name, verbatim. Empty for [`Destination::root`].
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `true` when this is [`Destination::root`].
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

impl Default for Destination {
    fn default() -> Self {
        Self::root()
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<&str> for Destination {
    fn from(name: &str) -> Self {
        Self(name.to_owned())
    }
}

impl From<String> for Destination {
    fn from(name: String) -> Self {
        Self(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_ref_round_trips_through_json_as_a_bare_string() {
        let reference = FileRef::new("docs/report.pdf");

        assert_eq!(
            serde_json::to_string(&reference).expect("serialisable"),
            "\"docs/report.pdf\""
        );
        assert_eq!(
            serde_json::from_str::<FileRef>("\"docs/report.pdf\"").expect("deserialisable"),
            reference
        );
    }

    #[test]
    fn a_destination_round_trips_through_json_as_a_bare_string() {
        let destination = Destination::named("downloads");

        assert_eq!(
            serde_json::to_string(&destination).expect("serialisable"),
            "\"downloads\""
        );
        assert_eq!(
            serde_json::from_str::<Destination>("\"downloads\"").expect("deserialisable"),
            destination
        );
    }

    #[test]
    fn the_root_destination_is_the_empty_one() {
        assert!(Destination::root().is_root());
        assert!(Destination::default().is_root());
        assert_eq!(Destination::root().as_str(), "");
        assert!(!Destination::named("x").is_root());
    }

    #[test]
    fn a_file_ref_keeps_the_handle_verbatim() {
        // The SDK must not normalise, parse or "fix" a handle it was given: it
        // is another component's identifier.
        let awkward = FileRef::new("../weird\\name:with*chars");

        assert_eq!(awkward.as_str(), "../weird\\name:with*chars");
        assert_eq!(awkward.to_string(), "../weird\\name:with*chars");
        assert_eq!(FileRef::from("a"), FileRef::new("a"));
        assert_eq!(FileRef::from(String::from("a")).as_str(), "a");
    }
}
