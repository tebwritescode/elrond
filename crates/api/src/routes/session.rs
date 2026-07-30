//! First-run setup, sign-in, sign-out, and session inspection.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::extract::cookie::CookieJar;
use elrond_application::auth::{EstablishedSession, FirstRunSetupInput, SignInInput};
use elrond_domain::{Role, User};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;

use crate::cookies::{CSRF_COOKIE, SESSION_COOKIE, csrf_cookie, expired_cookie, session_cookie};
use crate::error::{ApiError, ApiResult};
use crate::extract::{ClientAddress, CurrentUser, MaybeUser};
use crate::rate_limit::Decision;
use crate::state::AppState;

/// An account as the client sees it.
///
/// A separate type from [`User`] on purpose: the wire contract should not shift
/// just because a field is added to the domain entity, and there is no field here
/// that could ever carry credential material.
#[derive(Debug, Clone, Serialize)]
pub struct UserView {
    /// Identifier as a string, since JSON has no UUID type.
    pub id: String,
    /// Login name, also what the interface displays.
    pub username: String,
    /// Authority level.
    pub role: Role,
    /// Whether the account may sign in.
    pub is_active: bool,
    /// Creation time, RFC 3339 in UTC.
    pub created_at: String,
}

impl From<&User> for UserView {
    fn from(user: &User) -> Self {
        Self {
            id: user.id.to_string(),
            username: user.username.to_string(),
            role: user.role,
            is_active: user.is_active,
            created_at: user
                .created_at
                .format(&Rfc3339)
                .unwrap_or_else(|_| String::new()),
        }
    }
}

/// Everything the client needs on first load.
///
/// Delivered as one request so the application shell can decide between the
/// setup screen, the sign-in screen, and the workspace without a render flash or
/// a waterfall of round trips.
#[derive(Debug, Serialize)]
pub struct Bootstrap {
    /// Whether the instance still needs its first administrator.
    pub requires_setup: bool,
    /// The signed-in account, when there is one.
    pub user: Option<UserView>,
    /// Token to echo in the CSRF header on state-changing requests.
    pub csrf_token: String,
    /// Build version.
    pub version: &'static str,
}

/// A newly established session.
#[derive(Debug, Serialize)]
pub struct SessionCreated {
    /// The signed-in account.
    pub user: UserView,
    /// Rotated CSRF token for subsequent requests.
    pub csrf_token: String,
    /// Hard expiry of the session, RFC 3339 in UTC.
    pub expires_at: String,
}

/// Body of `POST /api/v1/setup`.
#[derive(Debug, Deserialize)]
pub struct SetupRequest {
    /// Login name for the first administrator.
    pub username: String,
    /// Password, validated against the domain policy then hashed.
    pub password: String,
}

/// Body of `POST /api/v1/session`.
#[derive(Debug, Deserialize)]
pub struct SignInRequest {
    /// Login name.
    pub username: String,
    /// Password.
    pub password: String,
}

/// `GET /api/v1/bootstrap`
///
/// Always issues a CSRF token, because the client needs one before it can make
/// its first state-changing request, including the very first sign-in.
pub async fn bootstrap(
    State(state): State<AppState>,
    jar: CookieJar,
    MaybeUser(user): MaybeUser,
) -> ApiResult<(CookieJar, Json<Bootstrap>)> {
    let requires_setup = state.auth.setup_state().await?.requires_setup();

    // Reuse an existing token so a second tab does not invalidate the first
    // tab's in-flight forms.
    let (token, jar) = match jar.get(CSRF_COOKIE) {
        Some(cookie) if !cookie.value().is_empty() => (cookie.value().to_owned(), jar),
        _ => {
            let token = state.tokens.generate().expose().to_owned();
            let jar = jar.add(csrf_cookie(
                token.clone(),
                state.session_policy.absolute_lifetime,
                &state.config,
            ));
            (token, jar)
        }
    };

    Ok((
        jar,
        Json(Bootstrap {
            requires_setup,
            user: user.as_ref().map(|auth| UserView::from(&auth.user)),
            csrf_token: token,
            version: state.version,
        }),
    ))
}

