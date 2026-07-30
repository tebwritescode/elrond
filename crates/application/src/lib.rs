use std::{
    collections::BTreeSet,
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use async_trait::async_trait;
use elrond_domain::{
    auth::{AuthenticatedUser, InitialAdmin, NewSession, UserCredentials},
    catalog::{CategorySummary, DocumentSummary},
    imports::{ImportSummary, PreparedDocument, PreparedImport},
    library::LibraryOverview,
};
use rand::TryRngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;
use zip::ZipArchive;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("the library repository could not complete the operation")]
    Repository(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("the username must contain 3 to 64 letters, numbers, dots, dashes, or underscores")]
    InvalidUsername,
    #[error("the password must contain 12 to 128 characters")]
    InvalidPassword,
    #[error("first-run setup has already been completed")]
    SetupCompleted,
    #[error("the username or password is incorrect")]
    InvalidCredentials,
    #[error("password hashing failed")]
    PasswordHash,
    #[error("secure session generation failed")]
    SessionGeneration,
    #[error("account storage failed")]
    Repository(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("the ZIP archive is empty or invalid")]
    InvalidArchive,
    #[error("the ZIP archive contains too many entries")]
    TooManyEntries,
    #[error("the ZIP archive expands beyond the 1 GiB safety limit")]
    ExpandedSizeLimit,
    #[error("an archived file exceeds the 100 MiB safety limit")]
    FileSizeLimit,
    #[error("the ZIP archive contains an unsafe path or symbolic link")]
    UnsafeEntry,
    #[error("an archived file's content does not match its supported file type")]
    InvalidFileType,
    #[error("the ZIP archive contains folders nested more than 20 levels")]
    DepthLimit,
    #[error("archive extraction failed")]
    Extraction,
    #[error("the import could not be stored")]
    Repository(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("the document catalog could not be loaded")]
    Repository(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait]
pub trait LibraryRepository: Send + Sync {
    async fn overview(
        &self,
        stirling_configured: bool,
    ) -> Result<LibraryOverview, ApplicationError>;
}

#[async_trait]
pub trait AuthRepository: Send + Sync {
    async fn create_initial_admin(
        &self,
        admin: InitialAdmin,
        session: NewSession,
    ) -> Result<(), AuthError>;
    async fn credentials_by_username(
        &self,
        username: &str,
    ) -> Result<Option<UserCredentials>, AuthError>;
    async fn create_session(&self, session: NewSession) -> Result<(), AuthError>;
    async fn user_by_session(
        &self,
        token_hash: &str,
        now: i64,
    ) -> Result<Option<AuthenticatedUser>, AuthError>;
    async fn delete_session(&self, token_hash: &str) -> Result<(), AuthError>;
}

#[async_trait]
pub trait ImportRepository: Send + Sync {
    async fn commit_import(
        &self,
        import: PreparedImport,
        actor_user_id: &str,
    ) -> Result<ImportSummary, ImportError>;
}

#[async_trait]
pub trait CatalogRepository: Send + Sync {
    async fn list_documents(&self) -> Result<Vec<DocumentSummary>, CatalogError>;
    async fn list_categories(&self) -> Result<Vec<CategorySummary>, CatalogError>;
}

#[derive(Clone)]
pub struct LibraryService {
    repository: Arc<dyn LibraryRepository>,
    stirling_configured: bool,
}

pub struct CreatedSession {
    pub token: String,
    pub username: String,
    pub max_age_seconds: i64,
}

#[derive(Clone)]
pub struct AuthService {
    repository: Arc<dyn AuthRepository>,
}

#[derive(Clone)]
pub struct ImportService {
    repository: Arc<dyn ImportRepository>,
}

#[derive(Clone)]
pub struct CatalogService {
    repository: Arc<dyn CatalogRepository>,
}

impl CatalogService {
    pub fn new(repository: Arc<dyn CatalogRepository>) -> Self {
        Self { repository }
    }

    pub async fn documents(&self) -> Result<Vec<DocumentSummary>, CatalogError> {
        self.repository.list_documents().await
    }

    pub async fn categories(&self) -> Result<Vec<CategorySummary>, CatalogError> {
        self.repository.list_categories().await
    }
}

