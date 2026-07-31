//! Application-level failures.

use elrond_domain::{DomainError, Role};
use thiserror::Error;

use crate::ports::{HashingError, RepositoryError};

/// Result alias for use cases.
pub type ApplicationResult<T> = Result<T, ApplicationError>;

/// Everything a use case can fail with.
///
/// Each variant maps onto exactly one HTTP contract in the API layer, so adding
/// a failure mode forces an explicit decision about how clients see it.
#[derive(Debug, Error)]
pub enum ApplicationError {
    /// Input violated a domain rule.
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// Storage failed.
    #[error(transparent)]
    Repository(#[from] RepositoryError),

    /// Content storage failed.
    #[error(transparent)]
    Storage(#[from] crate::ports::BlobError),

    /// Binder rendering failed.
    #[error(transparent)]
    Render(#[from] crate::ports::RenderError),

    /// An uploaded archive could not be extracted.
    #[error(transparent)]
    Archive(#[from] crate::ports::ArchiveError),

    /// Password hashing or verification failed for a reason unrelated to the
    /// supplied password being wrong.
    #[error(transparent)]
    Hashing(#[from] HashingError),

    /// The email/password pair did not match.
    ///
    /// Deliberately indistinguishable from "no such account" so the endpoint
    /// cannot be used to enumerate registered addresses.
    #[error("invalid credentials")]
    InvalidCredentials,

    /// Credentials were correct but the account is deactivated.
    #[error("this account is deactivated")]
    AccountDisabled,

    /// First-run setup was attempted on an already-initialized instance.
    #[error("this instance has already been set up")]
    SetupAlreadyCompleted,

    /// No session, or a session that has expired or been revoked.
    #[error("authentication required")]
    NotAuthenticated,

    /// Authenticated, but without sufficient authority.
    #[error("this action requires the {required} role")]
    Forbidden {
        /// Minimum role the action needs.
        required: Role,
    },

    /// The requested resource does not exist.
    #[error("{resource} not found")]
    NotFound {
        /// Resource kind, safe to surface to clients.
        resource: &'static str,
    },

    /// The request conflicts with current state.
    #[error("{resource} conflict: {reason}")]
    Conflict {
        /// Resource kind, safe to surface to clients.
        resource: &'static str,
        /// Machine-stable reason code.
        reason: &'static str,
    },
}

impl ApplicationError {
    /// Stable machine-readable code for clients to branch on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Domain(error) => error.code(),
            Self::Repository(RepositoryError::UniqueViolation { .. }) => "already_exists",
            // A database failure and a blob-store failure are the same thing from a
            // client's point of view: storage did not work, and the detail is in the
            // log rather than in the response.
            Self::Repository(RepositoryError::Backend(_))
            | Self::Storage(crate::ports::BlobError::Backend(_)) => "storage_failure",
            Self::Storage(crate::ports::BlobError::NotFound { .. }) => "content_not_found",
            Self::Storage(crate::ports::BlobError::IntegrityFailure { .. }) => {
                "content_integrity_failure"
            }
            Self::Storage(crate::ports::BlobError::TooLarge { .. }) => "content_too_large",
            Self::Render(crate::ports::RenderError::UnreadableSource { .. }) => {
                "binder_source_unreadable"
            }
            Self::Render(crate::ports::RenderError::EmptySource { .. }) => "binder_source_empty",
            Self::Render(crate::ports::RenderError::EmptyPlan) => "binder_empty",
            Self::Render(crate::ports::RenderError::Backend(_)) => "binder_render_failure",
            Self::Archive(crate::ports::ArchiveError::Unreadable) => "archive_unreadable",
            Self::Archive(crate::ports::ArchiveError::TooLarge { .. }) => "archive_too_large",
            Self::Archive(crate::ports::ArchiveError::TooManyEntries { .. }) => {
                "archive_too_many_entries"
            }
            Self::Archive(crate::ports::ArchiveError::Entry { .. }) => "archive_entry_unreadable",
            Self::Hashing(_) => "credential_processing_failure",
            Self::InvalidCredentials => "invalid_credentials",
            Self::AccountDisabled => "account_disabled",
            Self::SetupAlreadyCompleted => "setup_already_completed",
            Self::NotAuthenticated => "not_authenticated",
            Self::Forbidden { .. } => "forbidden",
            Self::NotFound { .. } => "not_found",
            Self::Conflict { .. } => "conflict",
        }
    }

    /// The offending field, when the failure is about one.
    pub const fn field(&self) -> Option<&'static str> {
        match self {
            Self::Domain(error) => error.field(),
            _ => None,
        }
    }

    /// Whether the failure is the caller's fault.
    ///
    /// Used to decide log severity: client errors are noise at `warn`, server
    /// errors deserve `error`.
    pub fn is_client_error(&self) -> bool {
        !matches!(
            self,
            Self::Repository(RepositoryError::Backend(_))
                | Self::Hashing(_)
                | Self::Storage(
                    crate::ports::BlobError::Backend(_)
                        | crate::ports::BlobError::IntegrityFailure { .. }
                        | crate::ports::BlobError::NotFound { .. }
                )
                | Self::Render(crate::ports::RenderError::Backend(_))
        )
    }
}
