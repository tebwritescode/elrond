//! Local accounts, roles, and credential policy.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::DomainError;
use crate::id::UserId;

/// Maximum stored length of a display name.
const DISPLAY_NAME_MAX: usize = 120;
/// Maximum stored length of an email address, matching the SMTP path limit.
const EMAIL_MAX: usize = 254;

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

/// A validated, normalized email address.
///
/// Validation is deliberately structural rather than exhaustive: the goal is to
/// reject obvious mistakes and guarantee a canonical lookup key, not to
/// reimplement RFC 5322.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct EmailAddress(String);

impl EmailAddress {
    /// Validates and normalizes an address.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::Required { field: "email" });
        }
        if trimmed.chars().count() > EMAIL_MAX {
            return Err(DomainError::TooLong {
                field: "email",
                max: EMAIL_MAX,
            });
        }
        if trimmed.chars().any(char::is_whitespace) {
            return Err(DomainError::Invalid {
                field: "email",
                reason: "contains_whitespace",
            });
        }

        let mut parts = trimmed.split('@');
        let local = parts.next().unwrap_or_default();
        let domain = parts.next().ok_or(DomainError::Invalid {
            field: "email",
            reason: "missing_at_sign",
        })?;
        if parts.next().is_some() {
            return Err(DomainError::Invalid {
                field: "email",
                reason: "multiple_at_signs",
            });
        }
        if local.is_empty() {
            return Err(DomainError::Invalid {
                field: "email",
                reason: "empty_local_part",
            });
        }
        if domain.is_empty() || !domain.contains('.') {
            return Err(DomainError::Invalid {
                field: "email",
                reason: "domain_must_contain_a_dot",
            });
        }
        if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
            return Err(DomainError::Invalid {
                field: "email",
                reason: "malformed_domain",
            });
        }

        // Only the domain is case-insensitive per spec, but Elrond lowercases the
        // whole address so a single account cannot be created twice with
        // different casing.
        Ok(Self(trimmed.to_lowercase()))
    }

    /// Borrows the normalized address.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for EmailAddress {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// A validated human-readable account name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct DisplayName(String);

impl DisplayName {
    /// Validates and normalizes a display name.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainError::Required {
                field: "display_name",
            });
        }
        if trimmed.chars().count() > DISPLAY_NAME_MAX {
            return Err(DomainError::TooLong {
                field: "display_name",
                max: DISPLAY_NAME_MAX,
            });
        }
        // Control characters would let a name corrupt logs, CSV exports, and
        // generated binder covers.
        if trimmed.chars().any(char::is_control) {
            return Err(DomainError::Invalid {
                field: "display_name",
                reason: "contains_control_characters",
            });
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Borrows the name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DisplayName {
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
    /// Normalized login address.
    pub email: EmailAddress,
    /// Name shown in the interface and audit trail.
    pub display_name: DisplayName,
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
    fn email_is_lowercased_and_trimmed() {
        let email = EmailAddress::parse("  Editor@Example.ORG  ").expect("valid");
        assert_eq!(email.as_str(), "editor@example.org");
    }

    #[test]
    fn malformed_emails_are_rejected() {
        for candidate in [
            "",
            "   ",
            "no-at-sign",
            "@example.org",
            "user@",
            "user@localhost",
            "user@.example.org",
            "user@example.org.",
            "user@exa..mple.org",
            "two@at@example.org",
            "spaced user@example.org",
        ] {
            assert!(
                EmailAddress::parse(candidate).is_err(),
                "expected rejection for {candidate:?}"
            );
        }
    }

    #[test]
    fn overlong_email_is_rejected() {
        let candidate = format!("{}@example.org", "a".repeat(EMAIL_MAX));
        let error = EmailAddress::parse(&candidate).expect_err("too long");
        assert_eq!(error.code(), "field_too_long");
    }

    #[test]
    fn display_name_rejects_blank_and_control_characters() {
        assert!(DisplayName::parse("   ").is_err());
        assert!(DisplayName::parse("Records\u{0007}Team").is_err());
        assert_eq!(
            DisplayName::parse("  Records Team  ").expect("valid").as_str(),
            "Records Team"
        );
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
            email: EmailAddress::parse("archivist@example.org").expect("valid"),
            display_name: DisplayName::parse("Archivist").expect("valid"),
            role: Role::Editor,
            is_active: false,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };
        assert!(!user.can_authenticate());
    }
}
