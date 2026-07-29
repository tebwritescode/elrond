//! Explicit domain errors.
//!
//! Variants are enumerated rather than collapsed into a string so the API layer
//! can map each one onto a stable HTTP contract, and so a new failure mode
//! cannot silently inherit an unrelated status code.

use thiserror::Error;

/// A violated domain rule.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    /// A required value was absent or contained only whitespace.
    #[error("{field} is required")]
    Required {
        /// Field name, safe to surface to clients.
        field: &'static str,
    },

    /// A value exceeded its maximum length.
    #[error("{field} must be at most {max} characters")]
    TooLong {
        /// Field name, safe to surface to clients.
        field: &'static str,
        /// Inclusive maximum length in characters.
        max: usize,
    },

    /// A value fell short of its minimum length.
    #[error("{field} must be at least {min} characters")]
    TooShort {
        /// Field name, safe to surface to clients.
        field: &'static str,
        /// Inclusive minimum length in characters.
        min: usize,
    },

    /// A value was structurally invalid.
    ///
    /// The message never embeds the rejected value, so validation failures can
    /// be logged without leaking credentials or document contents.
    #[error("{field} is not valid: {reason}")]
    Invalid {
        /// Field name, safe to surface to clients.
        field: &'static str,
        /// Machine-stable reason code.
        reason: &'static str,
    },

    /// A lifecycle transition was not permitted.
    #[error(transparent)]
    Lifecycle(#[from] crate::document::LifecycleTransitionError),
}

impl DomainError {
    /// Returns a stable, machine-readable code for this error.
    ///
    /// Clients branch on this rather than on the human-readable message.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Required { .. } => "field_required",
            Self::TooLong { .. } => "field_too_long",
            Self::TooShort { .. } => "field_too_short",
            Self::Invalid { .. } => "field_invalid",
            Self::Lifecycle(_) => "lifecycle_transition_forbidden",
        }
    }

    /// Returns the offending field name where one applies.
    pub const fn field(&self) -> Option<&'static str> {
        match self {
            Self::Required { field }
            | Self::TooLong { field, .. }
            | Self::TooShort { field, .. }
            | Self::Invalid { field, .. } => Some(field),
            Self::Lifecycle(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_distinct_per_variant() {
        let codes = [
            DomainError::Required { field: "title" }.code(),
            DomainError::TooLong {
                field: "title",
                max: 1,
            }
            .code(),
            DomainError::TooShort {
                field: "title",
                min: 1,
            }
            .code(),
            DomainError::Invalid {
                field: "title",
                reason: "shape",
            }
            .code(),
        ];
        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len());
    }

    #[test]
    fn invalid_message_omits_the_rejected_value() {
        let error = DomainError::Invalid {
            field: "email",
            reason: "missing_at_sign",
        };
        let rendered = error.to_string();
        assert!(rendered.contains("email"));
        assert!(rendered.contains("missing_at_sign"));
    }
}
