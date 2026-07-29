use async_trait::async_trait;
use elrond_application::{ApplicationError, LibraryRepository};
use elrond_domain::library::LibraryOverview;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

pub struct SqliteLibraryRepository {
    pool: SqlitePool,
}

impl SqliteLibraryRepository {
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        sqlx::migrate!("../../migrations").run(&pool).await?;

        Ok(Self { pool })
    }
}

#[async_trait]
impl LibraryRepository for SqliteLibraryRepository {
    async fn overview(
        &self,
        stirling_configured: bool,
    ) -> Result<LibraryOverview, ApplicationError> {
        let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(repository_error)?;
        let documents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM documents")
            .fetch_one(&self.pool)
            .await
            .map_err(repository_error)?;
        let categories: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM categories")
            .fetch_one(&self.pool)
            .await
            .map_err(repository_error)?;
        let binders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM binders")
            .fetch_one(&self.pool)
            .await
            .map_err(repository_error)?;
        let pending_reviews: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM documents WHERE status = 'in_review' OR (review_due_at IS NOT NULL AND review_due_at <= datetime('now', '+30 days'))",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(repository_error)?;

        Ok(LibraryOverview {
            setup_required: users == 0,
            documents,
            categories,
            binders,
            pending_reviews,
            stirling_configured,
        })
    }
}

fn repository_error(error: sqlx::Error) -> ApplicationError {
    ApplicationError::Repository(Box::new(error))
}
