//! HTTP-layer configuration.

use std::path::PathBuf;
use std::time::Duration;

/// Settings the HTTP layer needs at request time.
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// Origin the browser reaches Elrond on, used for the CSRF origin check.
    ///
    /// Stored without a trailing slash.
    pub public_origin: String,

    /// Whether to mark cookies `Secure`.
    ///
    /// Must be true behind TLS. It is configurable rather than always-on because
    /// a `Secure` cookie is silently dropped over plain HTTP, which would make
    /// local development impossible to debug.
    pub secure_cookies: bool,

    /// Extra origins accepted by the CSRF origin check.
    ///
    /// Needed in development, where the browser sits on the Vite dev server's
    /// origin and Vite forwards the original `Origin` header when it proxies to
    /// the API. Empty in production.
    pub additional_allowed_origins: Vec<String>,

    /// Whether to believe `X-Forwarded-For` when identifying a client.
    ///
    /// Off by default: with no reverse proxy in front, the header is
    /// attacker-controlled and would let anyone bypass rate limiting by varying
    /// it. Turn it on only when a trusted proxy always overwrites it.
    pub trust_forwarded_for: bool,

    /// Directory holding the built frontend, if it has been built.
    pub web_dir: Option<PathBuf>,

    /// Largest accepted request body.
    pub max_body_bytes: usize,

    /// How many credential attempts an address gets per window.
    pub auth_attempt_limit: u32,

    /// Length of the credential rate-limit window.
    pub auth_attempt_window: Duration,
}

impl ApiConfig {
    /// Builds a configuration suitable for local development.
    pub fn development() -> Self {
        Self {
            public_origin: "http://localhost:3100".to_owned(),
            secure_cookies: false,
            additional_allowed_origins: Vec::new(),
            trust_forwarded_for: false,
            web_dir: None,
            max_body_bytes: 256 * 1024 * 1024,
            auth_attempt_limit: 10,
            auth_attempt_window: Duration::from_secs(300),
        }
    }

    /// Normalizes a configured public URL into an origin with no trailing slash.
    #[must_use]
    pub fn with_public_url(mut self, url: &str) -> Self {
        url.trim_end_matches('/')
            .clone_into(&mut self.public_origin);
        self
    }

    /// Whether `origin` is permitted to make state-changing requests.
    pub fn allows_origin(&self, origin: &str) -> bool {
        let origin = origin.trim_end_matches('/');
        origin == self.public_origin
            || self
                .additional_allowed_origins
                .iter()
                .any(|allowed| allowed.trim_end_matches('/') == origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_origin_is_allowed() {
        let config = ApiConfig::development().with_public_url("https://elrond.example.org/");
        assert!(config.allows_origin("https://elrond.example.org"));
        assert!(config.allows_origin("https://elrond.example.org/"));
    }

    #[test]
    fn other_origins_are_refused_by_default() {
        let config = ApiConfig::development().with_public_url("https://elrond.example.org");
        assert!(!config.allows_origin("https://attacker.example.net"));
        // A prefix match would be a real vulnerability here.
        assert!(!config.allows_origin("https://elrond.example.org.attacker.net"));
    }

    #[test]
    fn additional_origins_are_honoured() {
        let config = ApiConfig {
            additional_allowed_origins: vec!["http://localhost:5273".to_owned()],
            ..ApiConfig::development()
        };
        assert!(config.allows_origin("http://localhost:5273"));
        assert!(!config.allows_origin("http://localhost:5274"));
    }
}
