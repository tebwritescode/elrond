//! Shared handler state.

use std::sync::Arc;

use elrond_application::auth::AuthService;
use elrond_application::categories::CategoryService;
use elrond_application::documents::DocumentService;
use elrond_application::ports::{SessionPolicy, SessionTokens, TagRepository};

use crate::config::ApiConfig;
use crate::rate_limit::RateLimiter;

/// Everything handlers need, cheap to clone.
#[derive(Clone)]
pub struct AppState {
    /// Authentication use cases.
    pub auth: AuthService,
    /// Category tree use cases.
    pub categories: CategoryService,
    /// Document ingestion and retrieval use cases.
    pub documents: DocumentService,
    /// Tag listing.
    ///
    /// Reached directly rather than through a use case: listing tags has no
    /// business rules beyond being signed in.
    pub tags: Arc<dyn TagRepository>,
    /// Opaque token generator.
    ///
    /// Shared with the session layer rather than duplicated, so CSRF tokens come
    /// from the same audited CSPRNG as session tokens.
    pub tokens: Arc<dyn SessionTokens>,
    /// HTTP-layer settings.
    pub config: Arc<ApiConfig>,
    /// Credential-endpoint throttling.
    pub limiter: Arc<RateLimiter>,
    /// Session lifetimes, used to set cookie expiry.
    pub session_policy: SessionPolicy,
    /// Version reported by the health endpoint.
    pub version: &'static str,
}

/// Everything the composition root has to supply to build [`AppState`].
///
/// A struct rather than a long positional argument list, so adding a service does
/// not silently reorder the existing ones at every call site.
pub struct AppServices {
    /// Authentication use cases.
    pub auth: AuthService,
    /// Category tree use cases.
    pub categories: CategoryService,
    /// Document use cases.
    pub documents: DocumentService,
    /// Tag storage.
    pub tags: Arc<dyn TagRepository>,
    /// Opaque token generator.
    pub tokens: Arc<dyn SessionTokens>,
}

impl AppState {
    /// Assembles the state.
    pub fn new(services: AppServices, config: ApiConfig, session_policy: SessionPolicy) -> Self {
        Self {
            auth: services.auth,
            categories: services.categories,
            documents: services.documents,
            tags: services.tags,
            tokens: services.tokens,
            config: Arc::new(config),
            limiter: Arc::new(RateLimiter::new()),
            session_policy,
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}
