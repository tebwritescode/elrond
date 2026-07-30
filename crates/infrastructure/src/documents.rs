//! SQLite-backed document and version storage.

use std::collections::HashMap;

use async_trait::async_trait;
use elrond_application::ports::{
    DocumentFilter, DocumentPage, DocumentRepository, DocumentSort, NewDocument, NewVersion,
    RepositoryError, SortOrder, StoredDocument,
};
use elrond_domain::{
    CategoryId, CategoryName, Document, DocumentId, DocumentTitle, DocumentVersion,
    DocumentVersionId, LifecycleState, MediaType, OriginalFilename, Sha256Checksum, StorageKey,
    Tag, TagId, TagLabel, UserId, VersionNumber,
};
use sqlx::sqlite::{SqliteArguments, SqliteRow};
use sqlx::{Arguments, Pool, Row, Sqlite, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::{Database, classify};

/// Columns selected whenever a document row is read.
const DOCUMENT_COLUMNS: &str = "d.id, d.title, d.category_id, d.lifecycle, d.current_version_id, \
     d.version_count, d.source_path, d.review_due_at, d.created_by, d.created_at, d.updated_at";

/// Version columns, aliased with a `v_` prefix.
///
/// The alias is required, not cosmetic: `documents` and `document_versions` both
/// have `id`, `created_by`, and `created_at`, and SQLite resolves a duplicate
/// column name in a joined result to the *first* match. Without the prefix a
/// version would silently be built from the document's identifier.
const VERSION_COLUMNS: &str = "v.id AS v_id, v.document_id AS v_document_id, \
     v.number AS v_number, v.original_filename AS v_original_filename, \
     v.media_type AS v_media_type, v.byte_size AS v_byte_size, v.checksum AS v_checksum, \
     v.storage_key AS v_storage_key, v.derivative_checksum AS v_derivative_checksum, \
     v.derivative_key AS v_derivative_key, v.created_by AS v_created_by, v.note AS v_note, \
     v.created_at AS v_created_at";

/// Prefix used by [`VERSION_COLUMNS`].
const VERSION_PREFIX: &str = "v_";

/// No prefix, for queries that read `document_versions` on its own.
const NO_PREFIX: &str = "";

/// Documents stored in SQLite.
#[derive(Debug, Clone)]
pub struct SqliteDocumentRepository {
    pool: Pool<Sqlite>,
}

impl SqliteDocumentRepository {
    /// Binds the repository to a connected database.
    pub fn new(database: &Database) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }

    /// Records a version row inside an open transaction.
    async fn insert_version(
        transaction: &mut Transaction<'_, Sqlite>,
        document_id: DocumentId,
        version: &NewVersion,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO document_versions
                 (id, document_id, number, original_filename, media_type, byte_size,
                  checksum, storage_key, created_by, note, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )
        .bind(version.id.into_uuid())
        .bind(document_id.into_uuid())
        .bind(i64::from(version.number.get()))
        .bind(version.original_filename.as_str())
        .bind(version.media_type.mime())
        .bind(version.byte_size.cast_signed())
        .bind(version.checksum.to_hex())
        .bind(version.storage_key.as_str())
        .bind(version.created_by.into_uuid())
        .bind(version.note.as_deref())
        .bind(version.created_at)
        .execute(&mut **transaction)
        .await
        .map_err(|error| classify(error, "version", "number"))?;

        // Deduplication means several versions can point at one blob, so the bytes
        // are reference counted rather than owned by whichever version wrote them.
        sqlx::query(
            "INSERT INTO blobs (storage_key, checksum, byte_size, reference_count, created_at)
             VALUES (?1, ?2, ?3, 1, ?4)
             ON CONFLICT (storage_key) DO UPDATE SET reference_count = reference_count + 1",
        )
        .bind(version.storage_key.as_str())
        .bind(version.checksum.to_hex())
        .bind(version.byte_size.cast_signed())
        .bind(version.created_at)
        .execute(&mut **transaction)
        .await
        .map_err(RepositoryError::backend)?;

        Ok(())
    }

    /// Reads one document with its current version, category, and tags.
    async fn load_stored(&self, id: DocumentId) -> Result<Option<StoredDocument>, RepositoryError> {
        let sql = format!(
            "SELECT {DOCUMENT_COLUMNS}, {VERSION_COLUMNS}, c.name AS category_name
             FROM documents d
             JOIN document_versions v ON v.id = d.current_version_id
             JOIN categories c ON c.id = d.category_id
             WHERE d.id = ?1"
        );
        let row = sqlx::query(&sql)
            .bind(id.into_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let mut stored = map_stored(&row)?;
        stored.tags = self.load_tags(&[id]).await?.remove(&id).unwrap_or_default();
        Ok(Some(stored))
    }

    /// Reads the tags for several documents in one query.
    async fn load_tags(
        &self,
        ids: &[DocumentId],
    ) -> Result<HashMap<DocumentId, Vec<Tag>>, RepositoryError> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        // One query with an expanded IN list, so a page of rows costs one round trip
        // rather than one per row.
        let placeholders = (1..=ids.len())
            .map(|index| format!("?{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT dt.document_id, t.id, t.label, t.created_at
             FROM document_tags dt
             JOIN tags t ON t.id = dt.tag_id
             WHERE dt.document_id IN ({placeholders})
             ORDER BY t.label_key"
        );

        let mut query = sqlx::query(&sql);
        for id in ids {
            query = query.bind(id.into_uuid());
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;

        let mut grouped: HashMap<DocumentId, Vec<Tag>> = HashMap::new();
        for row in &rows {
            let document_id: Uuid = row
                .try_get("document_id")
                .map_err(RepositoryError::backend)?;
            let tag_id: Uuid = row.try_get("id").map_err(RepositoryError::backend)?;
            let label: String = row.try_get("label").map_err(RepositoryError::backend)?;
            let created_at: OffsetDateTime = row
                .try_get("created_at")
                .map_err(RepositoryError::backend)?;

            grouped
                .entry(DocumentId::from_uuid(document_id))
                .or_default()
                .push(Tag {
                    id: TagId::from_uuid(tag_id),
                    label: TagLabel::parse(&label).map_err(|_| corrupt("tags.label"))?,
                    created_at,
                });
        }
        Ok(grouped)
    }
}

#[async_trait]
impl DocumentRepository for SqliteDocumentRepository {
    async fn insert(&self, new_document: NewDocument) -> Result<StoredDocument, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(RepositoryError::backend)?;

        // The document row is written with no version, then the version, then the
        // pointer. Doing it in one transaction is what keeps the schema's
        // "version_count implies current_version_id" check satisfied from the
        // outside, and stops a failed import leaving a document with no content.
        sqlx::query(
            "INSERT INTO documents
                 (id, title, category_id, lifecycle, current_version_id, version_count,
                  source_path, created_by, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, 0, ?5, ?6, ?7, ?7)",
        )
        .bind(new_document.id.into_uuid())
        .bind(new_document.title.as_str())
        .bind(new_document.category_id.into_uuid())
        .bind(new_document.lifecycle.as_str())
        .bind(new_document.source_path.as_deref())
        .bind(new_document.created_by.into_uuid())
        .bind(new_document.created_at)
        .execute(&mut *transaction)
        .await
        .map_err(|error| classify(error, "document", "id"))?;

        Self::insert_version(&mut transaction, new_document.id, &new_document.version).await?;

        sqlx::query(
            "UPDATE documents SET current_version_id = ?2, version_count = 1 WHERE id = ?1",
        )
        .bind(new_document.id.into_uuid())
        .bind(new_document.version.id.into_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::backend)?;

        transaction
            .commit()
            .await
            .map_err(RepositoryError::backend)?;

        self.load_stored(new_document.id)
            .await?
            .ok_or_else(|| corrupt("documents"))
    }

    async fn append_version(
        &self,
        document_id: DocumentId,
        version: NewVersion,
    ) -> Result<DocumentVersion, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(RepositoryError::backend)?;

        Self::insert_version(&mut transaction, document_id, &version).await?;

        sqlx::query(
            "UPDATE documents
             SET current_version_id = ?2, version_count = version_count + 1, updated_at = ?3
             WHERE id = ?1",
        )
        .bind(document_id.into_uuid())
        .bind(version.id.into_uuid())
        .bind(version.created_at)
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::backend)?;

        transaction
            .commit()
            .await
            .map_err(RepositoryError::backend)?;

        self.find_version(version.id)
            .await?
            .ok_or_else(|| corrupt("document_versions"))
    }

    async fn find_by_id(&self, id: DocumentId) -> Result<Option<StoredDocument>, RepositoryError> {
        self.load_stored(id).await
    }

    async fn list(&self, filter: &DocumentFilter) -> Result<DocumentPage, RepositoryError> {
        let (where_clause, mut arguments) = build_conditions(filter)?;

        // With a relevance-ordered id list the page cannot be sliced in SQL: the
        // ordering lives in the caller's vector, not in any column. The matched rows
        // are fetched, reordered, then sliced.
        let relevance_ordered = filter.ids.is_some() && filter.sort == DocumentSort::Relevance;

        let order = if relevance_ordered {
            // Stable, arbitrary; the real ordering is applied below.
            "d.id".to_owned()
        } else {
            let column = match filter.sort {
                DocumentSort::Title => "d.title",
                DocumentSort::Created => "d.created_at",
                DocumentSort::Size => "v.byte_size",
                // Relevance without a query has no meaning, so it falls back to
                // recency rather than returning an arbitrary order.
                DocumentSort::Updated | DocumentSort::Relevance => "d.updated_at",
            };
            let direction = match filter.order {
                SortOrder::Ascending => "ASC",
                SortOrder::Descending => "DESC",
            };
            // The id tiebreaker makes paging deterministic when the sort column ties.
            format!("{column} {direction}, d.id {direction}")
        };

        let mut sql = format!(
            "SELECT {DOCUMENT_COLUMNS}, {VERSION_COLUMNS}, c.name AS category_name
             FROM documents d
             JOIN document_versions v ON v.id = d.current_version_id
             JOIN categories c ON c.id = d.category_id
             {where_clause}
             ORDER BY {order}"
        );
        if !relevance_ordered {
            sql.push_str(" LIMIT ? OFFSET ?");
            arguments
                .add(i64::from(filter.limit))
                .map_err(bind_failed)?;
            arguments
                .add(i64::from(filter.offset))
                .map_err(bind_failed)?;
        }

        let rows = sqlx::query_with(&sql, arguments)
            .fetch_all(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;

        let mut documents: Vec<StoredDocument> =
            rows.iter().map(map_stored).collect::<Result<_, _>>()?;

        let total = if relevance_ordered {
            let matched = documents.len() as u64;

            // Reorder to match the search ranking, then apply the page window.
            if let Some(ids) = &filter.ids {
                let rank: HashMap<DocumentId, usize> = ids
                    .iter()
                    .enumerate()
                    .map(|(index, id)| (*id, index))
                    .collect();
                documents.sort_by_key(|stored| {
                    rank.get(&stored.document.id).copied().unwrap_or(usize::MAX)
                });
            }

            let start = filter.offset as usize;
            let end = start.saturating_add(filter.limit as usize);
            documents = documents
                .into_iter()
                .skip(start)
                .take(end.saturating_sub(start))
                .collect();
            matched
        } else {
            let (count_clause, count_arguments) = build_conditions(filter)?;
            let count_sql = format!(
                "SELECT COUNT(*) AS total
                 FROM documents d
                 JOIN document_versions v ON v.id = d.current_version_id
                 {count_clause}"
            );
            let row = sqlx::query_with(&count_sql, count_arguments)
                .fetch_one(&self.pool)
                .await
                .map_err(RepositoryError::backend)?;
            let total: i64 = row.try_get("total").map_err(RepositoryError::backend)?;
            total.max(0).cast_unsigned()
        };

        let ids: Vec<DocumentId> = documents.iter().map(|stored| stored.document.id).collect();
        let mut tags = self.load_tags(&ids).await?;
        for stored in &mut documents {
            stored.tags = tags.remove(&stored.document.id).unwrap_or_default();
        }

        Ok(DocumentPage { documents, total })
    }

    async fn find_version(
        &self,
        id: DocumentVersionId,
    ) -> Result<Option<DocumentVersion>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM document_versions WHERE id = ?1")
            .bind(id.into_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;

        row.as_ref()
            .map(|row| map_version_with(row, NO_PREFIX))
            .transpose()
    }

    async fn list_versions(
        &self,
        document_id: DocumentId,
    ) -> Result<Vec<DocumentVersion>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT * FROM document_versions WHERE document_id = ?1 ORDER BY number DESC",
        )
        .bind(document_id.into_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        rows.iter()
            .map(|row| map_version_with(row, NO_PREFIX))
            .collect()
    }

    async fn update_metadata(
        &self,
        id: DocumentId,
        title: &DocumentTitle,
        category_id: CategoryId,
        review_due_at: Option<OffsetDateTime>,
        at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "UPDATE documents
             SET title = ?2, category_id = ?3, review_due_at = ?4, updated_at = ?5
             WHERE id = ?1",
        )
        .bind(id.into_uuid())
        .bind(title.as_str())
        .bind(category_id.into_uuid())
        .bind(review_due_at)
        .bind(at)
        .execute(&self.pool)
        .await
        .map_err(|error| classify(error, "document", "category"))?;
        Ok(())
    }

    async fn set_lifecycle(
        &self,
        id: DocumentId,
        lifecycle: LifecycleState,
        at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE documents SET lifecycle = ?2, updated_at = ?3 WHERE id = ?1")
            .bind(id.into_uuid())
            .bind(lifecycle.as_str())
            .bind(at)
            .execute(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }

    async fn set_derivative(
        &self,
        version_id: DocumentVersionId,
        checksum: Sha256Checksum,
        key: &StorageKey,
    ) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(RepositoryError::backend)?;

        // The write-once trigger refuses a second attempt, so a retried conversion
        // job cannot change what an existing binder release renders.
        sqlx::query(
            "UPDATE document_versions
             SET derivative_key = ?2, derivative_checksum = ?3
             WHERE id = ?1",
        )
        .bind(version_id.into_uuid())
        .bind(key.as_str())
        .bind(checksum.to_hex())
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::backend)?;

        sqlx::query(
            "INSERT INTO blobs (storage_key, checksum, byte_size, reference_count, created_at)
             VALUES (?1, ?2, 0, 1, ?3)
             ON CONFLICT (storage_key) DO UPDATE SET reference_count = reference_count + 1",
        )
        .bind(key.as_str())
        .bind(checksum.to_hex())
        .bind(OffsetDateTime::now_utc())
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::backend)?;

        transaction
            .commit()
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }

    async fn find_by_checksum(
        &self,
        checksum: Sha256Checksum,
    ) -> Result<Option<DocumentId>, RepositoryError> {
        let row = sqlx::query(
            "SELECT document_id FROM document_versions WHERE checksum = ?1 ORDER BY id LIMIT 1",
        )
        .bind(checksum.to_hex())
        .fetch_optional(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        row.map(|row| {
            let id: Uuid = row
                .try_get("document_id")
                .map_err(RepositoryError::backend)?;
            Ok(DocumentId::from_uuid(id))
        })
        .transpose()
    }

    async fn delete(&self, id: DocumentId) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(RepositoryError::backend)?;

        // Released before the cascade removes the rows that name the blobs. The
        // schema's non-negative check turns a double release into a loud failure
        // rather than into deleted bytes another version still needs.
        sqlx::query(
            "UPDATE blobs SET reference_count = reference_count - 1
             WHERE storage_key IN (SELECT storage_key FROM document_versions WHERE document_id = ?1)",
        )
        .bind(id.into_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::backend)?;

        sqlx::query(
            "UPDATE blobs SET reference_count = reference_count - 1
             WHERE storage_key IN (
                 SELECT derivative_key FROM document_versions
                 WHERE document_id = ?1 AND derivative_key IS NOT NULL
             )",
        )
        .bind(id.into_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(RepositoryError::backend)?;

        sqlx::query("DELETE FROM documents WHERE id = ?1")
            .bind(id.into_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(RepositoryError::backend)?;

        transaction
            .commit()
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }
}

/// Builds the `WHERE` clause and its bound arguments for a listing.
///
/// Returned as a pair so the row query and the count query share exactly one
/// definition of what "matching" means; duplicating the predicate is how a total
/// silently stops agreeing with its rows.
fn build_conditions(
    filter: &DocumentFilter,
) -> Result<(String, SqliteArguments<'static>), RepositoryError> {
    let mut conditions: Vec<String> = Vec::new();
    let mut arguments = SqliteArguments::default();

    if let Some(category_id) = filter.category_id {
        conditions.push("d.category_id = ?".to_owned());
        arguments
            .add(category_id.into_uuid())
            .map_err(bind_failed)?;
    } else if !filter.category_ids.is_empty() {
        let placeholders = vec!["?"; filter.category_ids.len()].join(", ");
        conditions.push(format!("d.category_id IN ({placeholders})"));
        for id in &filter.category_ids {
            arguments.add(id.into_uuid()).map_err(bind_failed)?;
        }
    }

    if !filter.lifecycles.is_empty() {
        let placeholders = vec!["?"; filter.lifecycles.len()].join(", ");
        conditions.push(format!("d.lifecycle IN ({placeholders})"));
        for lifecycle in &filter.lifecycles {
            arguments.add(lifecycle.as_str()).map_err(bind_failed)?;
        }
    }

    if let Some(ids) = &filter.ids {
        if ids.is_empty() {
            // An empty allow-list means nothing matches. Omitting the condition
            // would instead match everything, which is the opposite.
            conditions.push("1 = 0".to_owned());
        } else {
            let placeholders = vec!["?"; ids.len()].join(", ");
            conditions.push(format!("d.id IN ({placeholders})"));
            for id in ids {
                arguments.add(id.into_uuid()).map_err(bind_failed)?;
            }
        }
    }

    for tag_id in &filter.tag_ids {
        // One EXISTS per tag, so the filter is "has all of these" rather than
        // "has any of these".
        conditions.push(
            "EXISTS (SELECT 1 FROM document_tags dt WHERE dt.document_id = d.id AND dt.tag_id = ?)"
                .to_owned(),
        );
        arguments.add(tag_id.into_uuid()).map_err(bind_failed)?;
    }

    let clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    Ok((clause, arguments))
}

/// Rebuilds a document, its current version, and its category name from a joined row.
fn map_stored(row: &SqliteRow) -> Result<StoredDocument, RepositoryError> {
    let document = map_document(row)?;
    let current_version = map_version_with(row, VERSION_PREFIX)?;
    let category_name: String = row
        .try_get("category_name")
        .map_err(RepositoryError::backend)?;

    Ok(StoredDocument {
        document,
        current_version,
        category_name: CategoryName::parse(&category_name)
            .map_err(|_| corrupt("categories.name"))?,
        tags: Vec::new(),
    })
}

/// Rebuilds a document from a row.
fn map_document(row: &SqliteRow) -> Result<Document, RepositoryError> {
    let id: Uuid = row.try_get("id").map_err(RepositoryError::backend)?;
    let title: String = row.try_get("title").map_err(RepositoryError::backend)?;
    let category_id: Uuid = row
        .try_get("category_id")
        .map_err(RepositoryError::backend)?;
    let lifecycle: String = row.try_get("lifecycle").map_err(RepositoryError::backend)?;
    let current_version_id: Option<Uuid> = row
        .try_get("current_version_id")
        .map_err(RepositoryError::backend)?;
    let version_count: i64 = row
        .try_get("version_count")
        .map_err(RepositoryError::backend)?;
    let source_path: Option<String> = row
        .try_get("source_path")
        .map_err(RepositoryError::backend)?;
    let review_due_at: Option<OffsetDateTime> = row
        .try_get("review_due_at")
        .map_err(RepositoryError::backend)?;
    let created_by: Uuid = row
        .try_get("created_by")
        .map_err(RepositoryError::backend)?;
    let created_at: OffsetDateTime = row
        .try_get("created_at")
        .map_err(RepositoryError::backend)?;
    let updated_at: OffsetDateTime = row
        .try_get("updated_at")
        .map_err(RepositoryError::backend)?;

    Ok(Document {
        id: DocumentId::from_uuid(id),
        title: DocumentTitle::parse(&title).map_err(|_| corrupt("documents.title"))?,
        category_id: CategoryId::from_uuid(category_id),
        lifecycle: lifecycle
            .parse()
            .map_err(|_| corrupt("documents.lifecycle"))?,
        current_version_id: current_version_id
            .map(DocumentVersionId::from_uuid)
            .ok_or_else(|| corrupt("documents.current_version_id"))?,
        version_count: u32::try_from(version_count.max(0)).unwrap_or(u32::MAX),
        source_path,
        review_due_at,
        created_by: UserId::from_uuid(created_by),
        created_at,
        updated_at,
    })
}

/// Rebuilds a version from a row whose version columns carry `prefix`.
fn map_version_with(row: &SqliteRow, prefix: &str) -> Result<DocumentVersion, RepositoryError> {
    /// Reads a column, applying the prefix.
    fn column(prefix: &str, name: &str) -> String {
        format!("{prefix}{name}")
    }

    let id: Uuid = row
        .try_get(column(prefix, "id").as_str())
        .map_err(RepositoryError::backend)?;
    let document_id: Uuid = row
        .try_get(column(prefix, "document_id").as_str())
        .map_err(RepositoryError::backend)?;
    let number: i64 = row
        .try_get(column(prefix, "number").as_str())
        .map_err(RepositoryError::backend)?;
    let original_filename: String = row
        .try_get(column(prefix, "original_filename").as_str())
        .map_err(RepositoryError::backend)?;
    let media_type: String = row
        .try_get(column(prefix, "media_type").as_str())
        .map_err(RepositoryError::backend)?;
    let byte_size: i64 = row
        .try_get(column(prefix, "byte_size").as_str())
        .map_err(RepositoryError::backend)?;
    let checksum: String = row
        .try_get(column(prefix, "checksum").as_str())
        .map_err(RepositoryError::backend)?;
    let storage_key: String = row
        .try_get(column(prefix, "storage_key").as_str())
        .map_err(RepositoryError::backend)?;
    let derivative_checksum: Option<String> = row
        .try_get(column(prefix, "derivative_checksum").as_str())
        .map_err(RepositoryError::backend)?;
    let derivative_key: Option<String> = row
        .try_get(column(prefix, "derivative_key").as_str())
        .map_err(RepositoryError::backend)?;
    let created_by: Uuid = row
        .try_get(column(prefix, "created_by").as_str())
        .map_err(RepositoryError::backend)?;
    let note: Option<String> = row
        .try_get(column(prefix, "note").as_str())
        .map_err(RepositoryError::backend)?;
    let created_at: OffsetDateTime = row
        .try_get(column(prefix, "created_at").as_str())
        .map_err(RepositoryError::backend)?;

    Ok(DocumentVersion {
        id: DocumentVersionId::from_uuid(id),
        document_id: DocumentId::from_uuid(document_id),
        number: u32::try_from(number)
            .ok()
            .and_then(|value| VersionNumber::new(value).ok())
            .ok_or_else(|| corrupt("document_versions.number"))?,
        original_filename: OriginalFilename::parse(&original_filename)
            .map_err(|_| corrupt("document_versions.original_filename"))?,
        media_type: MediaType::from_mime(&media_type)
            .ok_or_else(|| corrupt("document_versions.media_type"))?,
        byte_size: byte_size.max(0).cast_unsigned(),
        checksum: checksum
            .parse()
            .map_err(|_| corrupt("document_versions.checksum"))?,
        // Parsed rather than trusted: a tampered key must not become a path.
        storage_key: StorageKey::parse(&storage_key)
            .map_err(|_| corrupt("document_versions.storage_key"))?,
        derivative_checksum: derivative_checksum
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| corrupt("document_versions.derivative_checksum"))?,
        derivative_key: derivative_key
            .as_deref()
            .map(StorageKey::parse)
            .transpose()
            .map_err(|_| corrupt("document_versions.derivative_key"))?,
        created_by: UserId::from_uuid(created_by),
        note,
        created_at,
    })
}

/// Converts a bind failure into a repository error.
///
/// `Arguments::add` yields an already-boxed error, which the generic
/// `RepositoryError::backend` helper cannot take because its type parameter must
/// be `Sized`.
fn bind_failed(error: Box<dyn std::error::Error + Send + Sync>) -> RepositoryError {
    RepositoryError::Backend(error)
}

/// Reports a stored value this build cannot interpret.
fn corrupt(column: &'static str) -> RepositoryError {
    RepositoryError::backend(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{column} holds a value this build cannot interpret"),
    ))
}
