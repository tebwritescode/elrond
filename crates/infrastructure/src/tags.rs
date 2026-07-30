//! SQLite-backed tag storage.

use async_trait::async_trait;
use elrond_application::ports::{RepositoryError, TagRepository};
use elrond_domain::{DocumentId, Tag, TagId, TagLabel};
use sqlx::sqlite::SqliteRow;
use sqlx::{Pool, Row, Sqlite};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::Database;

/// Tags stored in SQLite.
#[derive(Debug, Clone)]
pub struct SqliteTagRepository {
    pool: Pool<Sqlite>,
}

impl SqliteTagRepository {
    /// Binds the repository to a connected database.
    pub fn new(database: &Database) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }
}

#[async_trait]
impl TagRepository for SqliteTagRepository {
    async fn ensure(
        &self,
        labels: &[TagLabel],
        at: OffsetDateTime,
    ) -> Result<Vec<Tag>, RepositoryError> {
        if labels.is_empty() {
            return Ok(Vec::new());
        }

        let mut transaction = self.pool.begin().await.map_err(RepositoryError::backend)?;
        let mut tags = Vec::with_capacity(labels.len());

        for label in labels {
            // `ON CONFLICT DO NOTHING` then select, rather than select-then-insert:
            // two uploads introducing the same new tag at once would otherwise race
            // between the check and the write.
            sqlx::query(
                "INSERT INTO tags (id, label, label_key, created_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (label_key) DO NOTHING",
            )
            .bind(TagId::new().into_uuid())
            .bind(label.as_str())
            .bind(label.key())
            .bind(at)
            .execute(&mut *transaction)
            .await
            .map_err(RepositoryError::backend)?;

            let row = sqlx::query("SELECT id, label, created_at FROM tags WHERE label_key = ?1")
                .bind(label.key())
                .fetch_one(&mut *transaction)
                .await
                .map_err(RepositoryError::backend)?;
            tags.push(map_tag(&row)?);
        }

        transaction
            .commit()
            .await
            .map_err(RepositoryError::backend)?;
        Ok(tags)
    }

    async fn list_with_counts(&self) -> Result<Vec<(Tag, u64)>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT t.id, t.label, t.created_at, COUNT(dt.document_id) AS total
             FROM tags t
             LEFT JOIN document_tags dt ON dt.tag_id = t.id
             GROUP BY t.id
             ORDER BY t.label_key",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        rows.iter()
            .map(|row| {
                let total: i64 = row.try_get("total").map_err(RepositoryError::backend)?;
                Ok((map_tag(row)?, total.max(0).cast_unsigned()))
            })
            .collect()
    }

    async fn set_for_document(
        &self,
        document_id: DocumentId,
        tag_ids: &[TagId],
    ) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(RepositoryError::backend)?;

        // Replace wholesale rather than diff: the caller always supplies the
        // complete intended set, and a diff would need to handle removals anyway.
        sqlx::query("DELETE FROM document_tags WHERE document_id = ?1")
            .bind(document_id.into_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(RepositoryError::backend)?;

        for tag_id in tag_ids {
            sqlx::query("INSERT INTO document_tags (document_id, tag_id) VALUES (?1, ?2)")
                .bind(document_id.into_uuid())
                .bind(tag_id.into_uuid())
                .execute(&mut *transaction)
                .await
                .map_err(RepositoryError::backend)?;
        }

        transaction
            .commit()
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }

    async fn list_for_document(
        &self,
        document_id: DocumentId,
    ) -> Result<Vec<Tag>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT t.id, t.label, t.created_at
             FROM tags t
             JOIN document_tags dt ON dt.tag_id = t.id
             WHERE dt.document_id = ?1
             ORDER BY t.label_key",
        )
        .bind(document_id.into_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        rows.iter().map(map_tag).collect()
    }

    async fn list_for_documents(
        &self,
        document_ids: &[DocumentId],
    ) -> Result<Vec<(DocumentId, Tag)>, RepositoryError> {
        if document_ids.is_empty() {
            return Ok(Vec::new());
        }

        // One query with an expanded IN list, so a page of 50 documents costs one
        // round trip rather than 50.
        let placeholders = (1..=document_ids.len())
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
        for id in document_ids {
            query = query.bind(id.into_uuid());
        }
        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;

        rows.iter()
            .map(|row| {
                let document_id: Uuid = row
                    .try_get("document_id")
                    .map_err(RepositoryError::backend)?;
                Ok((DocumentId::from_uuid(document_id), map_tag(row)?))
            })
            .collect()
    }

    async fn prune_unused(&self) -> Result<u64, RepositoryError> {
        let result = sqlx::query(
            "DELETE FROM tags
             WHERE id NOT IN (SELECT DISTINCT tag_id FROM document_tags)",
        )
        .execute(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;
        Ok(result.rows_affected())
    }
}

