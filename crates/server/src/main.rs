//! Elrond application binary.
//!
//! One process serves the API, the built frontend, and background work. This
//! file is the only place that knows which concrete adapter satisfies which port,
//! which is what keeps the layers below it substitutable.

mod settings;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use elrond_api::state::AppServices;
use elrond_api::{AppState, router};
use elrond_application::auth::AuthService;
use elrond_application::binders::BinderService;
use elrond_application::categories::CategoryService;
use elrond_application::documents::DocumentService;
use elrond_infrastructure::{
    Argon2idHasher, Database, DatabaseSettings, FilesystemBlobStore, MagicByteInspector,
    NativeBinderRenderer, RandomSessionTokens, SqliteCategoryRepository, SqliteDocumentRepository,
    SqliteSearchIndex, SqliteSessionRepository, SqliteTagRepository, SqliteUserRepository,
    SystemClock, ZipExtractor,
};
use settings::Settings;
use tokio::signal;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// How often expired sessions are swept from the database.
const SESSION_SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let settings = Settings::from_env().context("invalid configuration")?;
    log_startup(&settings);

    std::fs::create_dir_all(&settings.data_dir).with_context(|| {
        format!(
            "could not create the data directory at {}",
            settings.data_dir.display()
        )
    })?;

    let database = Database::connect(&DatabaseSettings::new(settings.database_url.clone()))
        .await
        .context("could not open the database")?;

    // The composition root: ports on the left, adapters on the right. This is the
    // only file that knows which concrete type satisfies which trait.
    let users = Arc::new(SqliteUserRepository::new(&database));
    let sessions = Arc::new(SqliteSessionRepository::new(&database));
    let categories_repo = Arc::new(SqliteCategoryRepository::new(&database));
    let categories_repo_for_binders = categories_repo.clone();
    let documents_repo = Arc::new(SqliteDocumentRepository::new(&database));
    let tags_repo = Arc::new(SqliteTagRepository::new(&database));
    let search = Arc::new(SqliteSearchIndex::new(&database));
    let blobs = Arc::new(FilesystemBlobStore::new(
        settings.data_dir.clone(),
        settings.max_blob_bytes,
    ));
    let inspector = Arc::new(MagicByteInspector);
    let hasher = Arc::new(Argon2idHasher::new());
    let tokens = Arc::new(RandomSessionTokens);
    let clock = Arc::new(SystemClock);

    let auth = AuthService::new(
        users,
        sessions,
        hasher,
        tokens.clone(),
        clock.clone(),
        settings.session_policy,
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
        inspector,
        search,
        categories.clone(),
        Arc::new(ZipExtractor),
        clock,
    );

    let state = AppState::new(
        AppServices {
            auth: auth.clone(),
            categories,
            binders,
            documents,
            tags: tags_repo,
            tokens,
        },
        settings.api.clone(),
        settings.session_policy,
    );

    // Seed the first administrator from configuration, before the listener
    // opens: either the account exists when the first request arrives, or the
    // process has refused to start with a clear reason. A bad seed must not
    // boot into the setup screen the operator configured it to bypass.
    if let Some(admin) = &settings.initial_admin {
        let created = auth
            .seed_first_admin(&admin.username, &admin.password)
            .await
            .context("could not create the administrator from ELROND_ADMIN_USERNAME")?;
        if !created {
            tracing::info!("accounts already exist; ELROND_ADMIN_USERNAME was left untouched");
        }
    }

    let sweeper = tokio::spawn(sweep_expired_sessions(auth));

    let listener = tokio::net::TcpListener::bind(settings.bind_address)
        .await
        .with_context(|| format!("could not bind {}", settings.bind_address))?;
    let bound = listener.local_addr().unwrap_or(settings.bind_address);
    report_ready(&settings, bound);

    // `into_make_service_with_connect_info` is what gives handlers the peer
    // address, which the rate limiter needs to tell clients apart.
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("the HTTP server stopped unexpectedly")?;

    sweeper.abort();
    // Flushes the WAL so a restart does not have to recover it.
    database.close().await;
    tracing::info!("shutdown complete");
    Ok(())
}

/// Installs the log subscriber.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,elrond_api=debug,elrond_server=debug,tower_http=info")
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                // Local time would need an offset lookup that is unsound in a
                // multi-threaded process, and UTC logs are easier to correlate
                // across hosts anyway.
                .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339()),
        )
        .init();
}

/// Records the effective configuration, without secrets.
fn log_startup(settings: &Settings) {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        data_dir = %settings.data_dir.display(),
        public_origin = %settings.api.public_origin,
        secure_cookies = settings.api.secure_cookies,
        trust_forwarded_for = settings.api.trust_forwarded_for,
        web_dir = settings.api.web_dir.as_ref().map(|dir| dir.display().to_string()),
        stirling_url = settings.stirling_url.as_deref().unwrap_or("<unset>"),
        // The key's value is never logged, only whether one was supplied.
        stirling_api_key = if settings.stirling_api_key_present { "set" } else { "unset" },
        "starting Elrond"
    );
}

/// Prints where to point a browser.
fn report_ready(settings: &Settings, bound: SocketAddr) {
    tracing::info!(%bound, "listening");
    if settings.api.web_dir.is_none() {
        tracing::info!(
            "no built frontend configured; run `npm run dev` in web/ and open http://localhost:5273"
        );
    }
}

/// Periodically removes expired sessions.
///
/// Expiry is already enforced on every request, so this is housekeeping rather
/// than a security control: it keeps the table from accumulating dead rows.
async fn sweep_expired_sessions(auth: AuthService) {
    let mut ticker = tokio::time::interval(SESSION_SWEEP_INTERVAL);
    // The first tick fires immediately, clearing anything left by a previous run.
    loop {
        ticker.tick().await;
        match auth.purge_expired_sessions().await {
            Ok(0) => {}
            Ok(purged) => tracing::debug!(purged, "removed expired sessions"),
            Err(error) => tracing::warn!(%error, "could not sweep expired sessions"),
        }
    }
}

/// Resolves when the process is asked to stop.
async fn shutdown_signal() {
    let interrupt = async {
        signal::ctrl_c()
            .await
            .expect("failed to listen for interrupt");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to listen for SIGTERM")
            .recv()
            .await;
    };

    // Windows has no SIGTERM; Ctrl-C is the only signal to wait on.
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => tracing::info!("interrupt received, shutting down"),
        () = terminate => tracing::info!("termination signal received, shutting down"),
    }
}
