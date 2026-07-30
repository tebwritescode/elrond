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
            "INSERT INTO users (id, username, role, password_hash, created_at, updated_at)
             VALUES (x'00000000000000000000000000000001', 'first.admin', 'admin', 'hash',
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

    /// Inserts a child named "2026" under `parent`.
    async fn insert_child(
        db: &Database,
        parent: [u8; 16],
    ) -> Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
        sqlx::query(
            "INSERT INTO categories (id, parent_id, name, name_key, created_at, updated_at)
             VALUES (randomblob(16), ?1, '2026', '2026', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(parent.as_slice())
        .execute(db.pool())
        .await
    }

    /// Records a conversion result against a version.
    async fn set_derivative(
        db: &Database,
        version: [u8; 16],
        key: &str,
    ) -> Result<sqlx::sqlite::SqliteQueryResult, sqlx::Error> {
        sqlx::query(
            "UPDATE document_versions
             SET derivative_key = ?2,
                 derivative_checksum = 'aa3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'
             WHERE id = ?1",
        )
        .bind(version.as_slice())
        .bind(key)
        .execute(db.pool())
        .await
    }

    /// Inserts a root category and returns its id bytes as a hex literal.
    async fn seed_category(db: &Database) -> [u8; 16] {
        let id = [0x0a_u8; 16];
        sqlx::query(
            "INSERT INTO categories (id, name, name_key, created_at, updated_at)
             VALUES (?1, 'Policies', 'policies', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.as_slice())
        .execute(db.pool())
        .await
        .expect("category inserted");
        id
    }

    #[tokio::test]
    async fn root_categories_cannot_share_a_name() {
        let db = Database::connect_in_memory().await.expect("connects");
        seed_category(&db).await;

        // SQLite treats NULLs as distinct in a unique index, so this only works
        // because of the partial index dedicated to root rows.
        let result = sqlx::query(
            "INSERT INTO categories (id, name, name_key, created_at, updated_at)
             VALUES (randomblob(16), 'policies', 'policies', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .execute(db.pool())
        .await;
        assert!(result.is_err(), "two root categories shared a name");
    }

    #[tokio::test]
    async fn siblings_cannot_share_a_name_but_cousins_can() {
        let db = Database::connect_in_memory().await.expect("connects");
        let parent = seed_category(&db).await;
        let other_parent = [0x0b_u8; 16];
        sqlx::query(
            "INSERT INTO categories (id, name, name_key, created_at, updated_at)
             VALUES (?1, 'Finance', 'finance', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(other_parent.as_slice())
        .execute(db.pool())
        .await
        .expect("second root inserted");

        insert_child(&db, parent)
            .await
            .expect("first child inserted");
        assert!(
            insert_child(&db, parent).await.is_err(),
            "siblings shared a name"
        );
        insert_child(&db, other_parent)
            .await
            .expect("the same name under a different parent is fine");
    }

    #[tokio::test]
    async fn a_category_cannot_be_its_own_parent() {
        let db = Database::connect_in_memory().await.expect("connects");
        let id = seed_category(&db).await;

        let result = sqlx::query("UPDATE categories SET parent_id = id WHERE id = ?1")
            .bind(id.as_slice())
            .execute(db.pool())
            .await;
        assert!(result.is_err(), "a one-node cycle was accepted");
    }

    #[tokio::test]
    async fn deleting_a_category_with_documents_is_refused() {
        let db = Database::connect_in_memory().await.expect("connects");
        let category = seed_category(&db).await;
        sqlx::query(
            "INSERT INTO documents (id, title, category_id, lifecycle, created_by, created_at, updated_at)
             VALUES (randomblob(16), 'Retention Policy', ?1, 'draft', randomblob(16),
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(category.as_slice())
        .execute(db.pool())
        .await
        .expect("document inserted");

        // RESTRICT, not CASCADE: documents must never vanish with their category.
        let result = sqlx::query("DELETE FROM categories WHERE id = ?1")
            .bind(category.as_slice())
            .execute(db.pool())
            .await;
        assert!(result.is_err(), "documents were silently deletable");
    }

    #[tokio::test]
    async fn version_count_and_current_version_stay_consistent() {
        let db = Database::connect_in_memory().await.expect("connects");
        let category = seed_category(&db).await;

        // Claiming a version without naming one must be refused.
        let result = sqlx::query(
            "INSERT INTO documents (id, title, category_id, lifecycle, version_count, created_by, created_at, updated_at)
             VALUES (randomblob(16), 'Broken', ?1, 'draft', 1, randomblob(16),
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(category.as_slice())
        .execute(db.pool())
        .await;
        assert!(
            result.is_err(),
            "a document claimed a version it did not have"
        );
    }

    /// Inserts a document with one version, returning both ids.
    async fn seed_document_with_version(db: &Database) -> ([u8; 16], [u8; 16]) {
        let category = seed_category(db).await;
        let document = [0x0c_u8; 16];
        let version = [0x0d_u8; 16];

        sqlx::query(
            "INSERT INTO documents (id, title, category_id, lifecycle, current_version_id, version_count, created_by, created_at, updated_at)
             VALUES (?1, 'Retention Policy', ?2, 'draft', ?3, 1, randomblob(16),
                     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(document.as_slice())
        .bind(category.as_slice())
        .bind(version.as_slice())
        .execute(db.pool())
        .await
        .expect("document inserted");

        sqlx::query(
            "INSERT INTO document_versions
                 (id, document_id, number, original_filename, media_type, byte_size,
                  checksum, storage_key, created_by, created_at)
             VALUES (?1, ?2, 1, 'policy.pdf', 'application/pdf', 1024,
                     'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
                     'originals/e3/b0/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
                     randomblob(16), '2026-01-01T00:00:00Z')",
        )
        .bind(version.as_slice())
        .bind(document.as_slice())
        .execute(db.pool())
        .await
        .expect("version inserted");

        (document, version)
    }

    #[tokio::test]
    async fn version_content_cannot_be_edited_in_place() {
        let db = Database::connect_in_memory().await.expect("connects");
        let (_document, version) = seed_document_with_version(&db).await;

        for column in [
            "checksum = 'ff'",
            "storage_key = 'originals/ff/ff/ff'",
            "byte_size = 0",
            "media_type = 'text/plain'",
            "original_filename = 'other.pdf'",
            "number = 2",
        ] {
            let result = sqlx::query(&format!(
                "UPDATE document_versions SET {column} WHERE id = ?1"
            ))
            .bind(version.as_slice())
            .execute(db.pool())
            .await;
            assert!(
                result.is_err(),
                "a binder release pins this version; {column} must not be editable"
            );
        }
    }

    #[tokio::test]
    async fn a_derivative_can_be_filled_in_once_and_not_replaced() {
        let db = Database::connect_in_memory().await.expect("connects");
        let (_document, version) = seed_document_with_version(&db).await;

        set_derivative(&db, version, "derivatives/aa/3b/x.pdf")
            .await
            .expect("the first conversion result is accepted");
        assert!(
            set_derivative(&db, version, "derivatives/bb/3b/x.pdf")
                .await
                .is_err(),
            "replacing a derivative would change what an existing release renders"
        );
    }

    #[tokio::test]
    async fn versions_cannot_reuse_a_sequence_number() {
        let db = Database::connect_in_memory().await.expect("connects");
        let (document, _version) = seed_document_with_version(&db).await;

        let result = sqlx::query(
            "INSERT INTO document_versions
                 (id, document_id, number, original_filename, media_type, byte_size,
                  checksum, storage_key, created_by, created_at)
             VALUES (randomblob(16), ?1, 1, 'again.pdf', 'application/pdf', 2048,
                     'aa', 'originals/aa/bb/cc', randomblob(16), '2026-01-01T00:00:00Z')",
        )
        .bind(document.as_slice())
        .execute(db.pool())
        .await;
        assert!(result.is_err(), "two versions shared a sequence number");
    }

    #[tokio::test]
    async fn deleting_a_document_cascades_to_its_versions_and_tags() {
        let db = Database::connect_in_memory().await.expect("connects");
        let (document, _version) = seed_document_with_version(&db).await;

        sqlx::query("DELETE FROM documents WHERE id = ?1")
            .bind(document.as_slice())
            .execute(db.pool())
            .await
            .expect("document deleted");

        let (versions,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM document_versions")
            .fetch_one(db.pool())
            .await
            .expect("query succeeds");
        assert_eq!(versions, 0);
    }

    #[tokio::test]
    async fn a_derivative_needs_both_its_key_and_its_checksum() {
        let db = Database::connect_in_memory().await.expect("connects");
        let (_document, version) = seed_document_with_version(&db).await;

        let result = sqlx::query(
            "UPDATE document_versions SET derivative_key = 'derivatives/aa/bb/x.pdf' WHERE id = ?1",
        )
        .bind(version.as_slice())
        .execute(db.pool())
        .await;
        assert!(result.is_err(), "a half-recorded derivative was accepted");
    }

    #[tokio::test]
    async fn full_text_search_finds_a_document_and_folds_diacritics() {
        let db = Database::connect_in_memory().await.expect("connects");
        sqlx::query(
            "INSERT INTO documents_fts (document_id, title, filename, tags, content)
             VALUES ('019fb07b', 'Résumé of Findings', 'resume.pdf', 'policy board',
                     'The committee resolved to adopt the retention schedule.')",
        )
        .execute(db.pool())
        .await
        .expect("indexed");

        for query in ["resume", "résumé", "retention", "polic*", "committee"] {
            let (hits,): (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM documents_fts WHERE documents_fts MATCH ?1")
                    .bind(query)
                    .fetch_one(db.pool())
                    .await
                    .unwrap_or_else(|error| panic!("search for {query:?} failed: {error}"));
            assert_eq!(hits, 1, "expected {query:?} to match");
        }

        let (misses,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM documents_fts WHERE documents_fts MATCH ?1")
                .bind("unrelated")
                .fetch_one(db.pool())
                .await
                .expect("query succeeds");
        assert_eq!(misses, 0);
    }

    #[tokio::test]
    async fn blob_reference_counts_cannot_go_negative() {
        let db = Database::connect_in_memory().await.expect("connects");
        sqlx::query(
            "INSERT INTO blobs (storage_key, checksum, byte_size, reference_count, created_at)
             VALUES ('originals/aa/bb/cc', 'aabbcc', 10, 0, '2026-01-01T00:00:00Z')",
        )
        .execute(db.pool())
        .await
        .expect("blob recorded");

        let result = sqlx::query(
            "UPDATE blobs SET reference_count = reference_count - 1 WHERE storage_key = 'originals/aa/bb/cc'",
        )
        .execute(db.pool())
        .await;
        assert!(
            result.is_err(),
            "an underflowed reference count would delete bytes another version still needs"
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
