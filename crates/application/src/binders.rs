//! Binder generation.
//!
//! A binder is produced directly from the category tree rather than from a stored
//! binder entity. That is a deliberate first step: it delivers the thing people
//! actually need — upload, categorise, print the lot — without first requiring
//! them to build and maintain a separate outline. A persistent binder with pinned
//! releases layers on top of this later without changing the renderer.

use std::collections::HashMap;
use std::sync::Arc;

use elrond_domain::{Category, CategoryId, LifecycleState, Role};
use time::format_description::well_known::Rfc3339;

use crate::auth::Authenticated;
use crate::error::{ApplicationError, ApplicationResult};
use crate::ports::{
    BinderPlan, BinderRenderer, BinderSettings, BlobStore, CategoryRepository, Clock, CoverSpec,
    DocumentFilter, DocumentRepository, DocumentSort, PlanEntry, SortOrder,
};

/// What to put in a binder.
#[derive(Debug, Clone)]
pub struct BuildBinderRequest {
    /// Cover title.
    pub title: String,
    /// Optional cover subtitle.
    pub subtitle: Option<String>,
    /// Optional owning organization, shown on the cover.
    pub organization: Option<String>,
    /// Categories to include. Empty means the whole library.
    ///
    /// A selected category always brings its descendants, because a binder of
    /// "Policies" that silently omitted "Policies / 2026" would be wrong.
    pub category_ids: Vec<CategoryId>,
    /// Lifecycle states to include. Empty means everything the caller may see.
    pub lifecycles: Vec<LifecycleState>,
    /// Output settings.
    pub settings: BinderSettings,
}

/// A generated binder.
#[derive(Debug, Clone)]
pub struct GeneratedBinder {
    /// The PDF bytes.
    pub pdf: Vec<u8>,
    /// Total pages.
    pub page_count: u32,
    /// How many documents were included.
    pub document_count: u32,
    /// Documents left out because no PDF is available for them yet.
    ///
    /// Reported rather than silently dropped: a binder that is quietly missing a
    /// document is worse than one that says so.
    pub skipped: Vec<String>,
}

/// Binder generation use cases.
#[derive(Clone)]
pub struct BinderService {
    categories: Arc<dyn CategoryRepository>,
    documents: Arc<dyn DocumentRepository>,
    blobs: Arc<dyn BlobStore>,
    renderer: Arc<dyn BinderRenderer>,
    clock: Arc<dyn Clock>,
}

impl BinderService {
    /// Wires the use cases to their adapters.
    pub fn new(
        categories: Arc<dyn CategoryRepository>,
        documents: Arc<dyn DocumentRepository>,
        blobs: Arc<dyn BlobStore>,
        renderer: Arc<dyn BinderRenderer>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            categories,
            documents,
            blobs,
            renderer,
            clock,
        }
    }

    /// Builds a printable binder.
    pub async fn build(
        &self,
        actor: &Authenticated,
        request: BuildBinderRequest,
    ) -> ApplicationResult<GeneratedBinder> {
        // Reviewers and above may bind drafts; a viewer only ever sees published
        // material, so its binder can only contain published material.
        let lifecycles = if actor.user.role.satisfies(Role::Reviewer) {
            request.lifecycles.clone()
        } else {
            vec![LifecycleState::Published]
        };

        let all = self.categories.list_all().await?;
        let selected = select_categories(&all, &request.category_ids);
        if selected.is_empty() {
            return Err(ApplicationError::Conflict {
                resource: "binder",
                reason: "no categories match the selection",
            });
        }

        let mut entries = Vec::new();
        let mut skipped = Vec::new();
        let mut document_count = 0_u32;

        for (category, level, path) in &selected {
            // Documents filed directly here. Descendants get their own section, so
            // including them again would duplicate every nested document.
            let page = self
                .documents
                .list(&DocumentFilter {
                    category_id: Some(category.id),
                    include_descendants: false,
                    lifecycles: lifecycles.clone(),
                    sort: DocumentSort::Title,
                    order: SortOrder::Ascending,
                    // Bounded, but far above any realistic single category.
                    limit: 1000,
                    ..Default::default()
                })
                .await?;

            // A section with nothing in it and no populated descendants would
            // print a separator introducing blank space.
            if page.documents.is_empty() && !has_content_below(&selected, category.id) {
                continue;
            }

            entries.push(PlanEntry::Section {
                level: *level,
                title: category.name.to_string(),
                path: path.clone(),
            });

            for stored in &page.documents {
                let Some(key) = stored.current_version.renderable_key() else {
                    // Not yet converted to PDF. Named in the response so the
                    // omission is visible rather than silent.
                    skipped.push(stored.document.title.to_string());
                    continue;
                };

                let pdf = self.blobs.get(key).await?;
                entries.push(PlanEntry::Document {
                    level: level.saturating_add(1),
                    title: stored.document.title.to_string(),
                    pdf,
                });
                document_count += 1;
            }
        }

        if document_count == 0 {
            return Err(ApplicationError::Conflict {
                resource: "binder",
                reason: if skipped.is_empty() {
                    "there are no documents to bind"
                } else {
                    "none of the selected documents have a PDF available yet"
                },
            });
        }

        let built_at = self.clock.now();
        let rendered = self
            .renderer
            .render(BinderPlan {
                cover: CoverSpec {
                    title: request.title.clone(),
                    subtitle: request.subtitle.clone(),
                    organization: request.organization.clone(),
                    release_label: None,
                    built_on: built_at.format(&Rfc3339).ok().map(|stamp| {
                        // Date only; a binder cover with a timestamp on it looks
                        // like a draft.
                        stamp.split('T').next().unwrap_or(&stamp).to_owned()
                    }),
                },
                settings: request.settings,
                entries,
                built_at,
            })
            .await?;

        tracing::info!(
            document_count,
            page_count = rendered.page_count,
            skipped = skipped.len(),
            "binder generated"
        );

        Ok(GeneratedBinder {
            pdf: rendered.bytes,
            page_count: rendered.page_count,
            document_count,
            skipped,
        })
    }
}

