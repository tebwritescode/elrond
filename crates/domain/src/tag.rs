//! Free-form tags.
//!
//! A document has one primary category and any number of tags. Tags are flat and
//! cheap to create, which is what makes them useful for the cross-cutting facets
//! a single hierarchy cannot express.

use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::DomainError;
use crate::id::TagId;

/// Longest tag label.
const LABEL_MAX: usize = 48;
/// Most tags one document may carry.
///
/// Bounded so a bulk import cannot attach thousands of tags to a document and
/// make every library view expensive to render.
pub const MAX_TAGS_PER_DOCUMENT: usize = 50;

/// A validated tag label.
///
/// Carries both the label as typed and a normalized key. The label is what a user
/// sees; the key is what uniqueness and lookup use, so `Board Minutes`,
/// `board minutes`, and `Board  Minutes` are all the same tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct TagLabel(String);

impl TagLabel {
    /// Validates and normalizes a label.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");

        if normalized.is_empty() {
            return Err(DomainError::Required { field: "tag" });
        }
        if normalized.chars().count() > LABEL_MAX {
            return Err(DomainError::TooLong {
                field: "tag",
                max: LABEL_MAX,
            });
        }
        if normalized.chars().any(char::is_control) {
            return Err(DomainError::Invalid {
                field: "tag",
                reason: "contains_control_characters",
            });
        }
        // Commas are how tag lists arrive in query strings and CSV exports.
        if normalized.contains(',') {
            return Err(DomainError::Invalid {
                field: "tag",
                reason: "contains_a_comma",
            });
        }

        Ok(Self(normalized))
    }

    /// Borrows the label as typed.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The normalized key used for uniqueness and lookup.
    pub fn key(&self) -> String {
        self.0.to_lowercase()
    }
}

impl fmt::Display for TagLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TagLabel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// A tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tag {
    /// Stable identifier.
    pub id: TagId,
    /// Label as typed.
    pub label: TagLabel,
    /// Creation timestamp in UTC.
    pub created_at: OffsetDateTime,
}

/// Normalizes a set of labels for assignment to a document.
///
/// Deduplicates by normalized key, keeping the first spelling seen, and enforces
/// the per-document cap.
pub fn normalize_tag_set(labels: &[TagLabel]) -> Result<Vec<TagLabel>, DomainError> {
    let mut seen = std::collections::HashSet::new();
    let mut unique = Vec::new();

    for label in labels {
        if seen.insert(label.key()) {
            unique.push(label.clone());
        }
    }

    if unique.len() > MAX_TAGS_PER_DOCUMENT {
        return Err(DomainError::TooLong {
            field: "tags",
            max: MAX_TAGS_PER_DOCUMENT,
        });
    }
    Ok(unique)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(raw: &str) -> TagLabel {
        TagLabel::parse(raw).expect("valid label")
    }

    #[test]
    fn labels_collapse_whitespace() {
        assert_eq!(label("  board   minutes ").as_str(), "board minutes");
    }

    #[test]
    fn labels_reject_blank_controls_and_commas() {
        for candidate in ["", "   ", "a\u{0007}b", "a,b"] {
            assert!(
                TagLabel::parse(candidate).is_err(),
                "accepted {candidate:?}"
            );
        }
    }

    #[test]
    fn overlong_labels_are_rejected() {
        assert_eq!(
            TagLabel::parse(&"a".repeat(LABEL_MAX + 1))
                .expect_err("too long")
                .code(),
            "field_too_long"
        );
    }

    #[test]
    fn keys_ignore_case_but_labels_preserve_it() {
        let typed = label("Board Minutes");
        assert_eq!(typed.as_str(), "Board Minutes");
        assert_eq!(typed.key(), "board minutes");
        assert_eq!(typed.key(), label("BOARD MINUTES").key());
    }

    #[test]
    fn a_tag_set_deduplicates_by_key_and_keeps_the_first_spelling() {
        let set = normalize_tag_set(&[
            label("Board Minutes"),
            label("board minutes"),
            label("BOARD  MINUTES"),
            label("Policy"),
        ])
        .expect("within the cap");

        assert_eq!(set.len(), 2);
        assert_eq!(set[0].as_str(), "Board Minutes");
        assert_eq!(set[1].as_str(), "Policy");
    }

    #[test]
    fn an_empty_set_is_valid() {
        assert!(normalize_tag_set(&[]).expect("valid").is_empty());
    }

    #[test]
    fn the_per_document_cap_is_enforced_after_deduplication() {
        // Duplicates must not count toward the cap.
        let duplicated: Vec<TagLabel> = (0..MAX_TAGS_PER_DOCUMENT)
            .flat_map(|index| [label(&format!("tag{index}")), label(&format!("TAG{index}"))])
            .collect();
        assert_eq!(
            normalize_tag_set(&duplicated)
                .expect("cap applies post-dedup")
                .len(),
            MAX_TAGS_PER_DOCUMENT
        );

        let too_many: Vec<TagLabel> = (0..=MAX_TAGS_PER_DOCUMENT)
            .map(|index| label(&format!("tag{index}")))
            .collect();
        assert_eq!(
            normalize_tag_set(&too_many)
                .expect_err("over the cap")
                .code(),
            "field_too_long"
        );
    }
}
