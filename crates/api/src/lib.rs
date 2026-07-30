use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{COOKIE, SET_COOKIE},
    },
    routing::{get, post},
};
use elrond_application::{
    AuthError, AuthService, BinderError, BinderService, CatalogError, CatalogService, ImportError,
    ImportService, LibraryService,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct ApiState {
    pub library: LibraryService,
    pub auth: AuthService,
    pub imports: ImportService,
    pub catalog: CatalogService,
    pub binders: BinderService,
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
        .route(
            "/api/v1/documents",
            get(documents)
                .post(upload_document)
                .layer(DefaultBodyLimit::max(256 * 1024 * 1024)),
        )
        .route("/api/v1/categories", get(categories))
        .route("/api/v1/binders/printable.pdf", get(printable_binder))
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

async fn documents(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<impl Serialize>, (StatusCode, Json<Value>)> {
    authenticated_user(&state, &headers).await?;
    state
        .catalog
        .documents()
        .await
        .map(Json)
        .map_err(catalog_error_response)
}

async fn upload_document(
    State(state): State<ApiState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let user = authenticated_user(&state, &headers).await?;
    let mut files = Vec::new();
    let mut category_path = Vec::new();
    while let Some(field) = multipart.next_field().await.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "The upload form could not be read." })),
        )
    })? {
        match field.name() {
            Some("file") => {
                let filename = field.file_name().map(str::to_owned).ok_or_else(|| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "The document filename is missing." })),
                    )
                })?;
                let content = field.bytes().await.map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "The document could not be read." })),
                    )
                })?;
                files.push((filename, content.to_vec()));
            }
            Some("categoryPath") => {
                let value = field.text().await.map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "The category path could not be read." })),
                    )
                })?;
                category_path = serde_json::from_str(&value).map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "The category path is invalid." })),
                    )
                })?;
            }
            _ => {}
        }
    }
    if files.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Choose one or more documents to upload." })),
        ));
    }
    let mut imported = 0;
    let mut categories_created = 0;
    let mut duplicates = 0;
    let mut unsupported = 0;
    for (filename, content) in files {
        let summary = state
            .imports
            .import_file(content, &filename, category_path.clone(), &user.id)
            .await
            .map_err(import_error_response)?;
        imported += summary.documents_imported;
        categories_created += summary.categories_created;
        duplicates += summary.duplicates_skipped;
        unsupported += summary.unsupported_skipped;
    }
    Ok(Json(json!({
        "categoriesCreated": categories_created,
        "documentsImported": imported,
        "duplicatesSkipped": duplicates,
        "unsupportedSkipped": unsupported,
    })))
}

async fn categories(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<Json<impl Serialize>, (StatusCode, Json<Value>)> {
    authenticated_user(&state, &headers).await?;
    state
        .catalog
        .categories()
        .await
        .map(Json)
        .map_err(catalog_error_response)
}

async fn printable_binder(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Result<(HeaderMap, Vec<u8>), (StatusCode, Json<Value>)> {
    authenticated_user(&state, &headers).await?;
    let binder = state
        .binders
        .generate_printable()
        .await
        .map_err(binder_error_response)?;
    let mut response_headers = HeaderMap::new();
    response_headers.insert("content-type", HeaderValue::from_static("application/pdf"));
    response_headers.insert(
        "content-disposition",
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", binder.filename))
            .map_err(|_| binder_error_response(BinderError::Render("invalid filename".into())))?,
    );
    response_headers.insert(
        "content-length",
        HeaderValue::from_str(&binder.content.len().to_string())
            .map_err(|_| binder_error_response(BinderError::Render("invalid length".into())))?,
    );
    Ok((response_headers, binder.content))
}

async fn authenticated_user(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<elrond_domain::auth::AuthenticatedUser, (StatusCode, Json<Value>)> {
    let token = session_token(headers).ok_or_else(unauthorized_response)?;
    state
        .auth
        .current_user(token)
        .await
        .map_err(auth_error_response)?
        .ok_or_else(unauthorized_response)
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

fn binder_error_response(error: BinderError) -> (StatusCode, Json<Value>) {
    let status = match error {
        BinderError::Empty => StatusCode::UNPROCESSABLE_ENTITY,
        BinderError::InvalidSource | BinderError::Render(_) => {
            tracing::error!(%error, "binder generation failed");
            StatusCode::UNPROCESSABLE_ENTITY
        }
        BinderError::Repository(_) => {
            tracing::error!(%error, "binder source loading failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    (status, Json(json!({ "error": error.to_string() })))
}

fn catalog_error_response(error: CatalogError) -> (StatusCode, Json<Value>) {
    tracing::error!(error = %error, "catalog request failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": error.to_string() })),
    )
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
