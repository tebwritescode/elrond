//! Elrond domain layer.
//!
//! This crate holds entities, value objects, and workflow invariants. It has no
//! knowledge of Axum, SQLite, the filesystem, or Stirling-PDF. Every rule that
//! must hold regardless of transport or storage lives here, which is what keeps
//! a future move to PostgreSQL or object storage from touching business logic.

pub mod category;
pub mod checksum;
pub mod document;
pub mod error;
pub mod filename;
pub mod id;
pub mod media;
pub mod tag;
pub mod user;

pub use category::{Category, CategoryName, CategoryTree, MoveError};
pub use checksum::Sha256Checksum;
pub use document::{
    Document, DocumentTitle, DocumentVersion, LifecycleState, LifecycleTransitionError,
    VersionNumber,
};
pub use error::DomainError;
pub use filename::{BlobClass, OriginalFilename, StorageKey};
pub use id::{CategoryId, DocumentId, DocumentVersionId, SessionId, TagId, UserId};
pub use media::{DocumentKind, MediaType};
pub use tag::{Tag, TagLabel};
pub use user::{PasswordPolicy, Role, User, Username};

/// Result alias for fallible domain operations.
pub type DomainResult<T> = Result<T, DomainError>;
