//! Uploaded filenames and the storage keys derived from content.
//!
//! The security rule this module exists to enforce: **a filename never
//! influences a storage path**. Filenames are attacker-controlled, and every
//! path-traversal bug in a document system starts with one being joined onto a
//! directory. Elrond keeps the original name as a display label only, and derives
//! the actual location from the content checksum.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::checksum::Sha256Checksum;
use crate::error::DomainError;
use crate::media::MediaType;

/// Longest filename Elrond will record.
///
/// Comfortably under the 255-byte limit most filesystems impose, though nothing
/// here is written to a filesystem path.
const FILENAME_MAX: usize = 200;

/// A sanitized display name for an uploaded file.
///
/// Sanitization strips every path component, so `../../etc/passwd` becomes
/// `passwd` and `C:\Windows\system32\x.pdf` becomes `x.pdf`. Even so, this value
/// is never used to build a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct OriginalFilename(String);

impl OriginalFilename {
    /// Sanitizes and validates a filename supplied by a client.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        // Take the last segment after either separator. A Windows client can send
        // backslashes even when the server is Unix, so both are treated as
        // separators regardless of platform.
        let base = raw
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .trim()
            // A leading dot-run is how `..` and `.` arrive; strip it so no name
            // can be interpreted as a relative path segment.
            .trim_start_matches('.')
            .trim();

        if base.is_empty() {
            return Err(DomainError::Required { field: "filename" });
        }
        if base.chars().count() > FILENAME_MAX {
            return Err(DomainError::TooLong {
                field: "filename",
                max: FILENAME_MAX,
            });
        }
        // Control characters would corrupt logs, CSV exports, HTTP headers, and
        // generated binder pages.
        if base.chars().any(char::is_control) {
            return Err(DomainError::Invalid {
                field: "filename",
                reason: "contains_control_characters",
            });
        }
        // A NUL byte truncates strings in C APIs, including some archive and PDF
        // libraries.
        if base.contains('\0') {
            return Err(DomainError::Invalid {
                field: "filename",
                reason: "contains_null_byte",
            });
        }

        Ok(Self(base.to_owned()))
    }

    /// Borrows the sanitized name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The extension, lowercased, without a leading dot.
    pub fn extension(&self) -> Option<String> {
        let (stem, extension) = self.0.rsplit_once('.')?;
        // A leading-dot name like `.gitignore` has no extension, it has a name.
        if stem.is_empty() || extension.is_empty() {
            return None;
        }
        Some(extension.to_ascii_lowercase())
    }

    /// The name without its extension.
    pub fn stem(&self) -> &str {
        match self.0.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem,
            _ => &self.0,
        }
    }

    /// The media type implied by the extension, if any.
    pub fn implied_media_type(&self) -> Option<MediaType> {
        MediaType::from_extension(&self.extension()?)
    }

    /// A filename safe to put in a `Content-Disposition` header.
    ///
    /// Quotes and backslashes are removed rather than escaped, because escaping
    /// rules differ between browsers and a mangled header is worse than a
    /// slightly altered name. Callers should also send the RFC 5987 `filename*`
    /// form for non-ASCII names.
    pub fn for_content_disposition(&self) -> String {
        self.0
            .chars()
            .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
            .collect()
    }
}

impl fmt::Display for OriginalFilename {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for OriginalFilename {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// Which class of file a storage key refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobClass {
    /// A byte-for-byte immutable upload.
    Original,
    /// A generated PDF copy of a non-PDF original.
    Derivative,
    /// A generated binder release.
    Binder,
}

impl BlobClass {
    /// Top-level directory this class lives under.
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Original => "originals",
            Self::Derivative => "derivatives",
            Self::Binder => "binders",
        }
    }
}

/// A content-addressed location, relative to the data directory.
///
/// Built only from a checksum and a class, so there is no input a client can
/// influence and no traversal to guard against.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageKey(String);

impl StorageKey {
    /// How many hex characters of the checksum form each shard directory.
    ///
    /// Two levels of 256 entries keeps any single directory well under the point
    /// where filesystem lookups degrade, even at millions of documents.
    const SHARD_WIDTH: usize = 2;