impl ImportService {
    const MAX_ENTRIES: usize = 10_000;
    const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
    const MAX_EXPANDED_BYTES: u64 = 1024 * 1024 * 1024;
    const MAX_DEPTH: usize = 20;

    pub fn new(repository: Arc<dyn ImportRepository>) -> Self {
        Self { repository }
    }

    pub async fn import_zip(
        &self,
        archive_bytes: Vec<u8>,
        root_category: &str,
        actor_user_id: &str,
    ) -> Result<ImportSummary, ImportError> {
        let root_category = root_category.to_owned();
        let prepared =
            tokio::task::spawn_blocking(move || Self::prepare_zip(archive_bytes, &root_category))
                .await
                .map_err(|_| ImportError::Extraction)??;
        self.repository.commit_import(prepared, actor_user_id).await
    }

    fn prepare_zip(
        archive_bytes: Vec<u8>,
        root_category: &str,
    ) -> Result<PreparedImport, ImportError> {
        let mut archive =
            ZipArchive::new(Cursor::new(archive_bytes)).map_err(|_| ImportError::InvalidArchive)?;
        if archive.is_empty() {
            return Err(ImportError::InvalidArchive);
        }
        if archive.len() > Self::MAX_ENTRIES {
            return Err(ImportError::TooManyEntries);
        }

        let root_category = clean_category_name(root_category).unwrap_or_else(|| "Imported".into());
        let mut expanded_bytes = 0_u64;
        let mut categories = BTreeSet::new();
        let mut documents = Vec::new();
        let mut unsupported_skipped = 0;

        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|_| ImportError::Extraction)?;
            let path = entry
                .enclosed_name()
                .ok_or(ImportError::UnsafeEntry)?
                .to_path_buf();
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
                || entry.encrypted()
            {
                return Err(ImportError::UnsafeEntry);
            }

            let components: Vec<String> = path
                .components()
                .filter_map(|component| {
                    clean_category_name(&component.as_os_str().to_string_lossy())
                })
                .collect();
            if components.len() > Self::MAX_DEPTH + 1 {
                return Err(ImportError::DepthLimit);
            }
            if components.first().is_some_and(|name| name == "__MACOSX") {
                continue;
            }

            if entry.is_dir() {
                if !components.is_empty() {
                    categories.insert(components);
                }
                continue;
            }

            let filename = components.last().cloned().ok_or(ImportError::UnsafeEntry)?;
            let extension = Path::new(&filename)
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_default();
            let Some(media_type) = supported_media_type(&extension) else {
                unsupported_skipped += 1;
                continue;
            };
            if entry.size() > Self::MAX_FILE_BYTES {
                return Err(ImportError::FileSizeLimit);
            }
            if entry.size() > 10 * 1024 * 1024
                && entry.compressed_size() > 0
                && entry.size() / entry.compressed_size() > 200
            {
                return Err(ImportError::ExpandedSizeLimit);
            }
            expanded_bytes = expanded_bytes
                .checked_add(entry.size())
                .ok_or(ImportError::ExpandedSizeLimit)?;
            if expanded_bytes > Self::MAX_EXPANDED_BYTES {
                return Err(ImportError::ExpandedSizeLimit);
            }

            let mut category_path = components[..components.len() - 1].to_vec();
            if category_path.is_empty() {
                category_path.push(root_category.clone());
            }
            for depth in 1..=category_path.len() {
                categories.insert(category_path[..depth].to_vec());
            }

            let mut content = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut content)
                .map_err(|_| ImportError::Extraction)?;
            if !content_matches_extension(&extension, &content) {
                return Err(ImportError::InvalidFileType);
            }
            let sha256 = hex::encode(Sha256::digest(&content));
            let title = Path::new(&filename)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(&filename)
                .replace(['_', '-'], " ");
            documents.push(PreparedDocument {
                category_path,
                filename,
                title,
                media_type: media_type.into(),
                sha256,
                content,
            });
        }

        Ok(PreparedImport {
            categories: categories.into_iter().collect(),
            documents,
            unsupported_skipped,
        })
    }
}

fn clean_category_name(name: &str) -> Option<String> {
    let name = name.trim().trim_matches('.').trim();
    (!name.is_empty()).then(|| name.chars().take(120).collect())
}

