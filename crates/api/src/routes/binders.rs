//! Binder generation endpoint.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use elrond_application::binders::BuildBinderRequest;
use elrond_application::ports::{BinderSettings, PageNumbering, PageSize};
use elrond_domain::{CategoryId, LifecycleState};
use serde::Deserialize;

use crate::error::{ApiError, ApiResult};
use crate::extract::CurrentUser;
use crate::state::AppState;

/// Body of `POST /api/v1/binders/build`.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent output toggles a user ticks individually, not a state \
              machine; an enum would misrepresent them as mutually exclusive"
)]
#[derive(Debug, Deserialize)]
pub struct BuildRequest {
    /// Cover title.
    pub title: String,
    /// Optional cover subtitle.
    #[serde(default)]
    pub subtitle: Option<String>,
    /// Optional organization shown on the cover.
    #[serde(default)]
    pub organization: Option<String>,
    /// Categories to include. Empty or absent means the whole library.
    #[serde(default)]
    pub category_ids: Vec<CategoryId>,
    /// Lifecycle states to include. Empty means everything the caller may see.
    #[serde(default)]
    pub lifecycles: Vec<LifecycleState>,
    /// Paper size, `a4` or `letter`.
    #[serde(default = "default_page_size")]
    pub page_size: PageSize,
    /// Whether to emit a front cover.
    #[serde(default = "yes")]
    pub include_cover: bool,
    /// Whether to emit a table of contents.
    #[serde(default = "yes")]
    pub include_toc: bool,
    /// Whether to emit a full-page separator before each category.
    #[serde(default = "yes")]
    pub include_separators: bool,
    /// Whether to emit a full-page separator before each document as well.
    #[serde(default = "yes")]
    pub document_separators: bool,
    /// Whether to number pages.
    #[serde(default = "yes")]
    pub page_numbers: bool,
    /// Whether to pad so each separator falls on a right-hand page.
    #[serde(default)]
    pub duplex_blank_pages: bool,
}

/// Default paper size.
const fn default_page_size() -> PageSize {
    PageSize::A4
}

/// Serde default for the opt-out booleans.
const fn yes() -> bool {
    true
}

/// `POST /api/v1/binders/build`
///
/// Generates a complete binder and returns it as a PDF.
///
/// The PDF is returned directly rather than stored and linked, because at this
/// milestone a binder is a report generated on demand from the category tree, not
/// a persisted artefact with its own release history. Nothing is written, so
/// generating one twice cannot leave anything behind.
pub async fn build(
    State(state): State<AppState>,
    current: CurrentUser,
    body: Result<Json<BuildRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<Response> {
    let Json(body) = body?;

    if body.title.trim().is_empty() {
        return Err(ApiError::Application(
            elrond_application::ApplicationError::Domain(elrond_domain::DomainError::Required {
                field: "title",
            }),
        ));
    }

    let generated = state
        .binders
        .build(
            &current.0,
            BuildBinderRequest {
                title: body.title.trim().to_owned(),
                subtitle: body.subtitle.filter(|value| !value.trim().is_empty()),
                organization: body.organization.filter(|value| !value.trim().is_empty()),
                category_ids: body.category_ids,
                lifecycles: body.lifecycles,
                settings: BinderSettings {
                    page_size: body.page_size,
                    include_cover: body.include_cover,
                    include_toc: body.include_toc,
                    include_separators: body.include_separators,
                    document_separators: body.document_separators,
                    page_numbering: if body.page_numbers {
                        PageNumbering::Continuous
                    } else {
                        PageNumbering::None
                    },
                    duplex_blank_pages: body.duplex_blank_pages,
                },
            },
        )
        .await?;

    let filename = safe_filename(&body.title);
    let mut response = (StatusCode::OK, generated.pdf).into_response();
    let headers = response.headers_mut();

    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/pdf"),
    );
    if let Ok(value) =
        header::HeaderValue::from_str(&format!("attachment; filename=\"{filename}.pdf\""))
    {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    // A generated report must never be cached: the library changes underneath it.
    if let Ok(value) = header::HeaderValue::from_str("no-store") {
        headers.insert(header::CACHE_CONTROL, value);
    }

    // Counts travel as headers so the browser can download the PDF directly from a
    // form submission while the client still learns what happened.
    if let Ok(value) = header::HeaderValue::from_str(&generated.page_count.to_string()) {
        headers.insert("x-elrond-page-count", value);
    }
    if let Ok(value) = header::HeaderValue::from_str(&generated.document_count.to_string()) {
        headers.insert("x-elrond-document-count", value);
    }
    if let Ok(value) = header::HeaderValue::from_str(&generated.skipped.len().to_string()) {
        headers.insert("x-elrond-skipped-count", value);
    }

    Ok(response)
}

/// Reduces a title to something safe for a `Content-Disposition` filename.
fn safe_filename(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();

    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "binder".to_owned()
    } else {
        trimmed.chars().take(80).collect()
    }
}
