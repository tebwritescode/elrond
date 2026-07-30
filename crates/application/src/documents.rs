//! Document ingestion and retrieval use cases.

use std::sync::Arc;

use elrond_domain::{
    BlobClass, CategoryId, DocumentId, DocumentTitle, DocumentVersion, DocumentVersionId,
    LifecycleState, MediaType, OriginalFilename, Role, StorageKey, TagLabel, VersionNumber,
};
use time::OffsetDateTime;

use crate::auth::Authenticated;
use crate::categories::CategoryService;
use crate::error::{ApplicationError, ApplicationResult};
use crate::ports::{
    BlobStore, Clock, ContentInspector, DocumentFilter, DocumentPage, DocumentRepository,
    IndexedDocument, NewDocument, NewVersion, SearchIndex, StoredDocument, TagRepository,
};

/// An upload as it arrives from the transport.
#[derive(Debug, Clone)]
pub struct UploadRequest {
    /// Filename as supplied by the client. Sanitized before use, and never used
    /// to build a path.
    pub filename: String,
    /// Content type the client claimed, if any. Advisory only.
    pub declared_media_type: Option<String>,
    /// The file's bytes.
    pub bytes: Vec<u8>,
    /// Category to file it under. Defaults to "Unfiled" when absent.
    pub category_id: Option<CategoryId>,
    /// Title. Derived from the filename when absent.
    pub title: Option<String>,
    /// Tags to attach.
    pub tags: Vec<String>,
    /// Folder-relative provenance, set by the ZIP importer.
    pub source_path: Option<String>,
}

/// The outcome of an upload.
#[derive(Debug, Clone)]
pub struct UploadOutcome {
    /// The stored document.
    pub document: StoredDocument,
    /// Whether the bytes were already present and were reused.
    pub deduplicated: bool,
    /// An existing document with identical content, if there is one.
    ///
    /// Reported rather than treated as an error: the same file legitimately
    /// belongs in more than one place, but the person uploading it should be told.
    pub duplicate_of: Option<DocumentId>,
}

/// Content ready to send to a client.
#[derive(Debug, Clone)]
pub struct DocumentContent {
    /// The bytes.
    pub bytes: Vec<u8>,
    /// Filename to offer.
    pub filename: OriginalFilename,
    /// Type of the returned bytes.
    pub media_type: MediaType,
    /// Checksum, for an `ETag`.
    pub checksum: elrond_domain::Sha256Checksum,
}

/// A document with its full version history.
#[derive(Debug, Clone)]
pub struct DocumentDetail {
    /// The document, current version, category, and tags.
    pub document: StoredDocument,
    /// Every version, newest first.
    pub versions: Vec<DocumentVersion>,
}

/// Document use cases.
#[derive(Clone)]
pub struct DocumentService {
    documents: Arc<dyn DocumentRepository>,
    tags: Arc<dyn TagRepository>,
    blobs: Arc<dyn BlobStore>,
    inspector: Arc<dyn ContentInspector>,
    search: Arc<dyn SearchIndex>,
    categories: CategoryService,
    clock: Arc<dyn Clock>,
}

