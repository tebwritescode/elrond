//! Environment-driven configuration.
//!
//! Every setting is read in one place and validated at startup, so a
//! misconfiguration surfaces as a clear message before the port is bound rather
//! than as a confusing failure on the first request.

use std::env::{self, VarError};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use elrond_api::ApiConfig;
use elrond_application::ports::SessionPolicy;

/// Resolved process configuration.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Address to bind.
    pub bind_address: SocketAddr,
    /// Directory holding the database, documents, and generated output.
    pub data_dir: PathBuf,
    /// SQLite connection URL.
    pub database_url: String,
    /// Largest single stored file, in bytes.
    ///
    /// Enforced by the blob store as well as by the HTTP body limit, so a future
    /// non-HTTP ingestion path (the ZIP importer) is bounded too.
    pub max_blob_bytes: u64,
    /// HTTP-layer settings.
    pub api: ApiConfig,
    /// Session lifetimes.
    pub session_policy: SessionPolicy,
    /// Base URL of the external Stirling-PDF service, if configured.
    pub stirling_url: Option<String>,
    /// Whether a Stirling API key was supplied.
    ///
    /// Only the presence is recorded here; the key itself is read where it is
    /// used, so it never sits in a struct that might be logged.
    pub stirling_api_key_present: bool,
}

impl Settings {
    /// Reads configuration from the environment, applying defaults.
    ///
    /// A `.env` file in the working directory is loaded first if present, and
    /// never overrides a variable that is already set.
    pub fn from_env() -> Result<Self> {
        // Absence is the normal case in production; only a real read error matters.
        match dotenvy::dotenv() {
            Ok(path) => tracing::debug!(path = %path.display(), "loaded environment file"),
            Err(error) if error.not_found() => {}
            Err(error) => tracing::warn!(%error, "could not read .env"),
        }

        let bind_address: SocketAddr = optional("ELROND_BIND_ADDRESS")?
            // 127.0.0.1 rather than 0.0.0.0: a document library should not become
            // reachable on every interface because someone forgot to configure it.
            // Container deployments set this explicitly.
            .unwrap_or_else(|| "127.0.0.1:3100".to_owned())
            .parse()
            .context("ELROND_BIND_ADDRESS must be an address like 127.0.0.1:3100")?;

        let data_dir =
            PathBuf::from(optional("ELROND_DATA_DIR")?.unwrap_or_else(|| "./dev-data".to_owned()));

        let database_url = optional("ELROND_DATABASE_URL")?.unwrap_or_else(|| {
            // Derived from the data directory so a single ELROND_DATA_DIR is
            // enough to relocate all persistent state.
            let path = data_dir.join("elrond.db");
            format!("sqlite://{}?mode=rwc", path.to_string_lossy())
        });

        let public_url =
            optional("ELROND_PUBLIC_URL")?.unwrap_or_else(|| format!("http://{bind_address}"));

        let secure_cookies = flag("ELROND_SECURE_COOKIES", false)?;
        let trust_forwarded_for = flag("ELROND_TRUST_FORWARDED_FOR", false)?;

        let additional_allowed_origins = optional("ELROND_ALLOWED_ORIGINS")?
            .map(|raw| {
                raw.split(',')
                    .map(|origin| origin.trim().trim_end_matches('/').to_owned())
                    .filter(|origin| !origin.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let web_dir = optional("ELROND_WEB_DIR")?.map(PathBuf::from).or_else(|| {
            // Convenience for `cargo run` from a checkout: pick the built client
            // up automatically if it exists.
            let candidate = PathBuf::from("web/dist");
            candidate.join("index.html").is_file().then_some(candidate)
        });

        let max_body_bytes = number("ELROND_MAX_UPLOAD_BYTES", 256 * 1024 * 1024)?;
        if max_body_bytes == 0 {
            bail!("ELROND_MAX_UPLOAD_BYTES must be greater than zero");
        }

        let api = ApiConfig {
            secure_cookies,
            additional_allowed_origins,
            trust_forwarded_for,
            web_dir,
            max_body_bytes,
            auth_attempt_limit: number("ELROND_AUTH_ATTEMPT_LIMIT", 10)?
                .try_into()
                .context("ELROND_AUTH_ATTEMPT_LIMIT is too large")?,
            auth_attempt_window: Duration::from_secs(number(
                "ELROND_AUTH_ATTEMPT_WINDOW_SECONDS",
                300,
            )? as u64),
            ..ApiConfig::development()
        }
        .with_public_url(&public_url);

        if secure_cookies && public_url.starts_with("http://") {
            // A Secure cookie is silently dropped over plain HTTP, which presents
            // as "sign-in does nothing" with no error anywhere.
            tracing::warn!(
                "ELROND_SECURE_COOKIES is on but ELROND_PUBLIC_URL is http://; \
                 browsers will discard the session cookie"
            );
        }

        let stirling_url =
            optional("STIRLING_URL")?.map(|url| url.trim_end_matches('/').to_owned());

        Ok(Self {
            bind_address,
            data_dir,
            database_url,
            max_blob_bytes: max_body_bytes as u64,
            api,
            session_policy: SessionPolicy::default(),
            stirling_url,
            stirling_api_key_present: optional("STIRLING_API_KEY")?
                .is_some_and(|key| !key.is_empty()),
        })
    }
}

/// Reads an optional variable, treating an empty value as absent.
fn optional(key: &str) -> Result<Option<String>> {
    match env::var(key) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(VarError::NotPresent) => Ok(None),
        Err(VarError::NotUnicode(_)) => bail!("{key} is not valid UTF-8"),
    }
}

/// Reads a boolean variable, accepting the spellings people actually type.
fn flag(key: &str, default: bool) -> Result<bool> {
    match optional(key)? {
        None => Ok(default),
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => bail!("{key} must be true or false, got {other:?}"),
        },
    }
}

/// Reads a numeric variable.
fn number(key: &str, default: usize) -> Result<usize> {
    match optional(key)? {
        None => Ok(default),
        Some(value) => value
            .parse()
            .with_context(|| format!("{key} must be a whole number, got {value:?}")),
    }
}
