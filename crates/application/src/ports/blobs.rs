//! Content-addressed blob storage.
//!
//! The port is deliberately narrow: put bytes, read bytes, drop bytes. It says
//! nothing about a filesystem, so an object-storage implementation is a
//! substitution rather than a rewrite.

use async_trait::async_trait;
use elrond_domain::{BlobClass, MediaType, Sha256Checksum, StorageKey};
use thiserror::Error;

/// Identifies a file's type from its contents.
///
/// A separate port because sniffing magic bytes is a library concern, while the
/// rule about what to do when the contents and the filename disagree is a domain
/// decision that belongs in a use case.
pub trait ContentInspector: Send + Sync + 'static {
    /// Identifies content, or returns `None` when it has no recognizable
    /// signature.
    ///
    /// Plain text, Markdown, and CSV have no magic bytes, so `None` is a normal
    /// answer rather than a failure.
    fn detect(&self, bytes: &[u8]) -> Option<MediaType>;
}

/// A failure in the blob store.
#[derive(Debug, Error)]
pub enum BlobError {
    /// The requested blob is not present.
    #[error("no stored content for {key}")]
    NotFound {
        /// The key that was asked for.
        key: String,
    },

    /// The stored bytes did not match the checksum they are filed under.
    ///
    /// Distinct from a generic I/O failure because it means the store has been
    /// corrupted or tampered with, which is an operational emergency rather than
    /// a transient error.
    #[error("stored content for {key} does not match its checksum")]
    IntegrityFailure {
        /// The key whose content failed verification.
        key: String,
    },

    /// The upload exceeded the configured size limit.
    #[error("content exceeds the {limit} byte limit")]
    TooLarge {
        /// Configured maximum in bytes.
        limit: u64,
    },

    /// Anything else the backend reported.
    #[error("blob storage failure")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}

impl BlobError {
    /// Wraps an arbitrary backend error.
    pub fn backend<E>(error: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::Backend(Box::new(error))
    }
}

/// The outcome of storing content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    /// Where the content now lives.
    pub key: StorageKey,
    /// Checksum computed while writing.
    pub checksum: Sha256Checksum,
    /// Size in bytes.
    pub byte_size: u64,
    /// Whether identical content was already present.
    ///
    /// Reported so the caller can tell a genuine upload from a deduplicated one
    /// and record the fact in the audit trail.
    pub deduplicated: bool,
}

/// Immutable content storage.
#[async_trait]
pub trait BlobStore: Send + Sync + 'static {
    /// Stores content and returns where it went.
    ///
    /// The checksum is computed by the store while writing rather than supplied by
    /// the caller, so the key can never disagree with the bytes behind it. If
    /// identical content is already present the existing blob is kept and
    /// `deduplicated` is set: content is immutable, so rewriting it could only
    /// ever produce the same bytes or corrupt them.
    async fn put(&self, class: BlobClass, bytes: Vec<u8>) -> Result<StoredBlob, BlobError>;

    /// Reads content back.
    async fn get(&self, key: &StorageKey) -> Result<Vec<u8>, BlobError>;

    /// Whether content exists.
    async fn exists(&self, key: &StorageKey) -> Result<bool, BlobError>;

    /// Reads content back and verifies it still matches its checksum.
    ///
    /// Used for backup verification and integrity audits, not on the hot read
    /// path: rehashing every download would make large files expensive to serve.
    async fn get_verified(&self, key: &StorageKey) -> Result<Vec<u8>, BlobError>;

    /// Removes content.
    ///
    /// Only ever called once no version references the blob. Removing a blob that
    /// is already absent is not an error, so cleanup is idempotent.
    async fn delete(&self, key: &StorageKey) -> Result<(), BlobError>;
}
