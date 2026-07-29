use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header::SET_COOKIE},
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

async fn setup(
    State(state): State<ApiState>,
    Json(request): Json<SetupRequest>,
) -> Result<(HeaderMap, Json<SetupResponse>), (StatusCode, Json<Value>)> {
    let session = state
        .auth
        .create_initial_admin(&request.username, &request.password)
        .await
        .map_err(auth_error_response)?;
    let secure = if state.secure_cookies { "; Secure" } else { "" };
    let cookie = format!(
        "elrond_session={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
        session.token, session.max_age_seconds, secure
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

    Ok((
        headers,
        Json(SetupResponse {
            username: session.username,
        }),
    ))
}

fn auth_error_response(error: AuthError) -> (StatusCode, Json<Value>) {
    let status = match error {
        AuthError::InvalidUsername | AuthError::InvalidPassword => StatusCode::UNPROCESSABLE_ENTITY,
        AuthError::SetupCompleted => StatusCode::CONFLICT,
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
