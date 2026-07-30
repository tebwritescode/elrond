//! Account listing.

use axum::Json;
use axum::extract::State;
use elrond_domain::Role;

use crate::error::ApiResult;
use crate::extract::CurrentUser;
use crate::routes::session::UserView;
use crate::state::AppState;

/// `GET /api/v1/users`
///
/// Administrators only. Listed here rather than under an admin module because
/// account management is the only administrative surface in this milestone.
pub async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Vec<UserView>>> {
    current.require(Role::Admin)?;

    // Reaches through to the repository via the use-case layer's port. Listing
    // has no business rules beyond authorization, so it needs no use case of its
    // own yet.
    let users = state.auth.list_users().await?;
    Ok(Json(users.iter().map(UserView::from).collect()))
}
