use async_trait::async_trait;
use elrond_application::{ApplicationError, AuthError, AuthRepository, LibraryRepository};
use elrond_domain::{
    auth::{InitialAdmin, NewSession},
    library::LibraryOverview,
};
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

#[async_trait]
impl AuthRepository for SqliteLibraryRepository {
    async fn create_initial_admin(
        &self,
        admin: InitialAdmin,
        session: NewSession,
    ) -> Result<(), AuthError> {
        let mut transaction = self.pool.begin().await.map_err(auth_repository_error)?;
        let users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&mut *transaction)
            .await
            .map_err(auth_repository_error)?;

        if users != 0 {
            return Err(AuthError::SetupCompleted);
        }

        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role) VALUES (?, ?, ?, 'admin')",
        )
        .bind(&admin.id)
        .bind(&admin.username)
        .bind(&admin.password_hash)
        .execute(&mut *transaction)
        .await
        .map_err(auth_repository_error)?;

        sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(&session.token_hash)
            .bind(&session.user_id)
            .bind(session.expires_at)
            .execute(&mut *transaction)
            .await
            .map_err(auth_repository_error)?;

        transaction.commit().await.map_err(auth_repository_error)
    }
}

fn repository_error(error: sqlx::Error) -> ApplicationError {
    ApplicationError::Repository(Box::new(error))
}

fn auth_repository_error(error: sqlx::Error) -> AuthError {
    AuthError::Repository(Box::new(error))
}