/// Whether any selected category beneath `parent` will contribute content.
///
/// Used to decide whether an empty category is still worth a separator: a branch
/// node with populated children needs one, a genuinely empty leaf does not.
fn has_content_below(selected: &[(Category, u8, Vec<String>)], parent: CategoryId) -> bool {
    selected
        .iter()
        .any(|(candidate, _, _)| candidate.parent_id == Some(parent))
}

/// Flattens the tree into render order, with depth and ancestor path.
///
/// Depth-first in sibling order, which is the order a reader expects a printed
/// binder to follow.
fn select_categories(
    all: &[Category],
    requested: &[CategoryId],
) -> Vec<(Category, u8, Vec<String>)> {
    let mut children: HashMap<Option<CategoryId>, Vec<&Category>> = HashMap::new();
    for category in all {
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

    let mut flattened = Vec::new();
    walk(None, 0, &[], &children, &mut flattened);

    if requested.is_empty() {
        return flattened;
    }

    // A requested category brings its whole subtree with it.
    let mut keep: Vec<CategoryId> = Vec::new();
    for (category, _, _) in &flattened {
        let is_requested = requested.contains(&category.id);
        let parent_kept = category
            .parent_id
            .is_some_and(|parent| keep.contains(&parent));
        if is_requested || parent_kept {
            keep.push(category.id);
        }
    }

    // Depth is recomputed relative to the selection, so a binder of one deep
    // category starts its separators at the top level rather than indented.
    let base: HashMap<CategoryId, u8> = flattened
        .iter()
        .filter(|(category, _, _)| keep.contains(&category.id))
        .map(|(category, level, _)| (category.id, *level))
        .collect();
    let shallowest = base.values().copied().min().unwrap_or(0);

    flattened
        .into_iter()
        .filter(|(category, _, _)| keep.contains(&category.id))
        .map(|(category, level, path)| {
            let depth = level.saturating_sub(shallowest);
            // Trim the ancestor path to the part still inside the selection.
            let trimmed = path
                .into_iter()
                .skip(usize::from(shallowest))
                .collect::<Vec<_>>();
            (category, depth, trimmed)
        })
        .collect()
}

/// Recursive depth-first walk.
fn walk(
    parent: Option<CategoryId>,
    level: u8,
    path: &[String],
    children: &HashMap<Option<CategoryId>, Vec<&Category>>,
    out: &mut Vec<(Category, u8, Vec<String>)>,
) {
    let Some(siblings) = children.get(&parent) else {
        return;
    };
    for category in siblings {
        out.push(((*category).clone(), level, path.to_vec()));

        let mut nested = path.to_vec();
        nested.push(category.name.to_string());
        walk(
            Some(category.id),
            level.saturating_add(1),
            &nested,
            children,
            out,
        );
    }
}