fn supported_media_type(extension: &str) -> Option<&'static str> {
    match extension {
        "pdf" => Some("application/pdf"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        "odt" => Some("application/vnd.oasis.opendocument.text"),
        "ods" => Some("application/vnd.oasis.opendocument.spreadsheet"),
        "odp" => Some("application/vnd.oasis.opendocument.presentation"),
        "txt" => Some("text/plain"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "tif" | "tiff" => Some("image/tiff"),
        _ => None,
    }
}

fn content_matches_extension(extension: &str, content: &[u8]) -> bool {
    match extension {
        "pdf" => content.starts_with(b"%PDF-"),
        "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => content.starts_with(b"PK"),
        "jpg" | "jpeg" => content.starts_with(&[0xff, 0xd8, 0xff]),
        "png" => content.starts_with(b"\x89PNG\r\n\x1a\n"),
        "tif" | "tiff" => content.starts_with(b"II*\0") || content.starts_with(b"MM\0*"),
        "txt" => std::str::from_utf8(content).is_ok(),
        _ => false,
    }
}

impl LibraryService {
    pub fn new(repository: Arc<dyn LibraryRepository>, stirling_configured: bool) -> Self {
        Self {
            repository,
            stirling_configured,
        }
    }

    pub async fn overview(&self) -> Result<LibraryOverview, ApplicationError> {
        self.repository.overview(self.stirling_configured).await
    }
}

impl AuthService {
    const SESSION_DURATION_SECONDS: i64 = 7 * 24 * 60 * 60;

    pub fn new(repository: Arc<dyn AuthRepository>) -> Self {
        Self { repository }
    }

    pub async fn create_initial_admin(
        &self,
        username: &str,
        password: &str,
    ) -> Result<CreatedSession, AuthError> {
        let username = username.trim();
        if !(3..=64).contains(&username.len())
            || !username
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
        {
            return Err(AuthError::InvalidUsername);
        }
        if !(12..=128).contains(&password.chars().count()) {
            return Err(AuthError::InvalidPassword);
        }

        let mut rng = rand::rngs::OsRng;
        let mut salt_bytes = [0_u8; 16];
        rng.try_fill_bytes(&mut salt_bytes)
            .map_err(|_| AuthError::PasswordHash)?;
        let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| AuthError::PasswordHash)?;
        let password_hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| AuthError::PasswordHash)?
            .to_string();

        let user_id = Uuid::new_v4().to_string();
        let session = Self::new_session(&user_id)?;

        self.repository
            .create_initial_admin(
                InitialAdmin {
                    id: user_id.clone(),
                    username: username.to_owned(),
                    password_hash,
                },
                session.stored,
            )
            .await?;

        Ok(CreatedSession {
            token: session.token,
            username: username.to_owned(),
            max_age_seconds: Self::SESSION_DURATION_SECONDS,
        })
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<CreatedSession, AuthError> {
        let credentials = self
            .repository
            .credentials_by_username(username.trim())
            .await?
            .ok_or(AuthError::InvalidCredentials)?;
        let parsed_hash = PasswordHash::new(&credentials.password_hash)
            .map_err(|_| AuthError::InvalidCredentials)?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| AuthError::InvalidCredentials)?;

        let session = Self::new_session(&credentials.id)?;
        self.repository.create_session(session.stored).await?;

        Ok(CreatedSession {
            token: session.token,
            username: credentials.username,
            max_age_seconds: Self::SESSION_DURATION_SECONDS,
        })
    }

    pub async fn current_user(
        &self,
        session_token: &str,
    ) -> Result<Option<AuthenticatedUser>, AuthError> {
        self.repository
            .user_by_session(&hash_session_token(session_token), unix_timestamp()?)
            .await
    }

    pub async fn logout(&self, session_token: &str) -> Result<(), AuthError> {
        self.repository
            .delete_session(&hash_session_token(session_token))
            .await
    }

    fn new_session(user_id: &str) -> Result<GeneratedSession, AuthError> {
        let mut session_bytes = [0_u8; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut session_bytes)
            .map_err(|_| AuthError::SessionGeneration)?;
        let token = hex::encode(session_bytes);

        Ok(GeneratedSession {
            stored: NewSession {
                token_hash: hash_session_token(&token),
                user_id: user_id.to_owned(),
                expires_at: unix_timestamp()? + Self::SESSION_DURATION_SECONDS,
            },
            token,
        })
    }
}

