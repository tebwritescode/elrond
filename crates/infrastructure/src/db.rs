//! SQLite connection management and migrations.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{
    SqliteAutoVacuum, SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous,
};
use sqlx::{Pool, Sqlite};
use thiserror::Error;

/// Embedded migration set, resolved at compile time from the repository root.
///
/// Compiling them in means the shipped image cannot drift from the schema it was
/// built against, and no migration CLI is needed at runtime.
static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Turns a driver error into the closest port-level error.
///
/// Shared by every repository so a uniqueness conflict is reported as a
/// conflict — and therefore as HTTP 409 — rather than as an opaque 500.
pub(crate) fn classify(
    error: sqlx::Error,
    resource: &'static str,
    field: &'static str,
) -> elrond_application::ports::RepositoryError {
    use elrond_application::ports::RepositoryError;

    if let sqlx::Error::Database(ref database_error) = error
        && database_error.is_unique_violation()
    {
        return RepositoryError::UniqueViolation { resource, field };
    }
    RepositoryError::backend(error)
}

/// A database setup or migration failure.
#[derive(Debug, Error)]
pub enum DatabaseError {
    /// The connection string was not a valid SQLite URL.
    #[error("invalid database URL")]
    InvalidUrl(#[source] sqlx::Error),

    /// The data directory could not be created.
    #[error("could not create the database directory at {path}")]
    Directory {
        /// Path that could not be created.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The pool could not be established.
    #[error("could not connect to the database")]
    Connect(#[source] sqlx::Error),

    /// A migration failed to apply.
    #[error("database migration failed")]
    Migrate(#[source] sqlx::migrate::MigrateError),

    /// A startup verification query failed.
    #[error("database verification failed")]
    Verify(#[source] sqlx::Error),
}

/// How the pool is configured.
#[derive(Debug, Clone)]
pub struct DatabaseSettings {
    /// SQLite connection URL, for example `sqlite://./dev-data/elrond.db?mode=rwc`.
    pub url: String,
    /// Maximum pooled connections.
    pub max_connections: u32,
    /// How long a statement waits on a write lock before giving up.
    pub busy_timeout: Duration,
}

impl DatabaseSettings {
    /// Builds settings from a URL using defaults tuned for SQLite.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            // SQLite serializes writes regardless of pool size, so a large pool
            // buys nothing but lock contention. A small pool keeps readers
            // concurrent under WAL while writers queue predictably.
            max_connections: 8,
            busy_timeout: Duration::from_secs(10),
        }
    }
}

/// A connected, migrated database.
#[derive(Debug, Clone)]
pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    /// Connects, applies pragmas, and runs pending migrations.
    pub async fn connect(settings: &DatabaseSettings) -> Result<Self, DatabaseError> {
        let mut options =
            SqliteConnectOptions::from_str(&settings.url).map_err(DatabaseError::InvalidUrl)?;

        // `from_str` records the requested filename; create its parent directory
        // before SQLite tries to open it so a fresh volume works with no manual
        // preparation.
        if let Some(parent) = options.get_filename().parent() {
            Self::ensure_directory(parent)?;
        }

        options = options
            .create_if_missing(true)
            // WAL lets readers proceed during a write, which is what makes a
            // single-file database usable for a browsing-heavy application.
            .journal_mode(SqliteJournalMode::Wal)
            // NORMAL is the standard WAL pairing: durable against process crash,
            // and it trades only the last commits against an OS-level crash.
            .synchronous(SqliteSynchronous::Normal)
            // SQLite leaves foreign keys off per connection by default, so the
            // cascade from users to sessions only works if this is set.
            .foreign_keys(true)
            .busy_timeout(settings.busy_timeout)
            // Reclaims space after large deletions without a manual VACUUM.
            .auto_vacuum(SqliteAutoVacuum::Incremental);

        let pool = SqlitePoolOptions::new()
            .max_connections(settings.max_connections)
            .acquire_timeout(Duration::from_secs(30))
            .connect_with(options)
            .await
            .map_err(DatabaseError::Connect)?;

        let database = Self { pool };
        database.migrate().await?;
        database.verify().await?;
        Ok(database)
    }

    /// Opens a private in-memory database, for tests.
    pub async fn connect_in_memory() -> Result<Self, DatabaseError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(DatabaseError::InvalidUrl)?
            .foreign_keys(true);

        // An in-memory database lives only as long as its connection, so the pool
        // is capped at one to guarantee every query sees the same database.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(DatabaseError::Connect)?;

        let database = Self { pool };
        database.migrate().await?;
        Ok(database)
    }

    /// Borrows the pool for adapters to run queries against.
    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }

