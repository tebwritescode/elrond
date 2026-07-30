//! The hierarchical category tree.
//!
//! Every document has exactly one primary category. Categories nest, and the tree
//! must never contain a cycle: a cycle would make the ZIP importer, the binder
//! outline walker, and the breadcrumb renderer all recurse forever.

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::DomainError;
use crate::id::CategoryId;

/// Longest category name.
const NAME_MAX: usize = 120;
/// Deepest nesting Elrond will create.
///
/// Bounded because the ZIP importer turns folder depth into category depth, and
/// an archive can nest arbitrarily.
pub const MAX_DEPTH: usize = 32;

/// A validated category name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct CategoryName(String);

impl CategoryName {
    /// Validates and normalizes a name.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        // Internal whitespace is collapsed so "Board  Minutes" and "Board
        // Minutes" cannot become two sibling categories that look identical.
        let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");

        if normalized.is_empty() {
            return Err(DomainError::Required {
                field: "category_name",
            });
        }
        if normalized.chars().count() > NAME_MAX {
            return Err(DomainError::TooLong {
                field: "category_name",
                max: NAME_MAX,
            });
        }
        if normalized.chars().any(char::is_control) {
            return Err(DomainError::Invalid {
                field: "category_name",
                reason: "contains_control_characters",
            });
        }
        // Path separators would be ambiguous in a breadcrumb and in the ZIP
        // importer's folder mapping.
        if normalized.contains('/') || normalized.contains('\\') {
            return Err(DomainError::Invalid {
                field: "category_name",
                reason: "contains_a_path_separator",
            });
        }

        Ok(Self(normalized))
    }

    /// Borrows the name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A case-insensitive key for sibling-uniqueness comparisons.
    ///
    /// The ZIP importer reuses a matching sibling rather than duplicating it, and
    /// folder names differing only in case should match.
    pub fn matching_key(&self) -> String {
        self.0.to_lowercase()
    }
}

impl fmt::Display for CategoryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CategoryName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// A node in the category tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Category {
    /// Stable identifier.
    pub id: CategoryId,
    /// Parent, or `None` for a root category.
    pub parent_id: Option<CategoryId>,
    /// Display name, unique among its siblings.
    pub name: CategoryName,
    /// Manual ordering among siblings.
    pub position: i64,
    /// Creation timestamp in UTC.
    pub created_at: OffsetDateTime,
    /// Last modification timestamp in UTC.
    pub updated_at: OffsetDateTime,
}

impl Category {
    /// Whether this is a root category.
    pub fn is_root(&self) -> bool {
        self.parent_id.is_none()
    }
}

/// Why a category could not be moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MoveError {
    /// A category cannot be its own parent.
    #[error("a category cannot be its own parent")]
    SelfParent,

    /// The proposed parent is a descendant of the category being moved.
    #[error("a category cannot be moved beneath one of its own descendants")]
    WouldCreateCycle,

    /// The proposed parent does not exist.
    #[error("the target parent category does not exist")]
    MissingParent,

    /// The move would nest deeper than Elrond allows.
    #[error("categories cannot nest more than {max} levels deep")]
    TooDeep {
        /// Maximum permitted depth.
        max: usize,
    },
}

/// An in-memory view of the tree, used to validate structural changes.
///
/// Reparenting is the one operation that cannot be checked with a local rule: a
/// cycle is a property of the whole tree. Rather than issue recursive queries
/// from the use case, the tree is loaded once and asked.
#[derive(Debug, Clone, Default)]
pub struct CategoryTree {
    parents: HashMap<CategoryId, Option<CategoryId>>,
}

