use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use elrond_application::LibraryService;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Clone)]
pub struct ApiState {
    pub library: LibraryService,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/v1/overview", get(overview))
        .with_state(state)
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