/// Rebuilds a tag from a row.
fn map_tag(row: &SqliteRow) -> Result<Tag, RepositoryError> {
    let id: Uuid = row.try_get("id").map_err(RepositoryError::backend)?;
    let label: String = row.try_get("label").map_err(RepositoryError::backend)?;
    let created_at: OffsetDateTime = row
        .try_get("created_at")
        .map_err(RepositoryError::backend)?;

    Ok(Tag {
        id: TagId::from_uuid(id),
        label: TagLabel::parse(&label).map_err(|_| {
            RepositoryError::backend(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tags.label holds a value this build cannot interpret",
            ))
        })?,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_767_225_600).expect("valid time")
    }

    fn label(raw: &str) -> TagLabel {
        TagLabel::parse(raw).expect("valid label")
    }

    async fn repository() -> (Database, SqliteTagRepository) {
        let database = Database::connect_in_memory().await.expect("connects");
        let repository = SqliteTagRepository::new(&database);
        (database, repository)
    }

    #[tokio::test]
    async fn an_empty_label_set_creates_nothing() {
        let (_db, tags) = repository().await;
        assert!(tags.ensure(&[], at()).await.expect("no-op").is_empty());
    }

    #[tokio::test]
    async fn ensure_creates_then_reuses() {
        let (_db, tags) = repository().await;
        let first = tags
            .ensure(&[label("Policy"), label("Board")], at())
            .await
            .expect("created");
        assert_eq!(first.len(), 2);

        let second = tags
            .ensure(&[label("policy"), label("BOARD")], at())
            .await
            .expect("reused");

        // Case variants must resolve to the same rows, not create parallel tags.
        let mut first_ids: Vec<_> = first.iter().map(|tag| tag.id).collect();
        let mut second_ids: Vec<_> = second.iter().map(|tag| tag.id).collect();
        first_ids.sort();
        second_ids.sort();
        assert_eq!(first_ids, second_ids);

        assert_eq!(tags.list_with_counts().await.expect("listed").len(), 2);
    }

    #[tokio::test]
    async fn the_first_spelling_is_the_one_kept() {
        let (_db, tags) = repository().await;
        tags.ensure(&[label("Board Minutes")], at())
            .await
            .expect("created");
        let reused = tags
            .ensure(&[label("board minutes")], at())
            .await
            .expect("reused");
        assert_eq!(reused[0].label.as_str(), "Board Minutes");
    }

    #[tokio::test]
    async fn counts_start_at_zero_for_an_unused_tag() {
        let (_db, tags) = repository().await;
        tags.ensure(&[label("Unused")], at())
            .await
            .expect("created");

        let listed = tags.list_with_counts().await.expect("listed");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].1, 0);
    }

    #[tokio::test]
    async fn tags_are_listed_alphabetically_by_key() {
        let (_db, tags) = repository().await;
        tags.ensure(&[label("zebra"), label("Apple"), label("mango")], at())
            .await
            .expect("created");

        let labels: Vec<_> = tags
            .list_with_counts()
            .await
            .expect("listed")
            .into_iter()
            .map(|(tag, _)| tag.label.as_str().to_owned())
            .collect();
        assert_eq!(labels, vec!["Apple", "mango", "zebra"]);
    }

    #[tokio::test]
    async fn pruning_removes_only_unused_tags() {
        let (_db, tags) = repository().await;
        tags.ensure(&[label("Orphan"), label("Kept")], at())
            .await
            .expect("created");

        assert_eq!(tags.prune_unused().await.expect("pruned"), 2);
        assert!(tags.list_with_counts().await.expect("listed").is_empty());
    }

    #[tokio::test]
    async fn tags_for_absent_documents_resolve_to_nothing() {
        let (_db, tags) = repository().await;
        assert!(
            tags.list_for_document(DocumentId::new())
                .await
                .expect("query succeeds")
                .is_empty()
        );
        assert!(
            tags.list_for_documents(&[])
                .await
                .expect("query succeeds")
                .is_empty()
        );
    }
}
