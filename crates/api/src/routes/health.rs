//! Liveness endpoint.

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::state::AppState;

/// Health payload.
#[derive(Debug, Serialize)]
pub struct Health {
    /// Always `"ok"` when the process is serving.
    pub status: &'static str,
    /// Build version, so a deployment can be identified without shell access.
    pub version: &'static str,
}

/// `GET /api/v1/health`
///
/// Unauthenticated on purpose: a container orchestrator has no session. It
/// reports only the status and version, never configuration or storage detail.
pub async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok",
        version: state.version,
    })
}
