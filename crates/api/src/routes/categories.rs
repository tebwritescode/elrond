//! Category tree endpoints.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use elrond_application::categories::CategoryNode;
use elrond_domain::{CategoryId, CategoryName, Role};
use serde::Deserialize;

use crate::error::ApiResult;
use crate::extract::CurrentUser;
use crate::state::AppState;

/// Body of `POST /api/v1/categories`.
#[derive(Debug, Deserialize)]
pub struct CreateCategory {
    /// Display name.
    pub name: String,
    /// Parent, or absent for a root category.
    #[serde(default)]
    pub parent_id: Option<CategoryId>,
}

/// A patch field with three distinguishable states.
///
/// A plain `Option` cannot tell "the client did not mention this" from "the client
/// asked to clear it", and for a parent reference those mean very different things:
/// leave the category where it is, versus promote it to a root.
#[derive(Debug, Default)]
pub enum FieldUpdate<T> {
    /// The field was absent from the body. Leave the current value alone.
    #[default]
    Unchanged,
    /// The field was present as `null`. Clear the current value.
    Clear,
    /// The field was present with a value.
    Set(T),
}

impl<'de, T> Deserialize<'de> for FieldUpdate<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde only calls this when the key is present, so `Unchanged` comes from
        // `#[serde(default)]` rather than from here.
        Ok(match Option::deserialize(deserializer)? {
            None => Self::Clear,
            Some(value) => Self::Set(value),
        })
    }
}

/// Body of `PATCH /api/v1/categories/{id}`.
///
/// Both fields are optional and independent: a rename, a move, or both.
#[derive(Debug, Deserialize)]
pub struct UpdateCategory {
    /// New name.
    #[serde(default)]
    pub name: Option<String>,
    /// New parent. An explicit `null` promotes the category to a root.
    #[serde(default)]
    pub parent_id: FieldUpdate<CategoryId>,
}

/// `GET /api/v1/categories`
///
/// The whole tree with per-category and rolled-up document counts. Returned whole
/// because the sidebar renders all of it, and a document library's category count
/// is measured in hundreds.
pub async fn tree(
    State(state): State<AppState>,
    _current: CurrentUser,
) -> ApiResult<Json<Vec<CategoryNode>>> {
    Ok(Json(state.categories.tree().await?))
}

/// `POST /api/v1/categories`
pub async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    body: Result<Json<CreateCategory>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<(StatusCode, Json<CategoryView>)> {
    current.require(Role::Editor)?;
    let Json(body) = body?;

    let name = CategoryName::parse(&body.name)?;
    let created = state.categories.create(body.parent_id, &name).await?;
    Ok((StatusCode::CREATED, Json(CategoryView::from(&created))))
}

/// `PATCH /api/v1/categories/{id}`
pub async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<CategoryId>,
    body: Result<Json<UpdateCategory>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<Json<CategoryView>> {
    current.require(Role::Editor)?;
    let Json(body) = body?;

    // Renamed first, then moved: a move can fail on a name clash at the
    // destination, and reporting that against the new name is less confusing.
    let mut category = state.categories.require(id).await?;
    if let Some(name) = &body.name {
        category = state
            .categories
            .rename(id, &CategoryName::parse(name)?)
            .await?;
    }
    match body.parent_id {
        FieldUpdate::Unchanged => {}
        FieldUpdate::Clear => category = state.categories.move_to(id, None).await?,
        FieldUpdate::Set(parent_id) => {
            category = state.categories.move_to(id, Some(parent_id)).await?;
        }
    }

    Ok(Json(CategoryView::from(&category)))
}

/// `DELETE /api/v1/categories/{id}`
///
/// Refuses while the category still holds documents or children, rather than
/// deleting them along with it.
pub async fn delete(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<CategoryId>,
) -> ApiResult<StatusCode> {
    current.require(Role::Editor)?;
    state.categories.delete(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A category as the client sees it.
#[derive(Debug, serde::Serialize)]
pub struct CategoryView {
    /// Identifier.
    pub id: String,
    /// Parent, or null for a root.
    pub parent_id: Option<String>,
    /// Display name.
    pub name: String,
    /// Ordering among siblings.
    pub position: i64,
}

impl From<&elrond_domain::Category> for CategoryView {
    fn from(category: &elrond_domain::Category) -> Self {
        Self {
            id: category.id.to_string(),
            parent_id: category.parent_id.map(|id| id.to_string()),
            name: category.name.to_string(),
            position: category.position,
        }
    }
}