impl CategoryTree {
    /// Builds a tree from every category in the library.
    pub fn from_categories<'a>(categories: impl IntoIterator<Item = &'a Category>) -> Self {
        Self {
            parents: categories
                .into_iter()
                .map(|category| (category.id, category.parent_id))
                .collect(),
        }
    }

    /// Whether the tree knows this category.
    pub fn contains(&self, id: CategoryId) -> bool {
        self.parents.contains_key(&id)
    }

    /// Walks from a category to its root, nearest ancestor first.
    ///
    /// Stops if it revisits a node, so an already-corrupt tree cannot hang the
    /// caller.
    pub fn ancestors(&self, id: CategoryId) -> Vec<CategoryId> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor = self.parents.get(&id).copied().flatten();

        while let Some(current) = cursor {
            if !seen.insert(current) {
                break;
            }
            chain.push(current);
            cursor = self.parents.get(&current).copied().flatten();
        }
        chain
    }

    /// Depth of a category, where a root category is depth 1.
    pub fn depth(&self, id: CategoryId) -> usize {
        self.ancestors(id).len() + 1
    }

    /// Depth of the deepest descendant beneath `id`, including `id` itself.
    fn subtree_height(&self, id: CategoryId) -> usize {
        let children: Vec<CategoryId> = self
            .parents
            .iter()
            .filter_map(|(child, parent)| (*parent == Some(id)).then_some(*child))
            .collect();

        children
            .into_iter()
            .map(|child| self.subtree_height(child))
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Whether `candidate` is `id` or one of its descendants.
    pub fn is_self_or_descendant_of(&self, candidate: CategoryId, id: CategoryId) -> bool {
        candidate == id || self.ancestors(candidate).contains(&id)
    }

    /// Validates moving `id` under `new_parent`.
    pub fn validate_move(
        &self,
        id: CategoryId,
        new_parent: Option<CategoryId>,
    ) -> Result<(), MoveError> {
        let Some(parent) = new_parent else {
            // Promoting to a root is always structurally safe.
            return Ok(());
        };

        if parent == id {
            return Err(MoveError::SelfParent);
        }
        if !self.contains(parent) {
            return Err(MoveError::MissingParent);
        }
        if self.is_self_or_descendant_of(parent, id) {
            return Err(MoveError::WouldCreateCycle);
        }

        // The whole subtree moves with the category, so the deepest leaf is what
        // has to fit under the limit.
        let resulting_depth = self.depth(parent) + self.subtree_height(id);
        if resulting_depth > MAX_DEPTH {
            return Err(MoveError::TooDeep { max: MAX_DEPTH });
        }

        Ok(())
    }

    /// Validates creating a child under `parent`.
    pub fn validate_new_child(&self, parent: Option<CategoryId>) -> Result<(), MoveError> {
        let Some(parent) = parent else {
            return Ok(());
        };
        if !self.contains(parent) {
            return Err(MoveError::MissingParent);
        }
        if self.depth(parent) + 1 > MAX_DEPTH {
            return Err(MoveError::TooDeep { max: MAX_DEPTH });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(id: CategoryId, parent: Option<CategoryId>) -> Category {
        Category {
            id,
            parent_id: parent,
            name: CategoryName::parse("Node").expect("valid"),
            position: 0,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    /// Builds a chain root → child → grandchild → …, returning the ids in order.
    fn chain(length: usize) -> (CategoryTree, Vec<CategoryId>) {
        let ids: Vec<CategoryId> = (0..length).map(|_| CategoryId::new()).collect();
        let categories: Vec<Category> = ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                category(
                    *id,
                    if index == 0 {
                        None
                    } else {
                        Some(ids[index - 1])
                    },
                )
            })
            .collect();
        (CategoryTree::from_categories(&categories), ids)
    }

    #[test]
    fn names_collapse_internal_whitespace() {
        assert_eq!(
            CategoryName::parse("  Board   Minutes \n")
                .expect("valid")
                .as_str(),
            "Board Minutes"
        );
    }

    #[test]
    fn names_reject_blank_control_characters_and_separators() {
        for candidate in [
            "",
            "   ",
            "Board\u{0007}",
            "Board/Minutes",
            r"Board\Minutes",
        ] {
            assert!(
                CategoryName::parse(candidate).is_err(),
                "accepted {candidate:?}"
            );
        }
    }

    #[test]
    fn overlong_names_are_rejected() {
        assert_eq!(
            CategoryName::parse(&"a".repeat(NAME_MAX + 1))
                .expect_err("too long")
                .code(),
            "field_too_long"
        );
    }

    #[test]
    fn matching_keys_ignore_case_so_siblings_are_reused() {
        let a = CategoryName::parse("Board Minutes").expect("valid");
        let b = CategoryName::parse("BOARD MINUTES").expect("valid");
        assert_ne!(a, b);
        assert_eq!(a.matching_key(), b.matching_key());
    }

    #[test]
    fn ancestors_walk_from_nearest_to_root() {
        let (tree, ids) = chain(4);
        assert_eq!(tree.ancestors(ids[3]), vec![ids[2], ids[1], ids[0]]);
        assert!(tree.ancestors(ids[0]).is_empty());
    }

    #[test]
    fn depth_counts_a_root_as_one() {
        let (tree, ids) = chain(3);
        assert_eq!(tree.depth(ids[0]), 1);
        assert_eq!(tree.depth(ids[1]), 2);
        assert_eq!(tree.depth(ids[2]), 3);
    }

    #[test]
    fn a_category_cannot_become_its_own_parent() {
        let (tree, ids) = chain(2);
        assert_eq!(
            tree.validate_move(ids[0], Some(ids[0])),
            Err(MoveError::SelfParent)
        );
    }

    #[test]
    fn a_category_cannot_move_beneath_its_own_descendant() {
        let (tree, ids) = chain(4);
        // Moving the root under its grandchild would detach the whole tree into a
        // cycle.
        assert_eq!(
            tree.validate_move(ids[0], Some(ids[2])),
            Err(MoveError::WouldCreateCycle)
        );
        assert_eq!(
            tree.validate_move(ids[1], Some(ids[3])),
            Err(MoveError::WouldCreateCycle)
        );
    }

    #[test]
    fn a_legitimate_move_is_permitted() {
        let root = CategoryId::new();
        let a = CategoryId::new();
        let b = CategoryId::new();
        let tree = CategoryTree::from_categories(&[
            category(root, None),
            category(a, Some(root)),
            category(b, Some(root)),
        ]);

        // Sibling under sibling is fine.
        assert_eq!(tree.validate_move(a, Some(b)), Ok(()));
    }

    #[test]
    fn promoting_to_a_root_is_always_allowed() {
        let (tree, ids) = chain(3);
        assert_eq!(tree.validate_move(ids[2], None), Ok(()));
    }

    #[test]
    fn a_missing_parent_is_reported_rather_than_ignored() {
        let (tree, ids) = chain(2);
        assert_eq!(
            tree.validate_move(ids[1], Some(CategoryId::new())),
            Err(MoveError::MissingParent)
        );
        assert_eq!(
            tree.validate_new_child(Some(CategoryId::new())),
            Err(MoveError::MissingParent)
        );
    }

    #[test]
    fn creating_a_child_at_the_depth_limit_is_refused() {
        let (tree, ids) = chain(MAX_DEPTH);
        assert_eq!(tree.depth(ids[MAX_DEPTH - 1]), MAX_DEPTH);
        assert_eq!(
            tree.validate_new_child(Some(ids[MAX_DEPTH - 1])),
            Err(MoveError::TooDeep { max: MAX_DEPTH })
        );
        // One level shallower is still fine.
        assert_eq!(tree.validate_new_child(Some(ids[MAX_DEPTH - 2])), Ok(()));
    }

    #[test]
    fn a_move_that_would_push_a_subtree_past_the_limit_is_refused() {
        // A chain of 6, moved under the deepest node of another chain such that the
        // combined depth exceeds the limit.
        let deep_ids: Vec<CategoryId> = (0..MAX_DEPTH - 2).map(|_| CategoryId::new()).collect();
        let mut categories: Vec<Category> = deep_ids
            .iter()
            .enumerate()
            .map(|(index, id)| {
                category(
                    *id,
                    if index == 0 {
                        None
                    } else {
                        Some(deep_ids[index - 1])
                    },
                )
            })
            .collect();

        // A separate three-deep subtree.
        let sub_root = CategoryId::new();
        let sub_mid = CategoryId::new();
        let sub_leaf = CategoryId::new();
        categories.push(category(sub_root, None));
        categories.push(category(sub_mid, Some(sub_root)));
        categories.push(category(sub_leaf, Some(sub_mid)));

        let tree = CategoryTree::from_categories(&categories);
        let deepest = *deep_ids.last().expect("non-empty");
        assert_eq!(tree.depth(deepest), MAX_DEPTH - 2);

        // Depth (MAX-2) + height 3 = MAX+1, one too many.
        assert_eq!(
            tree.validate_move(sub_root, Some(deepest)),
            Err(MoveError::TooDeep { max: MAX_DEPTH })
        );
        // Moving only the leaf is fine: height 1 lands exactly on the limit.
        assert_eq!(tree.validate_move(sub_leaf, Some(deepest)), Ok(()));
    }

    #[test]
    fn an_already_corrupt_tree_does_not_hang_the_walker() {
        // Two nodes each claiming the other as parent. This should be impossible,
        // but the walker must terminate rather than recurse forever if it happens.
        let a = CategoryId::new();
        let b = CategoryId::new();
        let tree = CategoryTree::from_categories(&[category(a, Some(b)), category(b, Some(a))]);
        assert!(tree.ancestors(a).len() <= 2);
    }

    #[test]
    fn descendant_checks_include_the_node_itself() {
        let (tree, ids) = chain(3);
        assert!(tree.is_self_or_descendant_of(ids[0], ids[0]));
        assert!(tree.is_self_or_descendant_of(ids[2], ids[0]));
        assert!(!tree.is_self_or_descendant_of(ids[0], ids[2]));
    }
}
