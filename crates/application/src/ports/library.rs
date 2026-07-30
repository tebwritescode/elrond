//! Ports for the document library: categories, documents, tags, and search.

use async_trait::async_trait;
use elrond_domain::{
    Category, CategoryId, CategoryName, Document, DocumentId, DocumentTitle, DocumentVersion,
    DocumentVersionId, LifecycleState, MediaType, OriginalFilename, Sha256Checksum, StorageKey,
    Tag, TagId, TagLabel, UserId, VersionNumber,
};
use time::OffsetDateTime;

use super::RepositoryError;

// ------------------------------------------------------------------ categories

/// Fields required to create a category.
#[derive(Debug, Clone)]
pub struct NewCategory {
    /// Stable identifier, chosen by the caller.
    pub id: CategoryId,
    /// Parent, or `None` for a root category.
    pub parent_id: Option<CategoryId>,
    /// Display name.
    pub name: CategoryName,
    /// Ordering among siblings.
    pub position: i64,
    /// Creation timestamp.
    pub created_at: OffsetDateTime,
}

/// Category persistence.
#[async_trait]
pub trait CategoryRepository: Send + Sync + 'static {
    /// Creates a category.
    async fn insert(&self, new_category: NewCategory) -> Result<Category, RepositoryError>;

    /// Reads every category.
    ///
    /// The whole tree is loaded rather than queried recursively: cycle and depth
    /// checks are properties of the entire tree, and a document library's category
    /// count is measured in hundreds, not millions.
    async fn list_all(&self) -> Result<Vec<Category>, RepositoryError>;

    /// Finds a category by identifier.
    async fn find_by_id(&self, id: CategoryId) -> Result<Option<Category>, RepositoryError>;

    /// Finds a child of `parent` whose name matches case-insensitively.
    ///
    /// Used by the ZIP importer to reuse an existing folder rather than creating a
    /// near-duplicate sibling.
    async fn find_child_by_name(
        &self,
        parent_id: Option<CategoryId>,
        name: &CategoryName,
    ) -> Result<Option<Category>, RepositoryError>;

    /// Renames a category.
    async fn rename(
        &self,
        id: CategoryId,
        name: &CategoryName,
        at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    /// Reparents a category. Cycle and depth checks happen before this is called.
    async fn set_parent(
        &self,
        id: CategoryId,
        parent_id: Option<CategoryId>,
        at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    /// Deletes a category. Fails if documents or child categories remain.
    async fn delete(&self, id: CategoryId) -> Result<(), RepositoryError>;

    /// Counts documents filed directly in each category.
    async fn document_counts(&self) -> Result<Vec<(CategoryId, u64)>, RepositoryError>;
}

// ------------------------------------------------------------------------ tags

/// Tag persistence.
#[async_trait]
pub trait TagRepository: Send + Sync + 'static {
    /// Returns the tags for these labels, creating any that do not exist.
    ///
    /// Idempotent by normalized key, so concurrent uploads using the same new tag
    /// converge on one row rather than racing to create two.
    async fn ensure(
        &self,
        labels: &[TagLabel],
        at: OffsetDateTime,
    ) -> Result<Vec<Tag>, RepositoryError>;

    /// Reads every tag, with how many documents carry it.
    async fn list_with_counts(&self) -> Result<Vec<(Tag, u64)>, RepositoryError>;

    /// Replaces a document's tags wholesale.
    async fn set_for_document(
        &self,
        document_id: DocumentId,
        tag_ids: &[TagId],
    ) -> Result<(), RepositoryError>;

    /// Reads the tags on one document.
    async fn list_for_document(&self, document_id: DocumentId)
    -> Result<Vec<Tag>, RepositoryError>;

    /// Reads the tags for several documents at once.
    ///
    /// Exists so a library listing does not issue one query per row.
    async fn list_for_documents(
        &self,
        document_ids: &[DocumentId],
    ) -> Result<Vec<(DocumentId, Tag)>, RepositoryError>;

    /// Removes tags no document carries. Returns how many were removed.
    async fn prune_unused(&self) -> Result<u64, RepositoryError>;
}

