//! Shared harness for the HTTP integration tests.
//!
//! Builds the real router with the real adapters — SQLite, Argon2id, the CSPRNG,
//! and a filesystem blob store in a temporary directory — so the tests exercise
//! the same wiring the binary does.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use elrond_api::state::AppServices;
use elrond_api::{ApiConfig, AppState, router};
use elrond_application::auth::AuthService;
use elrond_application::binders::BinderService;
use elrond_application::categories::CategoryService;
use elrond_application::documents::DocumentService;
use elrond_application::ports::SessionPolicy;
use elrond_infrastructure::{
    Argon2idHasher, Database, FilesystemBlobStore, MagicByteInspector, NativeBinderRenderer,
    RandomSessionTokens, SqliteCategoryRepository, SqliteDocumentRepository, SqliteSearchIndex,
    SqliteSessionRepository, SqliteTagRepository, SqliteUserRepository, SystemClock, ZipExtractor,
};

/// Makes each test's blob directory unique, so tests can run concurrently.
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// A router plus the temporary directory its blobs live in.
///
/// The directory is kept alive alongside the router because dropping it would
/// remove content the router still serves.
pub struct TestApp {
    /// The assembled router.
    pub router: Router,
    /// Where uploaded content is written.
    #[allow(dead_code)]
    pub data_dir: PathBuf,
}

/// Builds an app with a specific HTTP configuration.
pub async fn build_with(config: ApiConfig) -> TestApp {
    let unique = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
    let data_dir =
        std::env::temp_dir().join(format!("elrond-test-{}-{unique}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_dir);
    std::fs::create_dir_all(&data_dir).expect("data directory");

    build_at(config, &data_dir).await
}

/// Builds an app whose content is stored under `data_dir`.
async fn build_at(config: ApiConfig, data_dir: &Path) -> TestApp {
    let database = Database::connect_in_memory()
        .await
        .expect("in-memory database");

    let users = Arc::new(SqliteUserRepository::new(&database));
    let sessions = Arc::new(SqliteSessionRepository::new(&database));
    let categories_repo = Arc::new(SqliteCategoryRepository::new(&database));
    let categories_repo_for_binders = categories_repo.clone();
    let documents_repo = Arc::new(SqliteDocumentRepository::new(&database));
    let tags_repo = Arc::new(SqliteTagRepository::new(&database));
    let search = Arc::new(SqliteSearchIndex::new(&database));
    let blobs = Arc::new(FilesystemBlobStore::new(
        data_dir.to_path_buf(),
        16 * 1024 * 1024,
    ));
    let tokens = Arc::new(RandomSessionTokens);
    let clock = Arc::new(SystemClock);

    let auth = AuthService::new(
        users,
        sessions,
        Arc::new(Argon2idHasher::new()),
        tokens.clone(),
        clock.clone(),
        SessionPolicy::default(),
    );
    let categories = CategoryService::new(categories_repo, documents_repo.clone(), clock.clone());
    let binders = BinderService::new(
        categories_repo_for_binders,
        documents_repo.clone(),
        blobs.clone(),
        Arc::new(NativeBinderRenderer),
        clock.clone(),
    );
    let documents = DocumentService::new(
        documents_repo,
        tags_repo.clone(),
        blobs,
        Arc::new(MagicByteInspector),
        search,
        categories.clone(),
        Arc::new(ZipExtractor),
        clock,
    );

    // The database handle is intentionally dropped here: the pool is reference
    // counted and each repository holds a clone, so the in-memory database stays
    // alive for as long as the router does.
    let state = AppState::new(
        AppServices {
            auth,
            categories,
            binders,
            documents,
            tags: tags_repo,
            tokens,
        },
        config,
        SessionPolicy::default(),
    );

    TestApp {
        router: router(state),
        data_dir: data_dir.to_path_buf(),
    }
}
