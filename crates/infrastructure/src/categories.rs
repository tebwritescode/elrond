//! SQLite-backed category storage.

use async_trait::async_trait;
use elrond_application::ports::{CategoryRepository, NewCategory, RepositoryError};
use elrond_domain::{Category, CategoryId, CategoryName};
use sqlx::sqlite::SqliteRow;
use sqlx::{Pool, Row, Sqlite};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::{Database, classify};

/// Columns selected whenever a category is read.
const COLUMNS: &str = "id, parent_id, name, position, created_at, updated_at";

/// Categories stored in SQLite.
#[derive(Debug, Clone)]
pub struct SqliteCategoryRepository {
    pool: Pool<Sqlite>,
}

impl SqliteCategoryRepository {
    /// Binds the repository to a connected database.
    pub fn new(database: &Database) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }
}

#[async_trait]
impl CategoryRepository for SqliteCategoryRepository {
    async fn insert(&self, new_category: NewCategory) -> Result<Category, RepositoryError> {
        sqlx::query(
            "INSERT INTO categories (id, parent_id, name, name_key, position, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        )
        .bind(new_category.id.into_uuid())
        .bind(new_category.parent_id.map(CategoryId::into_uuid))
        .bind(new_category.name.as_str())
        .bind(new_category.name.matching_key())
        .bind(new_category.position)
        .bind(new_category.created_at)
        .execute(&self.pool)
        .await
        .map_err(|error| classify(error, "category", "name"))?;

        Ok(Category {
            id: new_category.id,
            parent_id: new_category.parent_id,
            name: new_category.name,
            position: new_category.position,
            created_at: new_category.created_at,
            updated_at: new_category.created_at,
        })
    }

    async fn list_all(&self) -> Result<Vec<Category>, RepositoryError> {
        let rows = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM categories ORDER BY parent_id, position, name"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        rows.iter().map(map_category).collect()
    }

    async fn find_by_id(&self, id: CategoryId) -> Result<Option<Category>, RepositoryError> {
        let row = sqlx::query(&format!("SELECT {COLUMNS} FROM categories WHERE id = ?1"))
            .bind(id.into_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;

        row.as_ref().map(map_category).transpose()
    }

    async fn find_child_by_name(
        &self,
        parent_id: Option<CategoryId>,
        name: &CategoryName,
    ) -> Result<Option<Category>, RepositoryError> {
        // `IS` rather than `=` so a NULL parent matches a NULL parent; `= NULL` is
        // never true in SQL and would silently never find a root category.
        let row = sqlx::query(&format!(
            "SELECT {COLUMNS} FROM categories WHERE parent_id IS ?1 AND name_key = ?2"
        ))
        .bind(parent_id.map(CategoryId::into_uuid))
        .bind(name.matching_key())
        .fetch_optional(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        row.as_ref().map(map_category).transpose()
    }

    async fn rename(
        &self,
        id: CategoryId,
        name: &CategoryName,
        at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "UPDATE categories SET name = ?2, name_key = ?3, updated_at = ?4 WHERE id = ?1",
        )
        .bind(id.into_uuid())
        .bind(name.as_str())
        .bind(name.matching_key())
        .bind(at)
        .execute(&self.pool)
        .await
        .map_err(|error| classify(error, "category", "name"))?;
        Ok(())
    }

    async fn set_parent(
        &self,
        id: CategoryId,
        parent_id: Option<CategoryId>,
        at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE categories SET parent_id = ?2, updated_at = ?3 WHERE id = ?1")
            .bind(id.into_uuid())
            .bind(parent_id.map(CategoryId::into_uuid))
            .bind(at)
            .execute(&self.pool)
            .await
            .map_err(|error| classify(error, "category", "name"))?;
        Ok(())
    }

    async fn delete(&self, id: CategoryId) -> Result<(), RepositoryError> {
        // The RESTRICT foreign keys refuse this while children or documents remain,
        // so a use-case check that raced with a concurrent upload still cannot
        // orphan anything.
        sqlx::query("DELETE FROM categories WHERE id = ?1")
            .bind(id.into_uuid())
            .execute(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }

    async fn document_counts(&self) -> Result<Vec<(CategoryId, u64)>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT category_id, COUNT(*) AS total FROM documents GROUP BY category_id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        rows.iter()
            .map(|row| {
                let id: Uuid = row
                    .try_get("category_id")
                    .map_err(RepositoryError::backend)?;
                let total: i64 = row.try_get("total").map_err(RepositoryError::backend)?;
                Ok((CategoryId::from_uuid(id), total.max(0).cast_unsigned()))
            })
            .collect()
    }
}

/// Rebuilds a category from a row.
fn map_category(row: &SqliteRow) -> Result<Category, RepositoryError> {
    let id: Uuid = row.try_get("id").map_err(RepositoryError::backend)?;
    let parent_id: Option<Uuid> = row.try_get("parent_id").map_err(RepositoryError::backend)?;
    let name: String = row.try_get("name").map_err(RepositoryError::backend)?;
    let position: i64 = row.try_get("position").map_err(RepositoryError::backend)?;
    let created_at: OffsetDateTime = row
        .try_get("created_at")
        .map_err(RepositoryError::backend)?;
    let updated_at: OffsetDateTime = row
        .try_get("updated_at")
        .map_err(RepositoryError::backend)?;

    Ok(Category {
        id: CategoryId::from_uuid(id),
        parent_id: parent_id.map(CategoryId::from_uuid),
        // Re-validated on the way out, so a hand-edited row fails loudly rather
        // than producing a name that breaks domain invariants.
        name: CategoryName::parse(&name).map_err(|_| {
            RepositoryError::backend(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "categories.name holds a value this build cannot interpret",
            ))
        })?,
        position,
        created_at,
        updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repository() -> (Database, SqliteCategoryRepository) {
        let database = Database::connect_in_memory().await.expect("connects");
        let repository = SqliteCategoryRepository::new(&database);
        (database, repository)
    }

    fn new_category(parent: Option<CategoryId>, name: &str) -> NewCategory {
        NewCategory {
            id: CategoryId::new(),
            parent_id: parent,
            name: CategoryName::parse(name).expect("valid name"),
            position: 0,
            created_at: OffsetDateTime::from_unix_timestamp(1_767_225_600).expect("valid time"),
        }
    }

    #[tokio::test]
    async fn a_root_category_round_trips() {
        let (_db, categories) = repository().await;
        let inserted = categories
            .insert(new_category(None, "Policies"))
            .await
            .expect("inserted");

        let found = categories
            .find_by_id(inserted.id)
            .await
            .expect("query succeeds")
            .expect("exists");
        assert_eq!(found, inserted);
        assert!(found.is_root());
    }

    #[tokio::test]
    async fn a_child_records_its_parent() {
        let (_db, categories) = repository().await;
        let parent = categories
            .insert(new_category(None, "Policies"))
            .await
            .expect("inserted");
        let child = categories
            .insert(new_category(Some(parent.id), "2026"))
            .await
            .expect("inserted");

        assert_eq!(child.parent_id, Some(parent.id));
        assert!(!child.is_root());
    }

    #[tokio::test]
    async fn a_root_category_is_findable_by_name() {
        let (_db, categories) = repository().await;
        let inserted = categories
            .insert(new_category(None, "Policies"))
            .await
            .expect("inserted");

        // `parent_id IS NULL` rather than `= NULL`: the latter is never true, and
        // getting it wrong would make the ZIP importer create a new root every run.
        let found = categories
            .find_child_by_name(None, &CategoryName::parse("Policies").expect("valid"))
            .await
            .expect("query succeeds")
            .expect("found");
        assert_eq!(found.id, inserted.id);
    }

    #[tokio::test]
    async fn name_matching_ignores_case_and_spacing() {
        let (_db, categories) = repository().await;
        let inserted = categories
            .insert(new_category(None, "Board Minutes"))
            .await
            .expect("inserted");

        for candidate in ["board minutes", "BOARD MINUTES", "  Board   Minutes  "] {
            let found = categories
                .find_child_by_name(None, &CategoryName::parse(candidate).expect("valid"))
                .await
                .expect("query succeeds")
                .expect("found");
            assert_eq!(found.id, inserted.id, "for {candidate:?}");
        }
    }

    #[tokio::test]
    async fn a_duplicate_sibling_name_is_a_unique_violation() {
        let (_db, categories) = repository().await;
        categories
            .insert(new_category(None, "Policies"))
            .await
            .expect("inserted");

        let error = categories
            .insert(new_category(None, "policies"))
            .await
            .expect_err("refused");
        assert!(matches!(
            error,
            RepositoryError::UniqueViolation {
                resource: "category",
                field: "name"
            }
        ));
    }

    #[tokio::test]
    async fn the_same_name_under_different_parents_is_allowed() {
        let (_db, categories) = repository().await;
        let a = categories
            .insert(new_category(None, "Policies"))
            .await
            .expect("inserted");
        let b = categories
            .insert(new_category(None, "Finance"))
            .await
            .expect("inserted");

        categories
            .insert(new_category(Some(a.id), "2026"))
            .await
            .expect("inserted");
        categories
            .insert(new_category(Some(b.id), "2026"))
            .await
            .expect("the same name under a different parent is fine");
    }

    #[tokio::test]
    async fn renaming_updates_the_matching_key_too() {
        let (_db, categories) = repository().await;
        let inserted = categories
            .insert(new_category(None, "Policies"))
            .await
            .expect("inserted");

        let renamed = CategoryName::parse("Retention Policies").expect("valid");
        let later = inserted.created_at + time::Duration::hours(1);
        categories
            .rename(inserted.id, &renamed, later)
            .await
            .expect("renamed");

        // If the key were not updated, lookups would still resolve the old name and
        // the importer would fail to find the category it just renamed.
        assert!(
            categories
                .find_child_by_name(None, &renamed)
                .await
                .expect("query succeeds")
                .is_some()
        );
        assert!(
            categories
                .find_child_by_name(None, &CategoryName::parse("Policies").expect("valid"))
                .await
                .expect("query succeeds")
                .is_none()
        );

        let found = categories
            .find_by_id(inserted.id)
            .await
            .expect("query succeeds")
            .expect("exists");
        assert_eq!(found.updated_at, later);
        assert_eq!(found.created_at, inserted.created_at);
    }

    #[tokio::test]
    async fn reparenting_moves_a_category() {
        let (_db, categories) = repository().await;
        let a = categories
            .insert(new_category(None, "Policies"))
            .await
            .expect("inserted");
        let b = categories
            .insert(new_category(None, "Finance"))
            .await
            .expect("inserted");

        categories
            .set_parent(b.id, Some(a.id), b.created_at)
            .await
            .expect("moved");
        let moved = categories
            .find_by_id(b.id)
            .await
            .expect("query succeeds")
            .expect("exists");
        assert_eq!(moved.parent_id, Some(a.id));

        categories
            .set_parent(b.id, None, b.created_at)
            .await
            .expect("promoted");
        assert!(
            categories
                .find_by_id(b.id)
                .await
                .expect("query succeeds")
                .expect("exists")
                .is_root()
        );
    }

    #[tokio::test]
    async fn a_category_with_children_cannot_be_deleted() {
        let (_db, categories) = repository().await;
        let parent = categories
            .insert(new_category(None, "Policies"))
            .await
            .expect("inserted");
        categories
            .insert(new_category(Some(parent.id), "2026"))
            .await
            .expect("inserted");

        assert!(
            categories.delete(parent.id).await.is_err(),
            "RESTRICT should refuse while a child remains"
        );

        // The leaf itself deletes fine.
        let child = categories
            .find_child_by_name(
                Some(parent.id),
                &CategoryName::parse("2026").expect("valid"),
            )
            .await
            .expect("query succeeds")
            .expect("exists");
        categories.delete(child.id).await.expect("leaf deleted");
        categories.delete(parent.id).await.expect("now empty");
    }

    #[tokio::test]
    async fn an_empty_library_reports_no_document_counts() {
        let (_db, categories) = repository().await;
        categories
            .insert(new_category(None, "Policies"))
            .await
            .expect("inserted");
        assert!(
            categories
                .document_counts()
                .await
                .expect("query succeeds")
                .is_empty()
        );
    }
}