// ------------------------------------------------------------------- documents

/// Fields required to create a document together with its first version.
#[derive(Debug, Clone)]
pub struct NewDocument {
    /// Stable identifier.
    pub id: DocumentId,
    /// Title.
    pub title: DocumentTitle,
    /// Primary category.
    pub category_id: CategoryId,
    /// Initial lifecycle state.
    pub lifecycle: LifecycleState,
    /// Folder-relative provenance, for bulk imports.
    pub source_path: Option<String>,
    /// Account creating the document.
    pub created_by: UserId,
    /// Creation timestamp.
    pub created_at: OffsetDateTime,
    /// The first version's content.
    pub version: NewVersion,
}

/// Fields required to append a version.
#[derive(Debug, Clone)]
pub struct NewVersion {
    /// Stable identifier. This is what a binder release pins.
    pub id: DocumentVersionId,
    /// Sequence number within the document.
    pub number: VersionNumber,
    /// Filename as uploaded, for display only.
    pub original_filename: OriginalFilename,
    /// Type of the original.
    pub media_type: MediaType,
    /// Size of the original.
    pub byte_size: u64,
    /// Checksum of the original.
    pub checksum: Sha256Checksum,
    /// Where the original is stored.
    pub storage_key: StorageKey,
    /// Optional note describing what changed.
    pub note: Option<String>,
    /// Account creating the version.
    pub created_by: UserId,
    /// Creation timestamp.
    pub created_at: OffsetDateTime,
}

/// A document with the details a listing needs, gathered in one read.
#[derive(Debug, Clone)]
pub struct StoredDocument {
    /// The document.
    pub document: Document,
    /// Its current version.
    pub current_version: DocumentVersion,
    /// Its category's name, so a listing need not resolve the tree per row.
    pub category_name: CategoryName,
    /// Its tags.
    pub tags: Vec<Tag>,
}

/// Which column a listing is ordered by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentSort {
    /// By title, alphabetically.
    Title,
    /// By when the document was created.
    Created,
    /// By when the document was last changed.
    Updated,
    /// By size of the current version.
    Size,
    /// By search relevance. Only meaningful with a query.
    Relevance,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    /// Smallest or earliest first.
    Ascending,
    /// Largest or latest first.
    Descending,
}

/// Which documents a listing should return.
#[derive(Debug, Clone)]
pub struct DocumentFilter {
    /// Restrict to one category.
    pub category_id: Option<CategoryId>,
    /// Restrict to any of these categories.
    ///
    /// Set by the use case once a single-category filter has been expanded across
    /// its descendants, so the repository never has to walk the tree itself.
    pub category_ids: Vec<CategoryId>,
    /// Whether a single-category filter should include nested categories.
    pub include_descendants: bool,
    /// Restrict to these lifecycle states. Empty means all.
    pub lifecycles: Vec<LifecycleState>,
    /// Require every one of these tags.
    pub tag_ids: Vec<TagId>,
    /// Restrict to documents whose ids appear here, in this order.
    ///
    /// How a search result set is intersected with the other filters while
    /// preserving relevance ranking.
    pub ids: Option<Vec<DocumentId>>,
    /// Column to sort by.
    pub sort: DocumentSort,
    /// Sort direction.
    pub order: SortOrder,
    /// Maximum rows to return.
    pub limit: u32,
    /// Rows to skip.
    pub offset: u32,
}

impl Default for DocumentFilter {
    fn default() -> Self {
        Self {
            category_id: None,
            category_ids: Vec::new(),
            include_descendants: true,
            lifecycles: Vec::new(),
            tag_ids: Vec::new(),
            ids: None,
            sort: DocumentSort::Updated,
            order: SortOrder::Descending,
            limit: 50,
            offset: 0,
        }
    }
}