impl DocumentService {
    /// Wires the use cases to their adapters.
    pub fn new(
        documents: Arc<dyn DocumentRepository>,
        tags: Arc<dyn TagRepository>,
        blobs: Arc<dyn BlobStore>,
        inspector: Arc<dyn ContentInspector>,
        search: Arc<dyn SearchIndex>,
        categories: CategoryService,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            documents,
            tags,
            blobs,
            inspector,
            search,
            categories,
            clock,
        }
    }

    /// Ingests a new document.
    pub async fn upload(
        &self,
        actor: &Authenticated,
        request: UploadRequest,
    ) -> ApplicationResult<UploadOutcome> {
        actor.require_role(Role::Editor)?;

        if request.bytes.is_empty() {
            return Err(ApplicationError::Domain(
                elrond_domain::DomainError::Required { field: "file" },
            ));
        }

        let filename = OriginalFilename::parse(&request.filename)?;
        let media_type = resolve_media_type(
            self.inspector.as_ref(),
            &request.bytes,
            &filename,
            request.declared_media_type.as_deref(),
        )?;

        let title = match &request.title {
            Some(raw) => DocumentTitle::parse(raw)?,
            None => DocumentTitle::from_filename(&filename)?,
        };

        let category_id = match request.category_id {
            Some(id) => self.categories.require(id).await?.id,
            None => self.categories.unfiled().await?.id,
        };

        let labels = parse_tags(&request.tags)?;

        let stored = self.blobs.put(BlobClass::Original, request.bytes).await?;

        // Looked up after the write but before the document row exists, so the
        // answer describes existing library content rather than this upload.
        let duplicate_of = self.documents.find_by_checksum(stored.checksum).await?;

        let now = self.clock.now();
        let document_id = DocumentId::new();
        let byte_size = stored.byte_size;

        let created = self
            .documents
            .insert(NewDocument {
                id: document_id,
                title,
                category_id,
                // Everything starts as a draft. Nothing reaches viewers or a binder
                // until it has been through review.
                lifecycle: LifecycleState::Draft,
                source_path: request.source_path,
                created_by: actor.user.id,
                created_at: now,
                version: NewVersion {
                    id: DocumentVersionId::new(),
                    number: VersionNumber::FIRST,
                    original_filename: filename,
                    media_type,
                    byte_size,
                    checksum: stored.checksum,
                    storage_key: stored.key.clone(),
                    note: None,
                    created_by: actor.user.id,
                    created_at: now,
                },
            })
            .await?;

        let document = self.apply_tags(created, &labels, now).await?;
        self.reindex(&document).await?;

        tracing::info!(
            document_id = %document.document.id,
            media_type = media_type.mime(),
            byte_size,
            deduplicated = stored.deduplicated,
            "document ingested"
        );

        Ok(UploadOutcome {
            document,
            deduplicated: stored.deduplicated,
            duplicate_of,
        })
    }

    /// Appends a new version to an existing document.
    pub async fn add_version(
        &self,
        actor: &Authenticated,
        document_id: DocumentId,
        request: UploadRequest,
        note: Option<String>,
    ) -> ApplicationResult<DocumentVersion> {
        actor.require_role(Role::Editor)?;

        let existing =
            self.documents
                .find_by_id(document_id)
                .await?
                .ok_or(ApplicationError::NotFound {
                    resource: "document",
                })?;

        // A published version is pinned by any binder release that references it,
        // so its content can never be replaced. Appending a version is the
        // supported way to publish a correction, and it leaves the old one intact.
        if existing.document.lifecycle == LifecycleState::InReview {
            return Err(ApplicationError::Conflict {
                resource: "document",
                reason: "the document is in review; content is frozen until it is approved or returned",
            });
        }

        let filename = OriginalFilename::parse(&request.filename)?;
        let media_type = resolve_media_type(
            self.inspector.as_ref(),
            &request.bytes,
            &filename,
            request.declared_media_type.as_deref(),
        )?;

        let stored = self.blobs.put(BlobClass::Original, request.bytes).await?;
        let now = self.clock.now();

        let version = self
            .documents
            .append_version(
                document_id,
                NewVersion {
                    id: DocumentVersionId::new(),
                    number: existing.current_version.number.next(),
                    original_filename: filename,
                    media_type,
                    byte_size: stored.byte_size,
                    checksum: stored.checksum,
                    storage_key: stored.key,
                    note,
                    created_by: actor.user.id,
                    created_at: now,
                },
            )
            .await?;

        if let Some(refreshed) = self.documents.find_by_id(document_id).await? {
            self.reindex(&refreshed).await?;
        }

        tracing::info!(
            document_id = %document_id,
            version = %version.number,
            "version appended"
        );
        Ok(version)
    }

    /// Lists documents, optionally narrowed by a search query.
    pub async fn list(
        &self,
        actor: &Authenticated,
        mut filter: DocumentFilter,
        query: Option<&str>,
    ) -> ApplicationResult<DocumentPage> {
        // A viewer may only see published material, whatever it asks for.
        if !actor.user.role.satisfies(Role::Reviewer) {
            filter.lifecycles = vec![LifecycleState::Published];
        }

        // Expanding here rather than in SQL keeps the recursive walk in one place
        // and out of every query that filters by category.
        if let Some(category_id) = filter.category_id
            && filter.include_descendants
        {
            let subtree = self.categories.subtree_ids(category_id).await?;
            if subtree.len() > 1 {
                filter.category_ids = subtree;
                filter.category_id = None;
            }
        }

        if let Some(query) = query.map(str::trim).filter(|q| !q.is_empty()) {
            // Over-fetch relative to the page size: the search result set is then
            // narrowed by the other filters, so asking for exactly one page would
            // under-fill it.
            let outcome = self
                .search
                .search(query, filter.limit.saturating_mul(10).max(200))
                .await?;
            if outcome.document_ids.is_empty() {
                return Ok(DocumentPage {
                    documents: Vec::new(),
                    total: 0,
                });
            }
            filter.ids = Some(outcome.document_ids);
        }

        Ok(self.documents.list(&filter).await?)
    }

    /// Reads one document with its version history.
    pub async fn detail(
        &self,
        actor: &Authenticated,
        id: DocumentId,
    ) -> ApplicationResult<DocumentDetail> {
        let document = self
            .documents
            .find_by_id(id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "document",
            })?;
        Self::require_visible(actor, &document)?;

        let versions = self.documents.list_versions(id).await?;
        Ok(DocumentDetail { document, versions })
    }

    /// Reads the original bytes of a version.
    pub async fn original(
        &self,
        actor: &Authenticated,
        version_id: DocumentVersionId,
    ) -> ApplicationResult<DocumentContent> {
        let version =
            self.documents
                .find_version(version_id)
                .await?
                .ok_or(ApplicationError::NotFound {
                    resource: "version",
                })?;

        let document = self
            .documents
            .find_by_id(version.document_id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "document",
            })?;
        Self::require_visible(actor, &document)?;

        Ok(DocumentContent {
            bytes: self.blobs.get(&version.storage_key).await?,
            filename: version.original_filename.clone(),
            media_type: version.media_type,
            checksum: version.checksum,
        })
    }

    /// Reads the PDF for a version: the original if it is already a PDF, otherwise
    /// its generated derivative.
    pub async fn renderable_pdf(
        &self,
        actor: &Authenticated,
        version_id: DocumentVersionId,
    ) -> ApplicationResult<DocumentContent> {
        let version =
            self.documents
                .find_version(version_id)
                .await?
                .ok_or(ApplicationError::NotFound {
                    resource: "version",
                })?;

        let document = self
            .documents
            .find_by_id(version.document_id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "document",
            })?;
        Self::require_visible(actor, &document)?;

        let key: &StorageKey = version.renderable_key().ok_or(ApplicationError::Conflict {
            resource: "version",
            reason: "the PDF copy of this file has not been generated yet",
        })?;

        Ok(DocumentContent {
            bytes: self.blobs.get(key).await?,
            filename: version.original_filename.clone(),
            media_type: MediaType::Pdf,
            checksum: version.derivative_checksum.unwrap_or(version.checksum),
        })
    }

    /// Updates title, category, review date, and tags.
    pub async fn update_metadata(
        &self,
        actor: &Authenticated,
        id: DocumentId,
        title: &str,
        category_id: CategoryId,
        review_due_at: Option<OffsetDateTime>,
        tags: &[String],
    ) -> ApplicationResult<StoredDocument> {
        actor.require_role(Role::Editor)?;

        let existing = self
            .documents
            .find_by_id(id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "document",
            })?;

        // Metadata stays editable after publication on purpose: retitling or
        // refiling a document does not change the bytes a binder release pins.
        let title = DocumentTitle::parse(title)?;
        let category_id = self.categories.require(category_id).await?.id;
        let labels = parse_tags(tags)?;
        let now = self.clock.now();

        self.documents
            .update_metadata(id, &title, category_id, review_due_at, now)
            .await?;

        let refreshed = self
            .documents
            .find_by_id(id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "document",
            })?;
        let updated = self.apply_tags(refreshed, &labels, now).await?;
        self.reindex(&updated).await?;

        tracing::info!(document_id = %id, previous = %existing.document.title, "metadata updated");
        Ok(updated)
    }

    /// Moves a document through its lifecycle.
    pub async fn transition(
        &self,
        actor: &Authenticated,
        id: DocumentId,
        next: LifecycleState,
    ) -> ApplicationResult<StoredDocument> {
        let existing = self
            .documents
            .find_by_id(id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "document",
            })?;

        // Submitting for review and archiving are authoring actions; approving is
        // a reviewer's.
        let required = match next {
            LifecycleState::Published => Role::Reviewer,
            _ => Role::Editor,
        };
        actor.require_role(required)?;

        // The state machine, not this function, decides what is legal.
        let updated = existing.document.clone().transition_to(next)?;
        self.documents
            .set_lifecycle(id, updated.lifecycle, self.clock.now())
            .await?;

        self.documents
            .find_by_id(id)
            .await?
            .ok_or(ApplicationError::NotFound {
                resource: "document",
            })
    }

    /// Rebuilds the entire search index.
    pub async fn rebuild_search_index(&self, actor: &Authenticated) -> ApplicationResult<u64> {
        actor.require_role(Role::Admin)?;

        // Paged rather than loaded whole: an administrative rebuild must not need
        // the library to fit in memory.
        let mut indexed = Vec::new();
        let mut offset = 0_u32;
        loop {
            let page = self
                .documents
                .list(&DocumentFilter {
                    limit: 500,
                    offset,
                    ..Default::default()
                })
                .await?;
            if page.documents.is_empty() {
                break;
            }
            // The page size is a u32 to begin with, so the count cannot exceed it;
            // saturating at u32::MAX would end the loop rather than misbehave.
            offset = offset.saturating_add(u32::try_from(page.documents.len()).unwrap_or(u32::MAX));
            indexed.extend(page.documents.iter().map(to_indexed));
            if u64::from(offset) >= page.total {
                break;
            }
        }

        let count = indexed.len() as u64;
        self.search.rebuild(indexed).await?;
        tracing::info!(count, "search index rebuilt");
        Ok(count)
    }

    /// Refuses access to a document the caller may not see.
    fn require_visible(actor: &Authenticated, document: &StoredDocument) -> ApplicationResult<()> {
        if actor.user.role.satisfies(Role::Reviewer) || document.document.is_visible_to_viewers() {
            Ok(())
        } else {
            // Not-found rather than forbidden: telling a viewer that a draft exists
            // is itself a disclosure.
            Err(ApplicationError::NotFound {
                resource: "document",
            })
        }
    }

    /// Resolves labels to tags and attaches exactly that set.
    async fn apply_tags(
        &self,
        document: StoredDocument,
        labels: &[TagLabel],
        at: OffsetDateTime,
    ) -> ApplicationResult<StoredDocument> {
        let tags = self.tags.ensure(labels, at).await?;
        let ids: Vec<_> = tags.iter().map(|tag| tag.id).collect();
        self.tags
            .set_for_document(document.document.id, &ids)
            .await?;

        Ok(StoredDocument { tags, ..document })
    }

    /// Refreshes a document's search entry.
    async fn reindex(&self, document: &StoredDocument) -> ApplicationResult<()> {
        self.search.index(to_indexed(document)).await?;
        Ok(())
    }
}