/// `POST /api/v1/setup`
///
/// Creates the first administrator and signs it in. Rate limited despite being
/// single-use, because it is reachable before any credential exists.
pub async fn complete_setup(
    State(state): State<AppState>,
    ClientAddress(client): ClientAddress,
    jar: CookieJar,
    body: Result<Json<SetupRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<(StatusCode, CookieJar, Json<SessionCreated>)> {
    enforce_limit(&state, &client, "setup")?;
    let Json(body) = body?;

    let established = state
        .auth
        .complete_first_run_setup(FirstRunSetupInput {
            username: body.username,
            password: body.password,
        })
        .await?;

    let (jar, response) = issue_session(&state, jar, &established);
    Ok((StatusCode::CREATED, jar, Json(response)))
}

/// `POST /api/v1/session`
pub async fn sign_in(
    State(state): State<AppState>,
    ClientAddress(client): ClientAddress,
    jar: CookieJar,
    body: Result<Json<SignInRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<(StatusCode, CookieJar, Json<SessionCreated>)> {
    enforce_limit(&state, &client, "sign_in")?;
    let Json(body) = body?;

    let established = state
        .auth
        .sign_in(SignInInput {
            username: body.username,
            password: body.password,
        })
        .await?;

    // Successful authentication clears the counter so someone who fumbled their
    // password a few times is not throttled afterwards.
    state.limiter.reset(&client, "sign_in");

    let (jar, response) = issue_session(&state, jar, &established);
    Ok((StatusCode::CREATED, jar, Json(response)))
}

/// `DELETE /api/v1/session`
///
/// Idempotent: signing out without a session is a success, so a stale tab does
/// not show an error the user can do nothing about.
pub async fn sign_out(
    State(state): State<AppState>,
    jar: CookieJar,
    MaybeUser(user): MaybeUser,
) -> ApiResult<(StatusCode, CookieJar)> {
    if let Some(authenticated) = user {
        state.auth.sign_out(authenticated.session_id).await?;
    }

    let jar = jar
        .add(expired_cookie(SESSION_COOKIE, &state.config))
        .add(expired_cookie(CSRF_COOKIE, &state.config));
    Ok((StatusCode::NO_CONTENT, jar))
}

/// `GET /api/v1/me`
pub async fn me(CurrentUser(authenticated): CurrentUser) -> Json<UserView> {
    Json(UserView::from(&authenticated.user))
}

/// Applies the credential rate limit for a scope.
fn enforce_limit(state: &AppState, client: &str, scope: &'static str) -> Result<(), ApiError> {
    match state.limiter.check(
        client,
        scope,
        state.config.auth_attempt_limit,
        state.config.auth_attempt_window,
    ) {
        Decision::Allowed => Ok(()),
        Decision::Limited {
            retry_after_seconds,
        } => {
            tracing::warn!(scope, "rate limited a credential attempt");
            Err(ApiError::RateLimited {
                retry_after_seconds,
            })
        }
    }
}

/// Sets the session and CSRF cookies for a newly established session.
fn issue_session(
    state: &AppState,
    jar: CookieJar,
    established: &EstablishedSession,
) -> (CookieJar, SessionCreated) {
    // The CSRF token is regenerated alongside the session. Carrying the
    // pre-authentication token forward would leave a value an attacker may
    // already have planted still valid after the privilege level changed.
    let csrf_token = state.tokens.generate().expose().to_owned();
    let max_age = state.session_policy.absolute_lifetime;

    let jar = jar
        .add(session_cookie(
            established.token.expose().to_owned(),
            max_age,
            &state.config,
        ))
        .add(csrf_cookie(csrf_token.clone(), max_age, &state.config));

    let response = SessionCreated {
        user: UserView::from(&established.user),
        csrf_token,
        expires_at: established
            .expires_at
            .format(&Rfc3339)
            .unwrap_or_else(|_| String::new()),
    };
    (jar, response)
}
