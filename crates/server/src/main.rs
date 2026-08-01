use std::{env, net::SocketAddr, path::Path, sync::Arc};

use axum::Router;
use elrond_api::ApiState;
use elrond_application::{
    AuthError, AuthService, BinderService, CatalogService, ConversionService, ImportService,
    LibraryService,
};
use elrond_infrastructure::{
    binders::LopdfBinderRenderer, sqlite::SqliteLibraryRepository, stirling::StirlingPdfConverter,
};
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
    let repository = Arc::new(SqliteLibraryRepository::connect(&database_url, &data_dir).await?);
    let stirling_url = env::var("STIRLING_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let stirling_configured = stirling_url.is_some();
    let library = LibraryService::new(repository.clone(), stirling_configured);
    let auth = AuthService::new(repository.clone());
    let bootstrap = bootstrap_credentials(
        non_empty_env("ELROND_ADMIN_USERNAME"),
        non_empty_env("ELROND_ADMIN_PASSWORD"),
    )?;
    if let Some((username, password)) = bootstrap
        && library.overview().await?.setup_required
    {
        match auth.create_initial_admin(&username, &password).await {
            Ok(session) => {
                auth.logout(&session.token).await?;
                tracing::info!(username = %username, "initial administrator created from environment");
            }
            Err(AuthError::SetupCompleted) => {}
            Err(error) => return Err(error.into()),
        }
    }
    let imports = ImportService::new(repository.clone());
    let catalog = CatalogService::new(repository.clone());
    let binders = BinderService::new(repository.clone(), Arc::new(LopdfBinderRenderer));
    if let Some(stirling_url) = stirling_url {
        let api_key = env::var("STIRLING_API_KEY").ok();
        let converter = Arc::new(StirlingPdfConverter::new(&stirling_url, api_key)?);
        let conversions = ConversionService::new(repository.clone(), converter);
        tokio::spawn(async move {
            loop {
                match conversions.run_one().await {
                    Ok(true) => {}
                    Ok(false) => tokio::time::sleep(std::time::Duration::from_secs(3)).await,
                    Err(error) => {
                        tracing::error!(%error, "conversion worker failed");
                        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                    }
                }
            }
        });
    }
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
            imports,
            catalog,
            binders,
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

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn bootstrap_credentials(
    username: Option<String>,
    password: Option<String>,
) -> Result<Option<(String, String)>, std::io::Error> {
    match (username, password) {
        (None, None) => Ok(None),
        (Some(username), Some(password)) => Ok(Some((username, password))),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "ELROND_ADMIN_USERNAME and ELROND_ADMIN_PASSWORD must be set together",
        )),
    }
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

#[cfg(test)]
mod tests {
    use super::bootstrap_credentials;

    #[test]
    fn bootstrap_requires_both_environment_values() {
        assert!(bootstrap_credentials(None, None).unwrap().is_none());
        assert!(bootstrap_credentials(Some("admin".into()), None).is_err());
        assert!(bootstrap_credentials(None, Some("secret".into())).is_err());
        assert_eq!(
            bootstrap_credentials(Some("admin".into()), Some("long password".into())).unwrap(),
            Some(("admin".into(), "long password".into()))
        );
    }
}
