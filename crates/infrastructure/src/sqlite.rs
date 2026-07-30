use std::{collections::HashMap, path::PathBuf};

use async_trait::async_trait;
use elrond_application::{
    ApplicationError, AuthError, AuthRepository, CatalogError, CatalogRepository, ConversionError,
    ConversionJobRepository, ImportError, ImportRepository, LibraryRepository,
};
use elrond_domain::{
    auth::{AuthenticatedUser, InitialAdmin, NewSession, UserCredentials},
    catalog::{CategorySummary, DocumentSummary},
    conversions::{ConversionJob, ConversionStatus, PdfDerivative},
    imports::{ImportSummary, PreparedImport},
    library::LibraryOverview,
};
use sqlx::{Sqlite, SqlitePool, Transaction, sqlite::SqlitePoolOptions};
use uuid::Uuid;

pub struct SqliteLibraryRepository {
    pool: SqlitePool,
    data_dir: PathBuf,
}

impl SqliteLibraryRepository {
    pub async fn connect(
        database_url: &str,
        data_dir: impl Into<PathBuf>,
    ) -> Result<Self, sqlx::Error> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        sqlx::migrate!("../../migrations").run(&pool).await?;

        Ok(Self {
            pool,
            data_dir: data_dir.into(),
        })
    }
}

#[async_trait]
impl CatalogRepository for SqliteLibraryRepository {
    async fn list_documents(&self) -> Result<Vec<DocumentSummary>, CatalogError> {
        sqlx::query_as::<_, (String, String, String, Option<String>, i64, String, bool, String, Option<String>, String)>(
            "SELECT documents.id, documents.title, documents.status, categories.name, document_versions.version_number, document_versions.original_filename, document_versions.pdf_storage_key IS NOT NULL, CASE WHEN document_versions.pdf_storage_key IS NOT NULL THEN 'ready' WHEN conversion_jobs.status = 'processing' THEN 'processing' WHEN conversion_jobs.status = 'failed' THEN 'failed' ELSE 'queued' END, conversion_jobs.last_error, documents.updated_at FROM documents LEFT JOIN categories ON categories.id = documents.category_id JOIN document_versions ON document_versions.document_id = documents.id LEFT JOIN conversion_jobs ON conversion_jobs.document_version_id = document_versions.id WHERE document_versions.version_number = (SELECT MAX(latest.version_number) FROM document_versions AS latest WHERE latest.document_id = documents.id) ORDER BY documents.updated_at DESC, documents.title COLLATE NOCASE LIMIT 500",
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(
                    |(
                        id,
                        title,
                        status,
                        category_name,
                        version_number,
                        original_filename,
                        has_pdf,
                        conversion_status,
                        conversion_error,
                        updated_at,
                    )| DocumentSummary {
                        id,
                        title,
                        status,
                        category_name,
                        version_number,
                        original_filename,
                        has_pdf,
                        conversion_status: parse_conversion_status(&conversion_status),
                        conversion_error,
                        updated_at,
                    },
                )
                .collect()
        })
        .map_err(catalog_repository_error)
    }

    async fn list_categories(&self) -> Result<Vec<CategorySummary>, CatalogError> {
        sqlx::query_as::<_, (String, Option<String>, String, i64)>(
            "SELECT categories.id, categories.parent_id, categories.name, COUNT(documents.id) FROM categories LEFT JOIN documents ON documents.category_id = categories.id GROUP BY categories.id ORDER BY categories.sort_order, categories.name COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|(id, parent_id, name, document_count)| CategorySummary {
                    id,
                    parent_id,
                    name,
                    document_count,
                })
                .collect()
        })
        .map_err(catalog_repository_error)
    }
}

#[async_trait]
impl ImportRepository for SqliteLibraryRepository {
    async fn commit_import(
        &self,
        import: PreparedImport,
        actor_user_id: &str,
    ) -> Result<ImportSummary, ImportError> {
        let originals_dir = self.data_dir.join("originals");
        tokio::fs::create_dir_all(&originals_dir)
            .await
            .map_err(import_repository_error)?;

        let mut transaction = self.pool.begin().await.map_err(import_repository_error)?;
        let mut category_ids = HashMap::<Vec<String>, String>::new();
        let mut categories_created = 0;
        for path in &import.categories {
            ensure_category_path(
                &mut transaction,
                path,
                &mut category_ids,
                &mut categories_created,
            )
            .await?;
        }

        let mut documents_imported = 0;
        let mut duplicates_skipped = 0;
        for document in import.documents {
            let duplicate: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM document_versions WHERE original_sha256 = ?)",
            )
            .bind(&document.sha256)
            .fetch_one(&mut *transaction)
            .await
            .map_err(import_repository_error)?;
            if duplicate {
                duplicates_skipped += 1;
                continue;
            }

