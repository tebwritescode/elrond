//! Category tree use cases.

use std::collections::HashMap;
use std::sync::Arc;

use elrond_domain::{Category, CategoryId, CategoryName, CategoryTree, MoveError};
use serde::Serialize;

use crate::error::{ApplicationError, ApplicationResult};
use crate::ports::{CategoryRepository, Clock, DocumentRepository, NewCategory};

/// Name given to the category that holds documents nothing else claims.
///
/// Created on demand rather than at setup, so an instance that never uploads
/// anything has an empty tree instead of a stub.
pub const UNFILED_CATEGORY: &str = "Unfiled";

/// A category with its children, ready to render as a tree.
#[derive(Debug, Clone, Serialize)]
pub struct CategoryNode {
    /// Identifier.
    pub id: CategoryId,
    /// Display name.
    pub name: String,
    /// Documents filed directly in this category.
    pub document_count: u64,
    /// Documents in this category and everything beneath it.
    ///
    /// Both counts are reported because the tree shows the rolled-up number while
    /// filtering by a category alone uses the direct one.
    pub total_document_count: u64,
    /// Nested children, in display order.
    pub children: Vec<CategoryNode>,
}

/// Category use cases.
#[derive(Clone)]
pub struct CategoryService {
    categories: Arc<dyn CategoryRepository>,
    documents: Arc<dyn DocumentRepository>,
    clock: Arc<dyn Clock>,
}

impl CategoryService {
    /// Wires the use cases to their adapters.
    pub fn new(
        categories: Arc<dyn CategoryRepository>,
        documents: Arc<dyn DocumentRepository>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            categories,
            documents,
            clock,
        }
    }

    /// Reads the whole tree with document counts.
    pub async fn tree(&self) -> ApplicationResult<Vec<CategoryNode>> {
        let categories = self.categories.list_all().await?;
        let counts: HashMap<CategoryId, u64> = self
            .categories
            .document_counts()
            .await?
            .into_iter()
            .collect();

        // Children are grouped in one pass rather than by re-scanning the list per
        // parent, so building the tree stays linear in the number of categories.
        let mut children: HashMap<Option<CategoryId>, Vec<&Category>> = HashMap::new();
        for category in &categories {
            children
                .entry(category.parent_id)
                .or_default()
                .push(category);
        }
        for siblings in children.values_mut() {
            siblings.sort_by(|a, b| {
                a.position
                    .cmp(&b.position)
                    .then_with(|| a.name.as_str().cmp(b.name.as_str()))
            });
        }

        Ok(build_nodes(None, &children, &counts))
    }

    /// Creates a category, reusing an existing sibling of the same name.
    ///
    /// Reuse rather than rejection because this is also how the ZIP importer maps
    /// folders: importing the same tree twice must not produce duplicates.
    pub async fn ensure(
        &self,
        parent_id: Option<CategoryId>,
        name: &CategoryName,
    ) -> ApplicationResult<Category> {
        if let Some(existing) = self.categories.find_child_by_name(parent_id, name).await? {
            return Ok(existing);
        }

        let tree = self.load_tree().await?;
        tree.validate_new_child(parent_id).map_err(to_error)?;

        // Appended after existing siblings, so manual ordering already applied to
        // the tree is not disturbed.
        let position = self
            .categories
            .list_all()
            .await?
            .iter()
            .filter(|candidate| candidate.parent_id == parent_id)
            .map(|candidate| candidate.position)
            .max()
            .map_or(0, |highest| highest.saturating_add(1));

        Ok(self
            .categories
            .insert(NewCategory {
                id: CategoryId::new(),
                parent_id,
                name: name.clone(),
                position,
                created_at: self.clock.now(),
            })
            .await?)
    }

    /// Creates a category, refusing a duplicate sibling name.
    ///
    /// The interactive counterpart to [`ensure`]: someone typing a name that
    /// already exists has made a mistake and should be told, rather than silently
    /// shown the other category.
    ///
    /// [`ensure`]: Self::ensure
    pub async fn create(
        &self,
        parent_id: Option<CategoryId>,
        name: &CategoryName,
    ) -> ApplicationResult<Category> {
        if self
            .categories
            .find_child_by_name(parent_id, name)
            .await?
            .is_some()
        {
            return Err(ApplicationError::Conflict {
                resource: "category",
                reason: "a category with that name already exists here",
            });
        }
        self.ensure(parent_id, name).await
    }

    /// Returns the "Unfiled" root category, creating it if needed.
    pub async fn unfiled(&self) -> ApplicationResult<Category> {
        let name = CategoryName::parse(UNFILED_CATEGORY)?;
        self.ensure(None, &name).await
    }

    /// Renames a category.
    pub async fn rename(&self, id: CategoryId, name: &CategoryName) -> ApplicationResult<Category> {
        let Some(existing) = self.categories.find_by_id(id).await? else {
            return Err(ApplicationError::NotFound {
                resource: "category",
            });
        };

        if let Some(clash) = self
            .categories
            .find_child_by_name(existing.parent_id, name)
            .await?
            && clash.id != id
        {
            return Err(ApplicationError::Conflict {
                resource: "category",
                reason: "a sibling category already has that name",
            });
        }

        self.categories.rename(id, name, self.clock.now()).await?;
        self.categories
            .find_by_id(id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "category",
            })
    }

    /// Moves a category to a new parent.
    pub async fn move_to(
        &self,
        id: CategoryId,
        parent_id: Option<CategoryId>,
    ) -> ApplicationResult<Category> {
        let Some(existing) = self.categories.find_by_id(id).await? else {
            return Err(ApplicationError::NotFound {
                resource: "category",
            });
        };
        if existing.parent_id == parent_id {
            return Ok(existing);
        }

        // Cycle and depth checks need the whole tree, so they cannot be expressed
        // as a constraint on the row being written.
        let tree = self.load_tree().await?;
        tree.validate_move(id, parent_id).map_err(to_error)?;

        if let Some(clash) = self
            .categories
            .find_child_by_name(parent_id, &existing.name)
            .await?
            && clash.id != id
        {
            return Err(ApplicationError::Conflict {
                resource: "category",
                reason: "the destination already has a category with that name",
            });
        }

        self.categories
            .set_parent(id, parent_id, self.clock.now())
            .await?;
        self.categories
            .find_by_id(id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "category",
            })
    }

    /// Deletes an empty category.
    ///
    /// Refuses while documents or children remain: deleting them along with it
    /// would be an irreversible action triggered by a single click.
    pub async fn delete(&self, id: CategoryId) -> ApplicationResult<()> {
        if self.categories.find_by_id(id).await?.is_none() {
            return Err(ApplicationError::NotFound {
                resource: "category",
            });
        }

        let categories = self.categories.list_all().await?;
        if categories.iter().any(|c| c.parent_id == Some(id)) {
            return Err(ApplicationError::Conflict {
                resource: "category",
                reason: "the category still has child categories",
            });
        }

        let filter = crate::ports::DocumentFilter {
            category_id: Some(id),
            include_descendants: false,
            limit: 1,
            ..Default::default()
        };
        if self.documents.list(&filter).await?.total > 0 {
            return Err(ApplicationError::Conflict {
                resource: "category",
                reason: "the category still contains documents",
            });
        }

        self.categories.delete(id).await?;
        Ok(())
    }

    /// Resolves a category id, or fails with a not-found error.
    pub async fn require(&self, id: CategoryId) -> ApplicationResult<Category> {
        self.categories
            .find_by_id(id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "category",
            })
    }

    /// Every category at or beneath `id`, including `id` itself.
    ///
    /// Used to expand a category filter across its descendants.
    pub async fn subtree_ids(&self, id: CategoryId) -> ApplicationResult<Vec<CategoryId>> {
        let categories = self.categories.list_all().await?;
        let tree = CategoryTree::from_categories(&categories);
        Ok(categories
            .iter()
            .map(|category| category.id)
            .filter(|candidate| tree.is_self_or_descendant_of(*candidate, id))
            .collect())
    }

    /// Loads the tree for structural validation.
    async fn load_tree(&self) -> ApplicationResult<CategoryTree> {
        let categories = self.categories.list_all().await?;
        Ok(CategoryTree::from_categories(&categories))
    }
}

