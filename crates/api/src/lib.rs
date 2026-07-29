use axum::{
    Json, Router,
    extract::State,
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{COOKIE, SET_COOKIE},
    },
    routing::{get, post},
};
use elrond_application::{AuthError, AuthService, LibraryService};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct ApiState {
    pub library: LibraryService,
    pub auth: AuthService,
    pub secure_cookies: bool,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/v1/overview", get(overview))
        .route("/api/v1/setup", post(setup))
        .route(
            "/api/v1/session",
            get(current_session).post(login).delete(logout),
        )
        .with_state(state)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetupRequest {
    username: String,
    password: String,
}

#[derive(Serialize)]
struct SetupResponse {
    username: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn setup(
    State(state): State<ApiState>,
    Json(request): Json<SetupRequest>,
) -> Result<(HeaderMap, Json<SetupResponse>), (StatusCode, Json<Value>)> {
    let session = state
        .auth
        .create_initial_admin(&request.username, &request.password)
        .await
        .map_err(auth_error_response)?;
    let headers = session_cookie_headers(
        &session.token,
        session.max_age_seconds,
        state.secure_cookies,
    )?;

    Ok((
        headers,
        Json(SetupResponse {
            username: session.username,
        }),
    ))
}

async fn login(
    State(state): State<ApiState>,
    Json(request): Json<LoginRequest>,
) -> Result<(HeaderMap, Json<SetupResponse>), (StatusCode, Json<Value>)> {
    let session = state
        .auth
        .login(&request.username, &request.password)
        .await
        .map_err(auth_error_response)?;
    let headers = session_cookie_headers(
        &session.token,
        session.max_age_seconds,
        state.secure_cookies,
    )?;
    Ok((
        headers,
        Json(SetupResponse {
            username: session.username,
        }),
    ))
}

async fn current_session(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<impl Serialize>, StatusCode> {
    let token = session_token(&headers).ok_or(StatusCode::UNAUTHORIZED)?;
    state
        .auth
        .current_user(token)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "session lookup failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map(Json)
        .ok_or(StatusCode::UNAUTHORIZED)
}

async fn logout(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<HeaderMap, (StatusCode, Json<Value>)> {
    if let Some(token) = session_token(&headers) {
        state
            .auth
            .logout(token)
            .await
            .map_err(auth_error_response)?;
    }
    session_cookie_headers("", 0, state.secure_cookies)
}

fn session_cookie_headers(
    token: &str,
    max_age: i64,
    secure: bool,
) -> Result<HeaderMap, (StatusCode, Json<Value>)> {
    let secure_attribute = if secure { "; Secure" } else { "" };
    let cookie = format!(
        "elrond_session={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure_attribute}"
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "The session could not be created." })),
            )
        })?,
    );
    Ok(headers)
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|cookie| cookie.trim().split_once('='))
        .find_map(|(name, value)| (name == "elrond_session").then_some(value))
        .filter(|value| !value.is_empty())
}

fn auth_error_response(error: AuthError) -> (StatusCode, Json<Value>) {
    let status = match error {
        AuthError::InvalidUsername | AuthError::InvalidPassword => StatusCode::UNPROCESSABLE_ENTITY,
        AuthError::SetupCompleted => StatusCode::CONFLICT,
        AuthError::InvalidCredentials => StatusCode::UNAUTHORIZED,
        AuthError::PasswordHash | AuthError::SessionGeneration | AuthError::Repository(_) => {
            tracing::error!(error = %error, "administrator setup failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    (status, Json(json!({ "error": error.to_string() })))
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "elrond" }))
}

async fn overview(
    State(state): State<ApiState>,
) -> Result<Json<impl Serialize>, (StatusCode, Json<Value>)> {
    state.library.overview().await.map(Json).map_err(|error| {
        tracing::error!(error = %error, "failed to load library overview");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "The library overview is temporarily unavailable." })),
        )
    })
}