/// One page of a listing.
#[derive(Debug, Clone)]
pub struct DocumentPage {
    /// The rows on this page.
    pub documents: Vec<StoredDocument>,
    /// How many rows match the filter in total.
    ///
    /// Returned so the interface can render a real pager rather than guessing
    /// whether another page exists.
    pub total: u64,
}

/// Document persistence.
#[async_trait]
pub trait DocumentRepository: Send + Sync + 'static {
    /// Creates a document and its first version in one transaction.
    ///
    /// Atomic because a document with no version violates the schema's own
    /// consistency check, and a half-written import must not leave one behind.
    async fn insert(&self, new_document: NewDocument) -> Result<StoredDocument, RepositoryError>;

    /// Appends a version and makes it current.
    async fn append_version(
        &self,
        document_id: DocumentId,
        version: NewVersion,
    ) -> Result<DocumentVersion, RepositoryError>;

    /// Reads one document with its current version, category, and tags.
    async fn find_by_id(&self, id: DocumentId) -> Result<Option<StoredDocument>, RepositoryError>;

    /// Reads a page of documents.
    async fn list(&self, filter: &DocumentFilter) -> Result<DocumentPage, RepositoryError>;

    /// Reads one version.
    async fn find_version(
        &self,
        id: DocumentVersionId,
    ) -> Result<Option<DocumentVersion>, RepositoryError>;

    /// Reads a document's versions, newest first.
    async fn list_versions(
        &self,
        document_id: DocumentId,
    ) -> Result<Vec<DocumentVersion>, RepositoryError>;

    /// Updates title, category, and review date.
    async fn update_metadata(
        &self,
        id: DocumentId,
        title: &DocumentTitle,
        category_id: CategoryId,
        review_due_at: Option<OffsetDateTime>,
        at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    /// Records a lifecycle change.
    async fn set_lifecycle(
        &self,
        id: DocumentId,
        lifecycle: LifecycleState,
        at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    /// Records a generated PDF against a version. Write-once.
    async fn set_derivative(
        &self,
        version_id: DocumentVersionId,
        checksum: Sha256Checksum,
        key: &StorageKey,
    ) -> Result<(), RepositoryError>;

    /// Finds an existing version with this content checksum.
    ///
    /// Used to report a duplicate upload before storing anything.
    async fn find_by_checksum(
        &self,
        checksum: Sha256Checksum,
    ) -> Result<Option<DocumentId>, RepositoryError>;

    /// Deletes a document, its versions, and its tag links.
    async fn delete(&self, id: DocumentId) -> Result<(), RepositoryError>;
}

// ---------------------------------------------------------------------- search

/// The text of one document, as the index sees it.
#[derive(Debug, Clone)]
pub struct IndexedDocument {
    /// Which document this describes.
    pub document_id: DocumentId,
    /// Title.
    pub title: String,
    /// Current version's filename.
    pub filename: String,
    /// Tag labels, space separated.
    pub tags: String,
    /// Extracted body text, empty until extraction or OCR has run.
    pub content: String,
}

/// A ranked search result set.
#[derive(Debug, Clone, Default)]
pub struct SearchOutcome {
    /// Matching document ids, most relevant first.
    pub document_ids: Vec<DocumentId>,
}

/// Full-text search.
#[async_trait]
pub trait SearchIndex: Send + Sync + 'static {
    /// Adds or replaces a document's entry.
    async fn index(&self, document: IndexedDocument) -> Result<(), RepositoryError>;

    /// Removes a document's entry.
    async fn remove(&self, document_id: DocumentId) -> Result<(), RepositoryError>;

    /// Runs a query.
    ///
    /// The query is user input, so an implementation must neutralize the FTS5
    /// query syntax rather than passing it through: an unbalanced quote or a bare
    /// `NEAR` would otherwise surface as a syntax error the user cannot act on.
    async fn search(&self, query: &str, limit: u32) -> Result<SearchOutcome, RepositoryError>;

    /// Replaces the entire index. Used by an administrative rebuild.
    async fn rebuild(&self, documents: Vec<IndexedDocument>) -> Result<(), RepositoryError>;
}
