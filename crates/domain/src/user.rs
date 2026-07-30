//! Local accounts, roles, and credential policy.
//!
//! Authentication is deliberately minimal: a username and a password. There is no
//! email address anywhere in the model, so Elrond stores no contact details, has
//! nothing to verify, and needs no mail transport to be useful.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::DomainError;
use crate::id::UserId;

/// Shortest accepted username.
const USERNAME_MIN: usize = 3;
/// Longest accepted username.
const USERNAME_MAX: usize = 32;

/// What an account is allowed to do.
///
/// Roles are ordered from least to most privileged so comparisons express
/// "at least this much authority" directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Read-only access to published material.
    Viewer,
    /// Viewer, plus the ability to approve or reject documents in review.
    Reviewer,
    /// Reviewer, plus ingestion, editing, and binder authoring.
    Editor,
    /// Full control, including account and system administration.
    Admin,
}

impl Role {
    /// Every role, least privileged first.
    pub const ALL: [Self; 4] = [Self::Viewer, Self::Reviewer, Self::Editor, Self::Admin];

    /// Stable wire representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Reviewer => "reviewer",
            Self::Editor => "editor",
            Self::Admin => "admin",
        }
    }

    /// Returns true when this role carries at least `required` authority.
    pub fn satisfies(self, required: Self) -> bool {
        self >= required
    }

    /// Whether the role may create, replace, or delete documents.
    pub fn can_write_documents(self) -> bool {
        self.satisfies(Self::Editor)
    }

    /// Whether the role may move documents through the review workflow.
    pub fn can_review(self) -> bool {
        self.satisfies(Self::Reviewer)
    }

    /// Whether the role may administer accounts and system settings.
    pub fn can_administer(self) -> bool {
        self.satisfies(Self::Admin)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Role {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "viewer" => Ok(Self::Viewer),
            "reviewer" => Ok(Self::Reviewer),
            "editor" => Ok(Self::Editor),
            "admin" => Ok(Self::Admin),
            _ => Err(DomainError::Invalid {
                field: "role",
                reason: "unknown_role",
            }),
        }
    }
}

/// A validated, normalized login name.
///
/// Normalizing to lowercase is what makes the uniqueness guarantee real: without
/// it, `Archivist` and `archivist` would be two accounts that look identical in
/// every list and audit record.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Username(String);

impl Username {
    /// Characters permitted inside a username, besides letters and digits.
    const SEPARATORS: [char; 3] = ['.', '_', '-'];

    /// Validates and normalizes a username.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::Required { field: "username" });
        }

        let normalized = trimmed.to_lowercase();
        let length = normalized.chars().count();
        if length < USERNAME_MIN {
            return Err(DomainError::TooShort {
                field: "username",
                min: USERNAME_MIN,
            });
        }
        if length > USERNAME_MAX {
            return Err(DomainError::TooLong {
                field: "username",
                max: USERNAME_MAX,
            });
        }

        // ASCII only. A username is an identifier, and allowing Unicode would
        // admit homoglyph pairs that render identically but compare unequal,
        // which is an impersonation risk in an audit trail.
        if !normalized
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || Self::SEPARATORS.contains(&c))
        {
            return Err(DomainError::Invalid {
                field: "username",
                reason: "letters_digits_dot_underscore_hyphen_only",
            });
        }

        let first = normalized.chars().next().unwrap_or_default();
        let last = normalized.chars().last().unwrap_or_default();
        if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
            return Err(DomainError::Invalid {
                field: "username",
                reason: "must_start_and_end_with_a_letter_or_digit",
            });
        }

        Ok(Self(normalized))
    }

    /// Borrows the normalized username.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Username {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Credential rules applied before a password reaches the hasher.
pub struct PasswordPolicy;

impl PasswordPolicy {
    /// Shortest accepted passphrase.
    ///
    /// Length is the only hard requirement. Composition rules push users toward
    /// predictable substitutions, and NIST SP 800-63B advises against them.
    pub const MIN_LENGTH: usize = 12;

    /// Longest accepted passphrase.
    ///
    /// Argon2id cost scales with input size, so an unbounded password field is a
    /// denial-of-service vector. The cap is far above any realistic passphrase.
    pub const MAX_LENGTH: usize = 1024;

