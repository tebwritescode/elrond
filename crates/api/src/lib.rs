use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{COOKIE, SET_COOKIE},
    },
    routing::{get, post},
};
use elrond_application::{AuthError, AuthService, ImportError, ImportService, LibraryService};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct ApiState {
    pub library: LibraryService,
    pub auth: AuthService,
    pub imports: ImportService,
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
        .route(
            "/api/v1/imports/zip",
            post(import_zip).layer(DefaultBodyLimit::max(256 * 1024 * 1024)),
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

async fn import_zip(
    State(state): State<ApiState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<impl Serialize>, (StatusCode, Json<Value>)> {
    let token = session_token(&headers).ok_or_else(unauthorized_response)?;
    let user = state
        .auth
        .current_user(token)
        .await
        .map_err(auth_error_response)?
        .ok_or_else(unauthorized_response)?;
    let mut archive_bytes = None;
    let mut root_category = "Imported".to_owned();

    while let Some(field) = multipart.next_field().await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "The upload form could not be read." })),
        )
    })? {
        match field.name() {
            Some("archive") => {
                let filename_is_zip = field
                    .file_name()
                    .map(|name| name.to_ascii_lowercase().ends_with(".zip"))
                    .unwrap_or(false);
                if !filename_is_zip {
                    return Err((
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(json!({ "error": "Choose a ZIP archive to import." })),
                    ));
                }
                archive_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|_| {
                            (
                                StatusCode::BAD_REQUEST,
                                Json(json!({ "error": "The ZIP archive could not be read." })),
                            )
                        })?
                        .to_vec(),
                );
            }
            Some("rootCategory") => {
                root_category = field.text().await.map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "The root category could not be read." })),
                    )
                })?;
            }
            _ => {}
        }
    }

    let archive_bytes = archive_bytes.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "A ZIP archive is required." })),
        )
    })?;
    state
        .imports
        .import_zip(archive_bytes, &root_category, &user.id)
        .await
        .map(Json)
        .map_err(import_error_response)
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

fn unauthorized_response() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "Sign in to continue." })),
    )
}

fn import_error_response(error: ImportError) -> (StatusCode, Json<Value>) {
    let status = match error {
        ImportError::TooManyEntries
        | ImportError::ExpandedSizeLimit
        | ImportError::FileSizeLimit => StatusCode::PAYLOAD_TOO_LARGE,
        ImportError::InvalidArchive
        | ImportError::UnsafeEntry
        | ImportError::InvalidFileType
        | ImportError::DepthLimit
        | ImportError::Extraction => StatusCode::UNPROCESSABLE_ENTITY,
        ImportError::Repository(_) => {
            tracing::error!(error = %error, "ZIP import failed");
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
