//! Archive extraction port.
//!
//! The ZIP importer needs the entries of an uploaded archive, not the archive
//! format itself, so the format lives behind a trait like every other piece of
//! infrastructure. Extraction is pure computation over bytes already in memory,
//! which is why this port is synchronous where the storage ports are not.

use thiserror::Error;

/// One file lifted out of an archive.
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    /// The directories enclosing the file, outermost first. Empty for a file at
    /// the archive root.
    pub directories: Vec<String>,
    /// The file's own name, with no path components.
    pub filename: String,
    /// The decompressed bytes.
    pub bytes: Vec<u8>,
}

/// Ceilings the extractor enforces while decompressing.
///
/// Enforced during extraction rather than checked afterwards, because a zip
/// bomb's whole point is that the damage happens while inflating.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveLimits {
    /// Maximum number of file entries.
    pub max_entries: usize,
    /// Maximum total decompressed size across every entry.
    pub max_total_bytes: u64,
}

/// A failure while reading an archive.
#[derive(Debug, Error)]
pub enum ArchiveError {
    /// The bytes are not a readable archive of the expected format.
    #[error("this file could not be read as a ZIP archive")]
    Unreadable,

    /// Decompressing would exceed the size ceiling.
    #[error("the archive decompresses to more than {limit} bytes")]
    TooLarge {
        /// The configured ceiling.
        limit: u64,
    },

    /// The archive holds more entries than allowed.
    #[error("the archive holds more than {limit} files")]
    TooManyEntries {
        /// The configured ceiling.
        limit: usize,
    },

    /// An entry could not be decompressed.
    #[error("entry {name:?} could not be extracted")]
    Entry {
        /// The entry's declared name.
        name: String,
    },
}

/// Extracts the file entries of an archive held in memory.
pub trait ArchiveExtractor: Send + Sync + 'static {
    /// Returns every file entry, skipping directories.
    ///
    /// Entry paths are returned decomposed and sanitized: no absolute paths, no
    /// `..`, no empty components. An entry whose name cannot be made safe is an
    /// [`ArchiveError::Entry`] failure rather than a silently altered path.
    fn extract(
        &self,
        bytes: &[u8],
        limits: &ArchiveLimits,
    ) -> Result<Vec<ArchiveEntry>, ArchiveError>;
}