            let storage_path = originals_dir
                .join(&document.sha256[..2])
                .join(&document.sha256);
            if !tokio::fs::try_exists(&storage_path)
                .await
                .map_err(import_repository_error)?
            {
                let parent = storage_path.parent().ok_or(ImportError::UnsafeEntry)?;
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(import_repository_error)?;
                tokio::fs::write(&storage_path, &document.content)
                    .await
                    .map_err(import_repository_error)?;
            }

            let category_id = category_ids
                .get(&document.category_path)
                .ok_or(ImportError::UnsafeEntry)?;
            let document_id = Uuid::new_v4().to_string();
            let version_id = Uuid::new_v4().to_string();
            let storage_key = storage_path
                .strip_prefix(&self.data_dir)
                .unwrap_or(&storage_path)
                .to_string_lossy()
                .replace('\\', "/");
            let is_pdf = document.media_type == "application/pdf";

            sqlx::query("INSERT INTO documents (id, category_id, title) VALUES (?, ?, ?)")
                .bind(&document_id)
                .bind(category_id)
                .bind(&document.title)
                .execute(&mut *transaction)
                .await
                .map_err(import_repository_error)?;
            sqlx::query(
                "INSERT INTO document_versions (id, document_id, version_number, original_filename, original_media_type, original_sha256, original_storage_key, pdf_sha256, pdf_storage_key) VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&version_id)
            .bind(&document_id)
            .bind(&document.filename)
            .bind(&document.media_type)
            .bind(&document.sha256)
            .bind(&storage_key)
            .bind(is_pdf.then_some(&document.sha256))
            .bind(is_pdf.then_some(&storage_key))
            .execute(&mut *transaction)
            .await
            .map_err(import_repository_error)?;
            if !is_pdf {
                sqlx::query("INSERT INTO conversion_jobs (id, document_version_id) VALUES (?, ?)")
                    .bind(Uuid::new_v4().to_string())
                    .bind(&version_id)
                    .execute(&mut *transaction)
                    .await
                    .map_err(import_repository_error)?;
            }
            sqlx::query(
                "INSERT INTO document_search (document_id, title, document_number, extracted_text) VALUES (?, ?, '', '')",
            )
            .bind(&document_id)
            .bind(&document.title)
            .execute(&mut *transaction)
            .await
            .map_err(import_repository_error)?;
            documents_imported += 1;
        }

        let summary = ImportSummary {
            categories_created,
            documents_imported,
            duplicates_skipped,
            unsupported_skipped: import.unsupported_skipped,
        };
        let details = serde_json::to_string(&summary).map_err(import_repository_error)?;
        sqlx::query(
            "INSERT INTO audit_events (actor_user_id, action, subject_type, details_json) VALUES (?, 'import.zip', 'library', ?)",
        )
        .bind(actor_user_id)
        .bind(details)
        .execute(&mut *transaction)
        .await
        .map_err(import_repository_error)?;
        transaction
            .commit()
            .await
            .map_err(import_repository_error)?;

        Ok(summary)
    }
}

async fn ensure_category_path(
    transaction: &mut Transaction<'_, Sqlite>,
    path: &[String],
    known: &mut HashMap<Vec<String>, String>,
    created: &mut usize,
) -> Result<String, ImportError> {
    let mut parent_id: Option<String> = None;
    for depth in 1..=path.len() {
        let current_path = path[..depth].to_vec();
        if let Some(existing) = known.get(&current_path) {
            parent_id = Some(existing.clone());
            continue;
        }
        let name = &path[depth - 1];
        let existing: Option<String> = if let Some(parent) = &parent_id {
            sqlx::query_scalar(
                "SELECT id FROM categories WHERE parent_id = ? AND name = ? COLLATE NOCASE",
            )
            .bind(parent)
            .bind(name)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(import_repository_error)?
        } else {
            sqlx::query_scalar(
                "SELECT id FROM categories WHERE parent_id IS NULL AND name = ? COLLATE NOCASE",
            )
            .bind(name)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(import_repository_error)?
        };
        let id = if let Some(existing) = existing {
            existing
        } else {
            let id = Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO categories (id, parent_id, name) VALUES (?, ?, ?)")
                .bind(&id)
                .bind(&parent_id)
                .bind(name)
                .execute(&mut **transaction)
                .await
                .map_err(import_repository_error)?;
            *created += 1;
            id
        };
        known.insert(current_path, id.clone());
        parent_id = Some(id);
    }
    parent_id.ok_or(ImportError::UnsafeEntry)
}