/// Recursively assembles nodes for one level of the tree.
fn build_nodes(
    parent: Option<CategoryId>,
    children: &HashMap<Option<CategoryId>, Vec<&Category>>,
    counts: &HashMap<CategoryId, u64>,
) -> Vec<CategoryNode> {
    children
        .get(&parent)
        .map(|siblings| {
            siblings
                .iter()
                .map(|category| {
                    let nested = build_nodes(Some(category.id), children, counts);
                    let direct = counts.get(&category.id).copied().unwrap_or(0);
                    let total = direct
                        + nested
                            .iter()
                            .map(|child| child.total_document_count)
                            .sum::<u64>();
                    CategoryNode {
                        id: category.id,
                        name: category.name.to_string(),
                        document_count: direct,
                        total_document_count: total,
                        children: nested,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Maps a structural failure onto the application error the API can render.
fn to_error(error: MoveError) -> ApplicationError {
    match error {
        MoveError::MissingParent => ApplicationError::NotFound {
            resource: "parent category",
        },
        MoveError::SelfParent => ApplicationError::Conflict {
            resource: "category",
            reason: "a category cannot be its own parent",
        },
        MoveError::WouldCreateCycle => ApplicationError::Conflict {
            resource: "category",
            reason: "that move would place a category inside itself",
        },
        MoveError::TooDeep { .. } => ApplicationError::Conflict {
            resource: "category",
            reason: "categories cannot nest any deeper",
        },
    }
}