    /// Validates a raw password without retaining or logging it.
    pub fn validate(raw: &str) -> Result<(), DomainError> {
        let length = raw.chars().count();
        if length == 0 {
            return Err(DomainError::Required { field: "password" });
        }
        if length < Self::MIN_LENGTH {
            return Err(DomainError::TooShort {
                field: "password",
                min: Self::MIN_LENGTH,
            });
        }
        if length > Self::MAX_LENGTH {
            return Err(DomainError::TooLong {
                field: "password",
                max: Self::MAX_LENGTH,
            });
        }
        Ok(())
    }
}

/// A local account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    /// Stable identifier.
    pub id: UserId,
    /// Normalized login name, also what the interface displays.
    pub username: Username,
    /// Authority level.
    pub role: Role,
    /// Deactivated accounts keep their audit history but cannot authenticate.
    pub is_active: bool,
    /// Creation timestamp in UTC.
    pub created_at: OffsetDateTime,
    /// Last modification timestamp in UTC.
    pub updated_at: OffsetDateTime,
}

impl User {
    /// Returns true when the account is permitted to start a session.
    pub fn can_authenticate(&self) -> bool {
        self.is_active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_are_ordered_by_authority() {
        assert!(Role::Admin > Role::Editor);
        assert!(Role::Editor > Role::Reviewer);
        assert!(Role::Reviewer > Role::Viewer);
    }

    #[test]
    fn capability_checks_follow_the_role_ladder() {
        assert!(Role::Admin.can_administer());
        assert!(!Role::Editor.can_administer());
        assert!(Role::Editor.can_write_documents());
        assert!(!Role::Reviewer.can_write_documents());
        assert!(Role::Reviewer.can_review());
        assert!(!Role::Viewer.can_review());
    }

    #[test]
    fn roles_round_trip_through_their_wire_form() {
        for role in Role::ALL {
            assert_eq!(role.as_str().parse::<Role>().expect("known role"), role);
        }
    }

    #[test]
    fn unknown_role_is_rejected() {
        assert!("superuser".parse::<Role>().is_err());
    }

    #[test]
    fn usernames_are_trimmed_and_lowercased() {
        let username = Username::parse("  Records.Admin  ").expect("valid");
        assert_eq!(username.as_str(), "records.admin");
    }

    #[test]
    fn case_variants_normalize_to_the_same_account() {
        assert_eq!(
            Username::parse("Archivist").expect("valid"),
            Username::parse("archivist").expect("valid")
        );
    }

    #[test]
    fn separators_are_allowed_inside_a_username() {
        for candidate in ["a_b", "a.b", "a-b", "user.name_1", "abc"] {
            assert!(
                Username::parse(candidate).is_ok(),
                "expected {candidate:?} to be accepted"
            );
        }
    }

    #[test]
    fn malformed_usernames_are_rejected() {
        for candidate in [
            "",          // empty
            "   ",       // whitespace only
            "ab",        // too short
            "_leading",  // starts with a separator
            "trailing-", // ends with a separator
            "has space", // whitespace inside
            "user@host", // an address, not a username
            "user name", // whitespace inside
            "tab\tname", // control character
            "аdmin",     // Cyrillic 'а', a homoglyph for ASCII 'a'
        ] {
            assert!(
                Username::parse(candidate).is_err(),
                "expected rejection for {candidate:?}"
            );
        }
    }

    #[test]
    fn overlong_username_is_rejected() {
        let error = Username::parse(&"a".repeat(USERNAME_MAX + 1)).expect_err("too long");
        assert_eq!(error.code(), "field_too_long");
        assert!(Username::parse(&"a".repeat(USERNAME_MAX)).is_ok());
    }

    #[test]
    fn password_policy_enforces_length_bounds() {
        assert!(PasswordPolicy::validate("").is_err());
        assert!(PasswordPolicy::validate("short").is_err());
        assert!(PasswordPolicy::validate(&"a".repeat(PasswordPolicy::MIN_LENGTH)).is_ok());
        assert!(PasswordPolicy::validate(&"a".repeat(PasswordPolicy::MAX_LENGTH)).is_ok());
        assert!(PasswordPolicy::validate(&"a".repeat(PasswordPolicy::MAX_LENGTH + 1)).is_err());
    }

    #[test]
    fn deactivated_accounts_cannot_authenticate() {
        let user = User {
            id: UserId::new(),
            username: Username::parse("archivist").expect("valid"),
            role: Role::Editor,
            is_active: false,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };
        assert!(!user.can_authenticate());
    }
}