#[async_trait]
impl ConversionJobRepository for SqliteLibraryRepository {
    async fn claim_next(&self) -> Result<Option<ConversionJob>, ConversionError> {
        sqlx::query("UPDATE conversion_jobs SET status = 'failed', lease_expires_at = NULL, lease_token = NULL, last_error = 'The conversion stopped before it completed.', updated_at = CURRENT_TIMESTAMP WHERE status = 'processing' AND lease_expires_at <= CURRENT_TIMESTAMP AND attempts >= max_attempts")
            .execute(&self.pool)
            .await
            .map_err(conversion_repository_error)?;
        let lease_token = Uuid::new_v4().to_string();
        let row = sqlx::query_as::<_, (String, String, String, String, String)>(
            "UPDATE conversion_jobs SET status = 'processing', attempts = attempts + 1, lease_expires_at = datetime('now', '+5 minutes'), lease_token = ?, updated_at = CURRENT_TIMESTAMP WHERE id = (SELECT id FROM conversion_jobs WHERE ((status IN ('queued', 'failed') AND available_at <= CURRENT_TIMESTAMP) OR (status = 'processing' AND lease_expires_at <= CURRENT_TIMESTAMP)) AND attempts < max_attempts ORDER BY created_at LIMIT 1) RETURNING id, document_version_id, (SELECT original_filename FROM document_versions WHERE document_versions.id = document_version_id), (SELECT original_media_type FROM document_versions WHERE document_versions.id = document_version_id), (SELECT original_storage_key FROM document_versions WHERE document_versions.id = document_version_id)",
        )
        .bind(&lease_token)
        .fetch_optional(&self.pool)
        .await
        .map_err(conversion_repository_error)?;
        let Some((
            id,
            document_version_id,
            original_filename,
            original_media_type,
            original_storage_key,
        )) = row
        else {
            return Ok(None);
        };
        Ok(Some(ConversionJob {
            id,
            lease_token,
            document_version_id,
            original_filename,
            original_media_type,
            original_storage_key,
        }))
    }

    async fn load_original(&self, job: &ConversionJob) -> Result<Vec<u8>, ConversionError> {
        tokio::fs::read(self.data_dir.join(&job.original_storage_key))
            .await
            .map_err(conversion_repository_error)
    }

    async fn complete(
        &self,
        job: &ConversionJob,
        derivative: PdfDerivative,
    ) -> Result<(), ConversionError> {
        let relative_key = format!(
            "derivatives/{}/{}.pdf",
            &derivative.sha256[..2],
            derivative.sha256
        );
        let storage_path = self.data_dir.join(&relative_key);
        if !tokio::fs::try_exists(&storage_path)
            .await
            .map_err(conversion_repository_error)?
        {
            let parent = storage_path.parent().ok_or_else(|| {
                conversion_repository_error(std::io::Error::other("invalid derivative path"))
            })?;
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(conversion_repository_error)?;
            let temporary = storage_path.with_extension(format!("{}.tmp", Uuid::new_v4()));
            tokio::fs::write(&temporary, &derivative.content)
                .await
                .map_err(conversion_repository_error)?;
            tokio::fs::rename(&temporary, &storage_path)
                .await
                .map_err(conversion_repository_error)?;
        }
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(conversion_repository_error)?;
        let completed = sqlx::query("UPDATE conversion_jobs SET status = 'completed', lease_expires_at = NULL, lease_token = NULL, last_error = NULL, completed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND status = 'processing' AND lease_token = ?")
            .bind(&job.id)
            .bind(&job.lease_token)
            .execute(&mut *transaction)
            .await
            .map_err(conversion_repository_error)?;
        if completed.rows_affected() != 1 {
            return Err(conversion_repository_error(std::io::Error::other(
                "conversion lease was lost",
            )));
        }
        sqlx::query(
            "UPDATE document_versions SET pdf_sha256 = ?, pdf_storage_key = ? WHERE id = ?",
        )
        .bind(&derivative.sha256)
        .bind(&relative_key)
        .bind(&job.document_version_id)
        .execute(&mut *transaction)
        .await
        .map_err(conversion_repository_error)?;
        transaction
            .commit()
            .await
            .map_err(conversion_repository_error)
    }

    async fn fail(&self, job: &ConversionJob, error: &str) -> Result<(), ConversionError> {
        sqlx::query("UPDATE conversion_jobs SET status = CASE WHEN attempts >= max_attempts THEN 'failed' ELSE 'queued' END, available_at = datetime('now', '+30 seconds'), lease_expires_at = NULL, lease_token = NULL, last_error = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND status = 'processing' AND lease_token = ?")
            .bind(error)
            .bind(&job.id)
            .bind(&job.lease_token)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(conversion_repository_error)
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

    async fn credentials_by_username(
        &self,
        username: &str,
    ) -> Result<Option<UserCredentials>, AuthError> {
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, username, password_hash FROM users WHERE username = ? COLLATE NOCASE",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map(|row| {
            row.map(|(id, username, password_hash)| UserCredentials {
                id,
                username,
                password_hash,
            })
        })
        .map_err(auth_repository_error)
    }

    async fn create_session(&self, session: NewSession) -> Result<(), AuthError> {
        sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?, ?, ?)")
            .bind(session.token_hash)
            .bind(session.user_id)
            .bind(session.expires_at)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(auth_repository_error)
    }

    async fn user_by_session(
        &self,
        token_hash: &str,
        now: i64,
    ) -> Result<Option<AuthenticatedUser>, AuthError> {
        sqlx::query_as::<_, (String, String, String)>(
            "SELECT users.id, users.username, users.role FROM sessions JOIN users ON users.id = sessions.user_id WHERE sessions.token_hash = ? AND sessions.expires_at > ?",
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map(|row| {
            row.map(|(id, username, role)| AuthenticatedUser { id, username, role })
        })
        .map_err(auth_repository_error)
    }

    async fn delete_session(&self, token_hash: &str) -> Result<(), AuthError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(auth_repository_error)
    }
}