/// Projects a stored document into its indexable text.
fn to_indexed(document: &StoredDocument) -> IndexedDocument {
    IndexedDocument {
        document_id: document.document.id,
        title: document.document.title.to_string(),
        filename: document.current_version.original_filename.to_string(),
        tags: document
            .tags
            .iter()
            .map(|tag| tag.label.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        // Body text arrives later, once extraction or OCR has run.
        content: String::new(),
    }
}

/// Validates and deduplicates tag labels.
fn parse_tags(raw: &[String]) -> ApplicationResult<Vec<TagLabel>> {
    let labels = raw
        .iter()
        .map(|value| TagLabel::parse(value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(elrond_domain::tag::normalize_tag_set(&labels)?)
}

/// Decides a file's type from its contents, its name, and what the client claimed.
///
/// Content wins. A `.pdf` that is actually a ZIP archive is either a mistake or an
/// attack, and storing it as a PDF would hand a broken file to the PDF pipeline.
/// The extension is only consulted for formats that have no magic bytes.
fn resolve_media_type(
    inspector: &dyn ContentInspector,
    bytes: &[u8],
    filename: &OriginalFilename,
    declared: Option<&str>,
) -> ApplicationResult<MediaType> {
    let detected = inspector.detect(bytes);
    let implied = filename.implied_media_type();

    if let Some(detected) = detected {
        // A disagreement is reported rather than quietly resolved, because the
        // person uploading needs to know the file is not what its name says.
        if let Some(implied) = implied
            && implied != detected
            && implied.kind() != detected.kind()
        {
            return Err(ApplicationError::Conflict {
                resource: "file",
                reason: "the file's contents do not match its extension",
            });
        }
        return Ok(detected);
    }

    // Plain text, Markdown, and CSV are indistinguishable by content, so the
    // extension is the only signal available.
    if let Some(implied) = implied {
        return Ok(implied);
    }

    // The client's claim is the last resort: it is trivially forged, so it is only
    // trusted when nothing else identifies the file at all.
    if let Some(declared) = declared.and_then(MediaType::from_mime) {
        return Ok(declared);
    }

    Err(ApplicationError::Domain(
        elrond_domain::DomainError::Invalid {
            field: "file",
            reason: "unsupported_file_type",
        },
    ))
}