    /// Closes the pool, flushing the WAL.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Applies pending migrations.
    async fn migrate(&self) -> Result<(), DatabaseError> {
        MIGRATIONS
            .run(&self.pool)
            .await
            .map_err(DatabaseError::Migrate)
    }

    /// Confirms the settings that matter actually took effect.
    ///
    /// A pragma silently failing to apply would be invisible until the first
    /// corruption or constraint bug, so it is checked at startup instead.
    async fn verify(&self) -> Result<(), DatabaseError> {
        let (journal_mode,): (String,) = sqlx::query_as("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(DatabaseError::Verify)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            tracing::warn!(
                journal_mode,
                "database is not in WAL mode; concurrent reads during writes will block"
            );
        }

        let (foreign_keys,): (i64,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(DatabaseError::Verify)?;
        if foreign_keys != 1 {
            tracing::warn!("foreign key enforcement is disabled");
        }

        Ok(())
    }

    /// Creates a directory tree if it is missing.
    fn ensure_directory(path: &Path) -> Result<(), DatabaseError> {
        if path.as_os_str().is_empty() || path.is_dir() {
            return Ok(());
        }
        std::fs::create_dir_all(path).map_err(|source| DatabaseError::Directory {
            path: path.display().to_string(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_apply_to_a_fresh_database() {
        let db = Database::connect_in_memory().await.expect("connects");
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'")
                .fetch_one(db.pool())
                .await
                .expect("query succeeds");
        assert!(count > 0, "migrations should have created tables");
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let db = Database::connect_in_memory().await.expect("connects");
        // Running the migrator a second time must be a no-op, which is what makes
        // a container restart safe.
        db.migrate()
            .await
            .expect("re-running migrations is a no-op");
    }

    #[tokio::test]
    async fn audit_events_reject_updates_and_deletes() {
        let db = Database::connect_in_memory().await.expect("connects");
        sqlx::query(
            "INSERT INTO audit_events (id, occurred_at, actor_label, action, subject_type)
             VALUES (randomblob(16), '2026-01-01T00:00:00Z', 'system', 'test', 'system')",
        )
        .execute(db.pool())
        .await
        .expect("insert is allowed");

        let update = sqlx::query("UPDATE audit_events SET action = 'tampered'")
            .execute(db.pool())
            .await;
        assert!(update.is_err(), "audit history must not be rewritable");

        let delete = sqlx::query("DELETE FROM audit_events")
            .execute(db.pool())
            .await;
        assert!(delete.is_err(), "audit history must not be erasable");
    }

    #[tokio::test]
    async fn deleting_a_user_cascades_to_its_sessions() {
        let db = Database::connect_in_memory().await.expect("connects");
        sqlx::query(
            "INSERT INTO users (id, email, display_name, role, password_hash, created_at, updated_at)
             VALUES (x'00000000000000000000000000000001', 'a@example.org', 'A', 'admin', 'hash',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .execute(db.pool())
        .await
        .expect("user inserted");
        sqlx::query(
            "INSERT INTO sessions (id, user_id, token_fingerprint, created_at, last_seen_at, expires_at)
             VALUES (x'00000000000000000000000000000002', x'00000000000000000000000000000001', 'fp',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-02-01T00:00:00Z')",
        )
        .execute(db.pool())
        .await
        .expect("session inserted");

        sqlx::query("DELETE FROM users")
            .execute(db.pool())
            .await
            .expect("user deleted");

        let (sessions,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sessions")
            .fetch_one(db.pool())
            .await
            .expect("query succeeds");
        assert_eq!(
            sessions, 0,
            "foreign key cascade should have removed the session"
        );
    }

    #[tokio::test]
    async fn role_check_constraint_rejects_unknown_roles() {
        let db = Database::connect_in_memory().await.expect("connects");
        let result = sqlx::query(
            "INSERT INTO users (id, email, display_name, role, password_hash, created_at, updated_at)
             VALUES (randomblob(16), 'b@example.org', 'B', 'superuser', 'hash',
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .execute(db.pool())
        .await;
        assert!(
            result.is_err(),
            "the CHECK constraint should mirror the Rust enum"
        );
    }
}
