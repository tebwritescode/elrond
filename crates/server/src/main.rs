use std::{env, net::SocketAddr, path::Path, sync::Arc};

use axum::Router;
use elrond_api::ApiState;
use elrond_application::{AuthService, LibraryService};
use elrond_infrastructure::sqlite::SqliteLibraryRepository;
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    init_tracing();

    let data_dir = env::var("ELROND_DATA_DIR").unwrap_or_else(|_| "data".to_owned());
    tokio::fs::create_dir_all(&data_dir).await?;

    let database_url = env::var("ELROND_DATABASE_URL")
        .unwrap_or_else(|_| format!("sqlite://{data_dir}/elrond.db?mode=rwc"));
    let repository = Arc::new(SqliteLibraryRepository::connect(&database_url).await?);
    let stirling_configured = env::var("STIRLING_URL")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let library = LibraryService::new(repository.clone(), stirling_configured);
    let auth = AuthService::new(repository);
    let secure_cookies = env::var("ELROND_SECURE_COOKIES")
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let web_dir = env::var("ELROND_WEB_DIR").unwrap_or_else(|_| "web/dist".to_owned());
    let index_path = Path::new(&web_dir).join("index.html");
    let static_files = ServeDir::new(&web_dir).fallback(ServeFile::new(index_path));

    let app = Router::new()
        .merge(elrond_api::router(ApiState {
            library,
            auth,
            secure_cookies,
        }))
        .fallback_service(static_files)
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    let bind_address: SocketAddr = env::var("ELROND_BIND_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:3000".to_owned())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(bind_address).await?;

    tracing::info!(address = %bind_address, "Elrond server is ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("elrond=info,tower_http=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install shutdown signal handler");
    }
}
