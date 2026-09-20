//! Identifier newtypes.
//!
//! These exist so a `TransferId` cannot be passed where a `DeviceId` is
//! expected. Nothing in the desktop shell uses them yet — Phase 2 is a pure
//! refactor and rewiring the call sites would be a behaviour-adjacent change
//! spread across the whole transfer module. The shell adopts them as it is
//! refactored in Phase 3.
//!
//! Every one of them serialises as a bare string, so adopting them does not
//! change a wire format.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

string_id! {
    /// Identifies one transfer on both sides of it.
    TransferId
}

string_id! {
    /// Identifies an installation of the app.
    DeviceId
}

string_id! {
    /// Identifies the other end of a transfer, as this side refers to it.
    PeerRef
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialise_as_bare_strings() {
        assert_eq!(
            serde_json::to_string(&TransferId::new("t-1")).unwrap(),
            "\"t-1\""
        );
        assert_eq!(
            serde_json::to_string(&DeviceId::new("d-1")).unwrap(),
            "\"d-1\""
        );
        assert_eq!(
            serde_json::to_string(&PeerRef::new("p-1")).unwrap(),
            "\"p-1\""
        );
    }

    #[test]
    fn ids_deserialise_from_bare_strings() {
        let id: TransferId = serde_json::from_str("\"t-1\"").unwrap();

        assert_eq!(id, TransferId::new("t-1"));
        assert_eq!(id.as_str(), "t-1");
        assert_eq!(id.to_string(), "t-1");
    }

    #[test]
    fn ids_convert_from_owned_and_borrowed_strings() {
        assert_eq!(TransferId::from("t-1"), TransferId::new("t-1"));
        assert_eq!(TransferId::from(String::from("t-1")).as_str(), "t-1");
        assert_eq!(DeviceId::from("d-1").as_str(), "d-1");
        assert_eq!(PeerRef::from("p-1").as_str(), "p-1");
    }
}