struct GeneratedSession {
    token: String,
    stored: NewSession,
}

fn hash_session_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn unix_timestamp() -> Result<i64, AuthError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthError::SessionGeneration)?
        .as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Write},
        sync::{Arc, Mutex},
    };

    use super::*;
    use zip::{ZipWriter, write::SimpleFileOptions};

    #[derive(Default)]
    struct RecordingAuthRepository {
        created: Mutex<Option<(InitialAdmin, NewSession)>>,
    }

    #[async_trait]
    impl AuthRepository for RecordingAuthRepository {
        async fn create_initial_admin(
            &self,
            admin: InitialAdmin,
            session: NewSession,
        ) -> Result<(), AuthError> {
            self.created
                .lock()
                .expect("test repository lock should remain available")
                .replace((admin, session));
            Ok(())
        }

        async fn credentials_by_username(
            &self,
            _username: &str,
        ) -> Result<Option<UserCredentials>, AuthError> {
            Ok(None)
        }

        async fn create_session(&self, _session: NewSession) -> Result<(), AuthError> {
            Ok(())
        }

        async fn user_by_session(
            &self,
            _token_hash: &str,
            _now: i64,
        ) -> Result<Option<AuthenticatedUser>, AuthError> {
            Ok(None)
        }

        async fn delete_session(&self, _token_hash: &str) -> Result<(), AuthError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn rejects_invalid_initial_credentials() {
        let repository = Arc::new(RecordingAuthRepository::default());
        let service = AuthService::new(repository);

        assert!(matches!(
            service.create_initial_admin("a", &"x".repeat(12)).await,
            Err(AuthError::InvalidUsername)
        ));
        assert!(matches!(
            service.create_initial_admin("admin", "short").await,
            Err(AuthError::InvalidPassword)
        ));
    }

    #[tokio::test]
    async fn hashes_password_and_session_material_before_storage() {
        let repository = Arc::new(RecordingAuthRepository::default());
        let service = AuthService::new(repository.clone());

        let created = service
            .create_initial_admin("admin", &"x".repeat(12))
            .await
            .expect("valid setup should succeed");
        let stored = repository
            .created
            .lock()
            .expect("test repository lock should remain available")
            .clone()
            .expect("credentials should be recorded");

        assert_eq!(stored.0.username, "admin");
        assert!(stored.0.password_hash.starts_with("$argon2id$"));
        assert_eq!(created.token.len(), 64);
        assert_eq!(stored.1.token_hash.len(), 64);
        assert_ne!(created.token, stored.1.token_hash);
        assert_eq!(stored.0.id, stored.1.user_id);
    }

    #[test]
    fn zip_folders_become_nested_categories() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("Policies/HR/leave_policy.txt", SimpleFileOptions::default())
            .expect("test entry should start");
        writer
            .write_all(b"Controlled leave policy")
            .expect("test content should write");
        writer
            .start_file("root_note.txt", SimpleFileOptions::default())
            .expect("test entry should start");
        writer
            .write_all(b"Root note")
            .expect("test content should write");
        let bytes = writer
            .finish()
            .expect("test ZIP should finish")
            .into_inner();

        let prepared =
            ImportService::prepare_zip(bytes, "Inbox").expect("valid test ZIP should be prepared");

        assert!(prepared.categories.contains(&vec!["Policies".into()]));
        assert!(
            prepared
                .categories
                .contains(&vec!["Policies".into(), "HR".into()])
        );
        assert!(prepared.categories.contains(&vec!["Inbox".into()]));
        assert_eq!(prepared.documents.len(), 2);
        assert_eq!(prepared.documents[0].category_path, ["Policies", "HR"]);
        assert_eq!(prepared.documents[1].category_path, ["Inbox"]);
    }

    #[test]
    fn zip_import_rejects_content_disguised_as_pdf() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("Policies/not_a_pdf.pdf", SimpleFileOptions::default())
            .expect("test entry should start");
        writer
            .write_all(b"not actually a PDF")
            .expect("test content should write");
        let bytes = writer
            .finish()
            .expect("test ZIP should finish")
            .into_inner();

        assert!(matches!(
            ImportService::prepare_zip(bytes, "Imported"),
            Err(ImportError::InvalidFileType)
        ));
    }
}