fn repository_error(error: sqlx::Error) -> ApplicationError {
    ApplicationError::Repository(Box::new(error))
}

fn auth_repository_error(error: sqlx::Error) -> AuthError {
    AuthError::Repository(Box::new(error))
}

fn import_repository_error(error: impl std::error::Error + Send + Sync + 'static) -> ImportError {
    ImportError::Repository(Box::new(error))
}

fn conversion_repository_error(
    error: impl std::error::Error + Send + Sync + 'static,
) -> ConversionError {
    ConversionError::Repository(Box::new(error))
}

fn parse_conversion_status(status: &str) -> ConversionStatus {
    match status {
        "processing" => ConversionStatus::Processing,
        "ready" => ConversionStatus::Ready,
        "failed" => ConversionStatus::Failed,
        _ => ConversionStatus::Queued,
    }
}

fn catalog_repository_error(error: sqlx::Error) -> CatalogError {
    CatalogError::Repository(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use elrond_domain::imports::PreparedDocument;

    #[tokio::test]
    async fn commits_hierarchy_documents_and_immutable_originals() {
        let directory = tempfile::tempdir().expect("temporary data directory should be created");
        let database_path = directory
            .path()
            .join("elrond-test.db")
            .to_string_lossy()
            .replace('\\', "/");
        let database_url = format!("sqlite://{database_path}?mode=rwc");
        let repository = SqliteLibraryRepository::connect(&database_url, directory.path())
            .await
            .expect("test repository should connect");
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role) VALUES ('tester', 'tester', 'fixture', 'admin')",
        )
        .execute(&repository.pool)
        .await
        .expect("test user should be inserted");
        let sha256 = "a".repeat(64);
        let import = PreparedImport {
            categories: vec![
                vec!["Policies".into()],
                vec!["Policies".into(), "HR".into()],
            ],
            documents: vec![PreparedDocument {
                category_path: vec!["Policies".into(), "HR".into()],
                filename: "leave.txt".into(),
                title: "leave".into(),
                media_type: "text/plain".into(),
                sha256: sha256.clone(),
                content: b"Controlled leave policy".to_vec(),
            }],
            unsupported_skipped: 0,
        };

        let summary = repository
            .commit_import(import, "tester")
            .await
            .expect("valid import should commit");

        assert_eq!(summary.categories_created, 2);
        assert_eq!(summary.documents_imported, 1);
        let documents = repository
            .list_documents()
            .await
            .expect("document catalog should load");
        let categories = repository
            .list_categories()
            .await
            .expect("category catalog should load");
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].title, "leave");
        assert_eq!(documents[0].category_name.as_deref(), Some("HR"));
        assert_eq!(documents[0].conversion_status, ConversionStatus::Queued);
        assert_eq!(categories.len(), 2);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM document_versions")
                .fetch_one(&repository.pool)
                .await
                .expect("version count should load"),
            1
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audit_events")
                .fetch_one(&repository.pool)
                .await
                .expect("audit count should load"),
            1
        );
        assert!(
            directory
                .path()
                .join("originals")
                .join("aa")
                .join(sha256)
                .is_file()
        );

        let job = repository
            .claim_next()
            .await
            .expect("conversion job should be claimable")
            .expect("text import should enqueue conversion");
        assert_eq!(job.original_media_type, "text/plain");
        assert_eq!(
            repository
                .load_original(&job)
                .await
                .expect("original should load"),
            b"Controlled leave policy"
        );
        let pdf = b"%PDF-1.7\nfixture".to_vec();
        repository
            .complete(
                &job,
                PdfDerivative {
                    sha256: "b".repeat(64),
                    content: pdf,
                },
            )
            .await
            .expect("conversion should complete");
        let converted = repository
            .list_documents()
            .await
            .expect("converted catalog should load");
        assert_eq!(converted[0].conversion_status, ConversionStatus::Ready);
        assert!(converted[0].has_pdf);
    }
}
