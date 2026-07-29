//! Service interfaces the infrastructure layer must satisfy.
//!
//! Ports are defined by what the use cases need, not by what SQLite happens to
//! make convenient. That is what keeps a future PostgreSQL or object-storage
//! backend a drop-in change.

use std::fmt;

use async_trait::async_trait;
use elrond_domain::{DisplayName, EmailAddress, Role, SessionId, User, UserId};
use thiserror::Error;
use time::{Duration, OffsetDateTime};

/// A storage failure.
#[derive(Debug, Error)]
pub enum RepositoryError {
    /// A uniqueness constraint was violated.
    #[error("that {field} is already taken")]
    UniqueViolation {
        /// Resource kind, safe to surface to clients.
        resource: &'static str,
        /// Conflicting field, safe to surface to clients.
        field: &'static str,
    },

    /// Anything else the backend reported.
    ///
    /// The `Display` text is deliberately generic; the underlying cause is kept
    /// as a source for logs so driver messages never reach a client response.
    #[error("storage backend failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl RepositoryError {
    /// Wraps an arbitrary backend error.
    pub fn backend<E>(error: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Backend(Box::new(error))
    }
}

/// A failure inside the password hasher itself.
///
/// A wrong password is *not* an error — `verify` returns `Ok(false)` for that.
/// This variant covers malformed stored hashes and hasher misconfiguration.
#[derive(Debug, Error)]
pub enum HashingError {
    /// The stored hash string could not be parsed.
    #[error("stored password hash is malformed")]
    MalformedHash,

    /// The hasher failed to run.
    #[error("password hashing failed")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// A serialized password hash, in PHC string format.
///
/// Wrapped in a newtype whose `Debug` and `Display` redact the contents, so a
/// stray `tracing` call can never write a verifier into the log.
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash(String);

impl PasswordHash {
    /// Wraps a PHC-format hash string.
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Exposes the hash for storage or verification.
    ///
    /// Named to make call sites obvious in review.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasswordHash(redacted)")
    }
}

impl fmt::Display for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// An opaque bearer token handed to a client exactly once.
///
/// Redacted in `Debug`/`Display` for the same reason as [`PasswordHash`].
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    /// Wraps generated token material.
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Exposes the token so it can be written to a cookie.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(redacted)")
    }
}

impl fmt::Display for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// The stored, non-reversible form of a session token.
///
/// Only the fingerprint is persisted, so a database dump does not yield usable
/// session cookies.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenFingerprint(String);

impl TokenFingerprint {
    /// Wraps a computed fingerprint.
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the fingerprint.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A user together with the credential material needed to authenticate it.
///
/// The hash is kept out of [`User`] so the entity that gets serialized into API
/// responses simply has no field that could leak.
#[derive(Debug, Clone)]
pub struct Credentialed {
    /// The account.
    pub user: User,
    /// Its stored password hash.
    pub password_hash: PasswordHash,
}

/// Fields required to create an account.
///
/// The identifier and timestamp are supplied by the caller rather than invented
/// by the repository, so every adapter shares the one injected clock and the
/// values are reproducible in tests.
#[derive(Debug, Clone)]
pub struct NewUser {
    /// Stable identifier.
    pub id: UserId,
    /// Normalized login address.
    pub email: EmailAddress,
    /// Name shown in the interface.
    pub display_name: DisplayName,
    /// Authority level.
    pub role: Role,
    /// Already-hashed password.
    pub password_hash: PasswordHash,
    /// Creation timestamp, also used as the initial `updated_at`.
    pub created_at: OffsetDateTime,
}

/// Account persistence.
#[async_trait]
pub trait UserRepository: Send + Sync + 'static {
    /// Counts accounts, used to detect an uninitialized instance.
    async fn count(&self) -> Result<u64, RepositoryError>;

    /// Looks up an account and its credential by login address.
    async fn find_credentialed_by_email(
        &self,
        email: &EmailAddress,
    ) -> Result<Option<Credentialed>, RepositoryError>;

    /// Looks up an account by identifier.
    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, RepositoryError>;

    /// Creates an account.
    async fn insert(&self, new_user: NewUser) -> Result<User, RepositoryError>;

    /// Lists accounts oldest first.
    async fn list(&self) -> Result<Vec<User>, RepositoryError>;
}

/// A persisted session.
#[derive(Debug, Clone)]
pub struct SessionRecord {
    /// Database key.
    pub id: SessionId,
    /// Owning account.
    pub user_id: UserId,
    /// When the session was established.
    pub created_at: OffsetDateTime,
    /// Most recent authenticated request.
    pub last_seen_at: OffsetDateTime,
    /// Hard expiry, enforced regardless of activity.
    pub expires_at: OffsetDateTime,
}

/// Fields required to create a session.
#[derive(Debug, Clone)]
pub struct NewSession {
    /// Database key.
    pub id: SessionId,
    /// Owning account.
    pub user_id: UserId,
    /// Hashed bearer token.
    pub token_fingerprint: TokenFingerprint,
    /// Creation time.
    pub created_at: OffsetDateTime,
    /// Hard expiry.
    pub expires_at: OffsetDateTime,
}

/// Session persistence.
#[async_trait]
pub trait SessionRepository: Send + Sync + 'static {
    /// Stores a new session.
    async fn insert(&self, session: NewSession) -> Result<(), RepositoryError>;

    /// Finds a session by token fingerprint, ignoring expiry.
    ///
    /// Expiry is evaluated by the use case against the injected clock so the
    /// rule is testable without waiting.
    async fn find_by_fingerprint(
        &self,
        fingerprint: &TokenFingerprint,
    ) -> Result<Option<SessionRecord>, RepositoryError>;

    /// Records activity and extends the idle window.
    async fn touch(
        &self,
        id: SessionId,
        seen_at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    /// Revokes one session.
    async fn delete(&self, id: SessionId) -> Result<(), RepositoryError>;

    /// Revokes every session belonging to an account.
    async fn delete_for_user(&self, user_id: UserId) -> Result<u64, RepositoryError>;

    /// Removes sessions that expired before `now`. Returns how many were purged.
    async fn delete_expired(&self, now: OffsetDateTime) -> Result<u64, RepositoryError>;
}

/// Password hashing.
///
/// Async because Argon2id is intentionally slow; an implementation must move the
/// work off the async runtime rather than stalling the executor.
#[async_trait]
pub trait PasswordHasher: Send + Sync + 'static {
    /// Hashes a password with a fresh random salt.
    async fn hash(&self, password: String) -> Result<PasswordHash, HashingError>;

    /// Verifies a password in constant time relative to the stored hash.
    async fn verify(&self, password: String, hash: PasswordHash) -> Result<bool, HashingError>;
}

/// Session token generation and fingerprinting.
#[async_trait]
pub trait SessionTokens: Send + Sync + 'static {
    /// Produces a fresh token from a cryptographically secure source.
    fn generate(&self) -> SessionToken;

    /// Computes the stored fingerprint of a token.
    fn fingerprint(&self, token: &SessionToken) -> TokenFingerprint;
}

/// The current time, injected so time-dependent rules are testable.
pub trait Clock: Send + Sync + 'static {
    /// Now, in UTC.
    fn now(&self) -> OffsetDateTime;
}

/// How long sessions live.
#[derive(Debug, Clone, Copy)]
pub struct SessionPolicy {
    /// How long a session may sit unused before it is rejected.
    pub idle_timeout: Duration,
    /// Maximum total lifetime, regardless of activity.
    pub absolute_lifetime: Duration,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::hours(12),
            absolute_lifetime: Duration::days(30),
        }
    }
}
