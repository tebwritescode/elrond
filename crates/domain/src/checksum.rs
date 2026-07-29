//! Content checksums.
//!
//! The domain models a checksum as an opaque 32-byte value with a canonical
//! lowercase hex representation. Computing it is infrastructure work; comparing
//! and storing it is a domain concern, because deduplication and binder
//! reproducibility both depend on checksum equality.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// Length of a SHA-256 digest in bytes.
pub const SHA256_BYTES: usize = 32;

/// A SHA-256 content checksum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sha256Checksum([u8; SHA256_BYTES]);

impl Sha256Checksum {
    /// Wraps a raw digest.
    pub const fn from_bytes(bytes: [u8; SHA256_BYTES]) -> Self {
        Self(bytes)
    }

    /// Borrows the raw digest.
    pub const fn as_bytes(&self) -> &[u8; SHA256_BYTES] {
        &self.0
    }

    /// Renders the canonical lowercase hex form.
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(SHA256_BYTES * 2);
        for byte in self.0 {
            use fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
        }
        out
    }

    /// Returns the first `n` hex characters, used for content-addressed
    /// storage sharding.
    ///
    /// Sharding on the checksum rather than the filename is deliberate: a
    /// user-supplied filename must never influence a storage path.
    pub fn hex_prefix(self, n: usize) -> String {
        self.to_hex().chars().take(n).collect()
    }
}

impl fmt::Display for Sha256Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl FromStr for Sha256Checksum {
    type Err = DomainError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != SHA256_BYTES * 2 {
            return Err(DomainError::Invalid {
                field: "checksum",
                reason: "expected_64_hex_characters",
            });
        }
        let mut bytes = [0_u8; SHA256_BYTES];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            let high = hex_value(chunk[0])?;
            let low = hex_value(chunk[1])?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for Sha256Checksum {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Sha256Checksum {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// Decodes one lowercase or uppercase hex digit.
fn hex_value(byte: u8) -> Result<u8, DomainError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(DomainError::Invalid {
            field: "checksum",
            reason: "non_hex_character",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SHA-256 of the empty input, a well-known constant.
    const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn hex_round_trips() {
        let checksum: Sha256Checksum = EMPTY_SHA256.parse().expect("valid digest");
        assert_eq!(checksum.to_hex(), EMPTY_SHA256);
    }

    #[test]
    fn uppercase_hex_is_accepted_and_normalized() {
        let checksum: Sha256Checksum = EMPTY_SHA256.to_uppercase().parse().expect("valid digest");
        assert_eq!(checksum.to_hex(), EMPTY_SHA256);
    }

    #[test]
    fn wrong_length_is_rejected() {
        let error = "abc".parse::<Sha256Checksum>().expect_err("too short");
        assert_eq!(error.code(), "field_invalid");
    }

    #[test]
    fn non_hex_characters_are_rejected() {
        let bad = "z".repeat(64);
        assert!(bad.parse::<Sha256Checksum>().is_err());
    }

    #[test]
    fn prefix_is_used_for_storage_sharding() {
        let checksum: Sha256Checksum = EMPTY_SHA256.parse().expect("valid digest");
        assert_eq!(checksum.hex_prefix(2), "e3");
        assert_eq!(checksum.hex_prefix(4), "e3b0");
    }

    #[test]
    fn serde_uses_the_hex_form() {
        let checksum: Sha256Checksum = EMPTY_SHA256.parse().expect("valid digest");
        let json = serde_json::to_string(&checksum).expect("serializes");
        assert_eq!(json, format!("\"{EMPTY_SHA256}\""));
    }
}