    /// Derives the key for a piece of content.
    pub fn derive(class: BlobClass, checksum: Sha256Checksum) -> Self {
        let hex = checksum.to_hex();
        let first = &hex[..Self::SHARD_WIDTH];
        let second = &hex[Self::SHARD_WIDTH..Self::SHARD_WIDTH * 2];
        Self(format!("{}/{first}/{second}/{hex}", class.directory()))
    }

    /// Derives the key for a generated PDF derivative.
    ///
    /// The `.pdf` suffix is cosmetic; it makes a data directory legible to an
    /// operator poking around with `ls`.
    pub fn derive_derivative(checksum: Sha256Checksum) -> Self {
        Self(format!(
            "{}.pdf",
            Self::derive(BlobClass::Derivative, checksum).0
        ))
    }

    /// Rehydrates a key read back from storage.
    ///
    /// Rejects anything that is not the shape [`derive`] produces, so a corrupted
    /// or tampered database row cannot turn into a path outside the data
    /// directory.
    ///
    /// [`derive`]: Self::derive
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let invalid = |reason: &'static str| DomainError::Invalid {
            field: "storage_key",
            reason,
        };

        if raw.contains("..") || raw.starts_with('/') || raw.contains('\\') || raw.contains('\0') {
            return Err(invalid("not_a_relative_content_path"));
        }

        let mut parts = raw.split('/');
        let directory = parts.next().ok_or_else(|| invalid("missing_class"))?;
        if ![
            BlobClass::Original.directory(),
            BlobClass::Derivative.directory(),
            BlobClass::Binder.directory(),
        ]
        .contains(&directory)
        {
            return Err(invalid("unknown_class"));
        }

        let first = parts.next().ok_or_else(|| invalid("missing_shard"))?;
        let second = parts.next().ok_or_else(|| invalid("missing_shard"))?;
        let name = parts.next().ok_or_else(|| invalid("missing_name"))?;
        if parts.next().is_some() {
            return Err(invalid("too_many_segments"));
        }

        let digest = name.strip_suffix(".pdf").unwrap_or(name);
        let checksum: Sha256Checksum = digest
            .parse()
            .map_err(|_| invalid("name_is_not_a_digest"))?;

        // The shards must actually be derived from the digest, not merely
        // plausible hex.
        let hex = checksum.to_hex();
        if first != &hex[..Self::SHARD_WIDTH]
            || second != &hex[Self::SHARD_WIDTH..Self::SHARD_WIDTH * 2]
        {
            return Err(invalid("shards_do_not_match_the_digest"));
        }

        Ok(Self(raw.to_owned()))
    }

    /// Borrows the key.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The path segments, for joining onto the data directory.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for StorageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-256 of the empty input.
    const DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn checksum() -> Sha256Checksum {
        DIGEST.parse().expect("valid digest")
    }

    #[test]
    fn path_components_are_stripped() {
        for (input, expected) in [
            ("../../etc/passwd", "passwd"),
            ("/etc/passwd", "passwd"),
            (r"C:\Windows\system32\report.pdf", "report.pdf"),
            (r"..\..\secret.docx", "secret.docx"),
            ("folder/sub/report.pdf", "report.pdf"),
            ("  spaced name.pdf  ", "spaced name.pdf"),
        ] {
            assert_eq!(
                OriginalFilename::parse(input).expect("sanitizes").as_str(),
                expected,
                "for {input:?}"
            );
        }
    }

    #[test]
    fn traversal_only_names_are_rejected() {
        for input in ["..", ".", "../", "./", "...", "/", r"\", "", "   "] {
            assert!(
                OriginalFilename::parse(input).is_err(),
                "accepted {input:?}"
            );
        }
    }

    #[test]
    fn control_characters_and_nulls_are_rejected() {
        assert!(OriginalFilename::parse("report\u{0007}.pdf").is_err());
        assert!(OriginalFilename::parse("report\n.pdf").is_err());
        assert!(OriginalFilename::parse("report\0.pdf").is_err());
    }

    #[test]
    fn overlong_names_are_rejected() {
        let long = format!("{}.pdf", "a".repeat(FILENAME_MAX));
        assert_eq!(
            OriginalFilename::parse(&long).expect_err("too long").code(),
            "field_too_long"
        );
    }

    #[test]
    fn extension_and_stem_are_split_correctly() {
        let name = OriginalFilename::parse("Annual Report 2026.final.PDF").expect("valid");
        assert_eq!(name.extension().as_deref(), Some("pdf"));
        assert_eq!(name.stem(), "Annual Report 2026.final");
        assert_eq!(name.implied_media_type(), Some(MediaType::Pdf));
    }

    #[test]
    fn a_name_with_no_extension_reports_none() {
        let name = OriginalFilename::parse("README").expect("valid");
        assert_eq!(name.extension(), None);
        assert_eq!(name.stem(), "README");
        assert_eq!(name.implied_media_type(), None);
    }

    #[test]
    fn an_unsupported_extension_implies_no_media_type() {
        let name = OriginalFilename::parse("payload.exe").expect("valid");
        assert_eq!(name.extension().as_deref(), Some("exe"));
        assert_eq!(name.implied_media_type(), None);
    }

    #[test]
    fn content_disposition_cannot_be_broken_out_of() {
        let name = OriginalFilename::parse("re\"port.pdf").expect("valid");
        let header = name.for_content_disposition();
        assert!(!header.contains('"'));
        assert!(!header.contains('\\'));
    }

    #[test]
    fn keys_are_derived_from_content_not_from_names() {
        let key = StorageKey::derive(BlobClass::Original, checksum());
        assert_eq!(key.as_str(), format!("originals/e3/b0/{DIGEST}"));
    }

    #[test]
    fn identical_content_yields_an_identical_key() {
        // This is what makes deduplication work.
        assert_eq!(
            StorageKey::derive(BlobClass::Original, checksum()),
            StorageKey::derive(BlobClass::Original, checksum())
        );
    }

    #[test]
    fn classes_are_stored_apart() {
        let original = StorageKey::derive(BlobClass::Original, checksum());
        let derivative = StorageKey::derive_derivative(checksum());
        assert_ne!(original, derivative);
        // Asserting the whole key rather than a suffix: the shard layout and the
        // trailing `.pdf` are both part of the contract the store relies on.
        assert_eq!(
            derivative.as_str(),
            format!("derivatives/e3/b0/{DIGEST}.pdf")
        );
    }

    #[test]
    fn derived_keys_round_trip_through_parsing() {
        for key in [
            StorageKey::derive(BlobClass::Original, checksum()),
            StorageKey::derive(BlobClass::Binder, checksum()),
            StorageKey::derive_derivative(checksum()),
        ] {
            assert_eq!(
                StorageKey::parse(key.as_str()).expect("round-trips"),
                key,
                "for {key}"
            );
        }
    }

    #[test]
    fn a_tampered_key_cannot_escape_the_data_directory() {
        for candidate in [
            "originals/../../etc/passwd",
            "/originals/e3/b0/x",
            r"originals\e3\b0\x",
            "originals/e3/b0/not-a-digest",
            "secrets/e3/b0/e3b0c442",
            // Correct digest, wrong shards: would place content somewhere the
            // deduplication lookup would never find it.
            &format!("originals/ff/ff/{DIGEST}"),
            // An extra segment could be a path smuggled past a naive join.
            &format!("originals/e3/b0/{DIGEST}/extra"),
            "",
        ] {
            assert!(
                StorageKey::parse(candidate).is_err(),
                "accepted {candidate:?}"
            );
        }
    }

    #[test]
    fn segments_never_contain_a_traversal() {
        let key = StorageKey::derive(BlobClass::Original, checksum());
        for segment in key.segments() {
            assert_ne!(segment, "..");
            assert_ne!(segment, ".");
            assert!(!segment.is_empty());
        }
    }
}
