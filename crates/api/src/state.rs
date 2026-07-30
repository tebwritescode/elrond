//! Shared handler state.

use std::sync::Arc;

use elrond_application::auth::AuthService;
use elrond_application::ports::{SessionPolicy, SessionTokens};

use crate::config::ApiConfig;
use crate::rate_limit::RateLimiter;

/// Everything handlers need, cheap to clone.
#[derive(Clone)]
pub struct AppState {
    /// Authentication use cases.
    pub auth: AuthService,
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

impl AppState {
    /// Assembles the state.
    pub fn new(
        auth: AuthService,
        tokens: Arc<dyn SessionTokens>,
        config: ApiConfig,
        session_policy: SessionPolicy,
    ) -> Self {
        Self {
            auth,
            tokens,
            config: Arc::new(config),
            limiter: Arc::new(RateLimiter::new()),
            session_policy,
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}
