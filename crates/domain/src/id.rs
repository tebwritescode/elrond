//! Strongly typed identifiers.
//!
//! Every entity gets its own newtype so a `DocumentId` can never be passed
//! where a `UserId` is expected. UUIDv7 is used because it sorts by creation
//! time, which keeps SQLite index locality reasonable as the library grows.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Declares a UUIDv7-backed identifier newtype.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Generates a fresh time-ordered identifier.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Wraps an existing UUID, for rehydrating from storage.
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Borrows the underlying UUID.
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// Copies out the underlying UUID.
            pub const fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(value)?))
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }
    };
}

define_id!(
    /// Identifies a local account.
    UserId
);
define_id!(
    /// Identifies a logical document, independent of its versions.
    DocumentId
);
define_id!(
    /// Identifies one immutable version of a document.
    DocumentVersionId
);
define_id!(
    /// Identifies a node in the hierarchical category tree.
    CategoryId
);
define_id!(
    /// Identifies a server-side session record.
    ///
    /// This is the database key, never the bearer token. The token itself is
    /// opaque random material that is only ever stored hashed.
    SessionId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_time_ordered() {
        let first = UserId::new();
        let second = UserId::new();
        assert!(first < second, "UUIDv7 must sort by creation order");
    }

    #[test]
    fn ids_round_trip_through_strings() {
        let id = DocumentId::new();
        let parsed: DocumentId = id.to_string().parse().expect("valid uuid");
        assert_eq!(id, parsed);
    }

    #[test]
    fn distinct_ids_are_generated() {
        let a = CategoryId::new();
        let b = CategoryId::new();
        assert_ne!(a, b);
    }
}
