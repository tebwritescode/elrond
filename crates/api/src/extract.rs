//! Request extractors.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum_extra::extract::cookie::CookieJar;
use elrond_application::ApplicationError;
use elrond_application::auth::Authenticated;
use elrond_application::ports::SessionToken;
use elrond_domain::Role;

use crate::cookies::SESSION_COOKIE;
use crate::error::ApiError;
use crate::state::AppState;

/// An authenticated caller. Rejects with 401 when there is no valid session.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub Authenticated);

impl CurrentUser {
    /// Requires at least `role`, rejecting with 403 otherwise.
    pub fn require(&self, role: Role) -> Result<(), ApiError> {
        self.0.require_role(role).map_err(ApiError::from)
    }
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let MaybeUser(user) = MaybeUser::from_request_parts(parts, state).await?;
        user.map(Self)
            .ok_or(ApiError::Application(ApplicationError::NotAuthenticated))
    }
}

/// An optionally authenticated caller, for endpoints that behave differently
/// when signed in but do not require it.
#[derive(Debug, Clone)]
pub struct MaybeUser(pub Option<Authenticated>);

impl FromRequestParts<AppState> for MaybeUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_headers(&parts.headers);
        let Some(cookie) = jar.get(SESSION_COOKIE) else {
            return Ok(Self(None));
        };

        let token = SessionToken::new(cookie.value().to_owned());
        match state.auth.authenticate(&token).await {
            Ok(authenticated) => Ok(Self(Some(authenticated))),
            // An absent, expired, or revoked session is simply "not signed in".
            // Anything else — a storage failure, a deactivated account — must
            // surface rather than be silently downgraded to anonymous access.
            Err(ApplicationError::NotAuthenticated) => Ok(Self(None)),
            Err(error) => Err(ApiError::Application(error)),
        }
    }
}

/// The client's address, as used for rate limiting.
#[derive(Debug, Clone)]
pub struct ClientAddress(pub String);

impl FromRequestParts<AppState> for ClientAddress {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if state.config.trust_forwarded_for
            && let Some(forwarded) = parts
                .headers
                .get("x-forwarded-for")
                .and_then(|value| value.to_str().ok())
            // The leftmost entry is the original client. Later entries are
            // appended by each hop, so only the first is meaningful.
            && let Some(candidate) = forwarded.split(',').next().map(str::trim)
            && !candidate.is_empty()
        {
            return Ok(Self(candidate.to_owned()));
        }

        let address = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map_or_else(
                // Falling back to a shared bucket is the safe direction to fail: it
                // throttles more aggressively rather than letting requests through
                // uncounted.
                || "unknown".to_owned(),
                |ConnectInfo(address)| address.ip().to_string(),
            );
        Ok(Self(address))
    }
}
