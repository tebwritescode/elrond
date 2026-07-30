//! Document endpoints: upload, listing, detail, and download.

use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use elrond_application::documents::{DocumentContent, UploadRequest};
use elrond_application::ports::{DocumentFilter, DocumentSort, SortOrder, StoredDocument};
use elrond_domain::{
    CategoryId, DocumentId, DocumentVersion, DocumentVersionId, LifecycleState, Role, TagId,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::{ApiError, ApiResult};
use crate::extract::CurrentUser;
use crate::state::AppState;

/// Largest page a client may request.
///
/// Capped so a crafted `limit` cannot ask the server to materialize the whole
/// library in one response.
const MAX_PAGE_SIZE: u32 = 200;

// -------------------------------------------------------------------- views

/// A tag as the client sees it.
#[derive(Debug, Serialize)]
pub struct TagView {
    /// Identifier.
    pub id: String,
    /// Label as typed.
    pub label: String,
}

/// A version as the client sees it.
///
/// Storage keys are deliberately absent: the internal layout is not a client
/// concern, and exposing it would invite requests built from it.
#[derive(Debug, Serialize)]
pub struct VersionView {
    /// Identifier. This is what a binder release pins.
    pub id: String,
    /// Sequence number within the document.
    pub number: u32,
    /// Filename as uploaded.
    pub filename: String,
    /// Content type of the original.
    pub media_type: String,
    /// Size of the original in bytes.
    pub byte_size: u64,
    /// Checksum of the original, lowercase hex.
    pub checksum: String,
    /// Whether a PDF is available to view: the original, or a generated copy.
    pub has_pdf: bool,
    /// Whether a PDF copy still has to be generated.
    pub awaiting_conversion: bool,
    /// Note describing what changed.
    pub note: Option<String>,
    /// Creation time, RFC 3339 in UTC.
    pub created_at: String,
}

impl From<&DocumentVersion> for VersionView {
    fn from(version: &DocumentVersion) -> Self {
        Self {
            id: version.id.to_string(),
            number: version.number.get(),
            filename: version.original_filename.to_string(),
            media_type: version.media_type.mime().to_owned(),
            byte_size: version.byte_size,
            checksum: version.checksum.to_hex(),
            has_pdf: version.is_renderable(),
            awaiting_conversion: version.awaits_derivative(),
            note: version.note.clone(),
            created_at: format_time(version.created_at),
        }
    }
}

/// A document as the client sees it.
#[derive(Debug, Serialize)]
pub struct DocumentView {
    /// Identifier.
    pub id: String,
    /// Title.
    pub title: String,
    /// Primary category identifier.
    pub category_id: String,
    /// Primary category name, so a listing needs no second request.
    pub category_name: String,
    /// Lifecycle state.
    pub lifecycle: LifecycleState,
    /// How many versions exist.
    pub version_count: u32,
    /// The current version.
    pub current_version: VersionView,
    /// Tags.
    pub tags: Vec<TagView>,
    /// Folder-relative provenance from a bulk import.
    pub source_path: Option<String>,
    /// When review falls due.
    pub review_due_at: Option<String>,
    /// Creation time.
    pub created_at: String,
    /// Last modification time.
    pub updated_at: String,
}

impl From<&StoredDocument> for DocumentView {
    fn from(stored: &StoredDocument) -> Self {
        Self {
            id: stored.document.id.to_string(),
            title: stored.document.title.to_string(),
            category_id: stored.document.category_id.to_string(),
            category_name: stored.category_name.to_string(),
            lifecycle: stored.document.lifecycle,
            version_count: stored.document.version_count,
            current_version: VersionView::from(&stored.current_version),
            tags: stored
                .tags
                .iter()
                .map(|tag| TagView {
                    id: tag.id.to_string(),
                    label: tag.label.as_str().to_owned(),
                })
                .collect(),
            source_path: stored.document.source_path.clone(),
            review_due_at: stored.document.review_due_at.map(format_time),
            created_at: format_time(stored.document.created_at),
            updated_at: format_time(stored.document.updated_at),
        }
    }
}

/// One page of a listing.
#[derive(Debug, Serialize)]
pub struct DocumentPageView {
    /// The rows on this page.
    pub documents: Vec<DocumentView>,
    /// How many rows match in total, so a real pager can be rendered.
    pub total: u64,
    /// Echo of the page size actually applied, which may be capped.
    pub limit: u32,
    /// Echo of the offset applied.
    pub offset: u32,
}

/// A document with its version history.
#[derive(Debug, Serialize)]
pub struct DocumentDetailView {
    /// The document.
    #[serde(flatten)]
    pub document: DocumentView,
    /// Every version, newest first.
    pub versions: Vec<VersionView>,
}

// ------------------------------------------------------------------ requests

/// Query string for `GET /api/v1/documents`.
#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    /// Full-text query.
    #[serde(default)]
    pub q: Option<String>,
    /// Restrict to one category.
    #[serde(default)]
    pub category_id: Option<CategoryId>,
    /// Whether to include nested categories. Defaults to true.
    #[serde(default)]
    pub include_descendants: Option<bool>,
    /// Comma-separated lifecycle states.
    #[serde(default)]
    pub lifecycle: Option<String>,
    /// Comma-separated tag identifiers. A document must carry all of them.
    #[serde(default)]
    pub tags: Option<String>,
    /// Column to sort by.
    #[serde(default)]
    pub sort: Option<String>,
    /// Sort direction.
    #[serde(default)]
    pub order: Option<String>,
    /// Page size.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Rows to skip.
    #[serde(default)]
    pub offset: Option<u32>,
}

impl ListQuery {
    /// Translates the query string into a repository filter.
    fn to_filter(&self) -> ApiResult<DocumentFilter> {
        let mut filter = DocumentFilter {
            category_id: self.category_id,
            include_descendants: self.include_descendants.unwrap_or(true),
            limit: self.limit.unwrap_or(50).clamp(1, MAX_PAGE_SIZE),
            offset: self.offset.unwrap_or(0),
            ..Default::default()
        };

        if let Some(raw) = &self.lifecycle {
            filter.lifecycles = raw
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::parse::<LifecycleState>)
                .collect::<Result<Vec<_>, _>>()?;
        }

        if let Some(raw) = &self.tags {
            filter.tag_ids = raw
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    value
                        .parse::<TagId>()
                        .map_err(|_| ApiError::MalformedRequest {
                            code: "request_body_invalid",
                            message: "tags must be a comma-separated list of identifiers"
                                .to_owned(),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
        }

        filter.sort = match self.sort.as_deref() {
            None => {
                // With a query, relevance is the only ordering that makes sense;
                // without one, recency is.
                if self.q.is_some() {
                    DocumentSort::Relevance
                } else {
                    DocumentSort::Updated
                }
            }
            Some("title") => DocumentSort::Title,
            Some("created") => DocumentSort::Created,
            Some("updated") => DocumentSort::Updated,
            Some("size") => DocumentSort::Size,
            Some("relevance") => DocumentSort::Relevance,
            Some(other) => {
                return Err(ApiError::MalformedRequest {
                    code: "request_body_invalid",
                    message: format!("unknown sort column {other:?}"),
                });
            }
        };

        filter.order = match self.order.as_deref() {
            None => match filter.sort {
                // Alphabetical defaults to A–Z; everything else to newest or
                // largest first, which is what people expect from those columns.
                DocumentSort::Title => SortOrder::Ascending,
                _ => SortOrder::Descending,
            },
            Some("asc") => SortOrder::Ascending,
            Some("desc") => SortOrder::Descending,
            Some(other) => {
                return Err(ApiError::MalformedRequest {
                    code: "request_body_invalid",
                    message: format!("unknown sort order {other:?}"),
                });
            }
        };

        Ok(filter)
    }
}

/// Body of `PATCH /api/v1/documents/{id}`.
#[derive(Debug, Deserialize)]
pub struct UpdateDocument {
    /// New title.
    pub title: String,
    /// New primary category.
    pub category_id: CategoryId,
    /// New review date, or null to clear it.
    #[serde(default)]
    pub review_due_at: Option<String>,
    /// Complete replacement tag set.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Body of `POST /api/v1/documents/{id}/lifecycle`.
#[derive(Debug, Deserialize)]
pub struct TransitionRequest {
    /// The state to move to.
    pub lifecycle: LifecycleState,
}

// ------------------------------------------------------------------ handlers

/// `POST /api/v1/documents`
///
/// Multipart upload. The `file` part is required; `category_id`, `title`, and
/// `tags` are optional text parts.
pub async fn upload(
    State(state): State<AppState>,
    current: CurrentUser,
    multipart: Multipart,
) -> ApiResult<(StatusCode, Json<UploadResultView>)> {
    current.require(Role::Editor)?;

    let request = read_upload(multipart).await?;
    let outcome = state.documents.upload(&current.0, request).await?;

    Ok((
        StatusCode::CREATED,
        Json(UploadResultView {
            document: DocumentView::from(&outcome.document),
            deduplicated: outcome.deduplicated,
            duplicate_of: outcome.duplicate_of.map(|id| id.to_string()),
        }),
    ))
}

/// The result of an upload.
#[derive(Debug, Serialize)]
pub struct UploadResultView {
    /// The stored document.
    pub document: DocumentView,
    /// Whether the bytes were already present and were reused.
    pub deduplicated: bool,
    /// An existing document with identical content, if any.
    pub duplicate_of: Option<String>,
}

/// `GET /api/v1/documents`
pub async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(query): Query<ListQuery>,
) -> ApiResult<Json<DocumentPageView>> {
    let filter = query.to_filter()?;
    let limit = filter.limit;
    let offset = filter.offset;

    let page = state
        .documents
        .list(&current.0, filter, query.q.as_deref())
        .await?;

    Ok(Json(DocumentPageView {
        documents: page.documents.iter().map(DocumentView::from).collect(),
        total: page.total,
        limit,
        offset,
    }))
}

/// `GET /api/v1/documents/{id}`
pub async fn detail(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<DocumentId>,
) -> ApiResult<Json<DocumentDetailView>> {
    let detail = state.documents.detail(&current.0, id).await?;
    Ok(Json(DocumentDetailView {
        document: DocumentView::from(&detail.document),
        versions: detail.versions.iter().map(VersionView::from).collect(),
    }))
}

/// `PATCH /api/v1/documents/{id}`
pub async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<DocumentId>,
    body: Result<Json<UpdateDocument>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<Json<DocumentView>> {
    let Json(body) = body?;

    let review_due_at =
        match body.review_due_at.as_deref() {
            None | Some("") => None,
            Some(raw) => Some(OffsetDateTime::parse(raw, &Rfc3339).map_err(|_| {
                ApiError::MalformedRequest {
                    code: "request_body_invalid",
                    message: "review_due_at must be an RFC 3339 timestamp".to_owned(),
                }
            })?),
        };

    let updated = state
        .documents
        .update_metadata(
            &current.0,
            id,
            &body.title,
            body.category_id,
            review_due_at,
            &body.tags,
        )
        .await?;
    Ok(Json(DocumentView::from(&updated)))
}

/// `POST /api/v1/documents/{id}/lifecycle`
pub async fn transition(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<DocumentId>,
    body: Result<Json<TransitionRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<Json<DocumentView>> {
    let Json(body) = body?;
    let updated = state
        .documents
        .transition(&current.0, id, body.lifecycle)
        .await?;
    Ok(Json(DocumentView::from(&updated)))
}

/// `POST /api/v1/documents/{id}/versions`
///
/// Appends a version. The previous version is retained, because a binder release
/// may pin it.
pub async fn add_version(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<DocumentId>,
    multipart: Multipart,
) -> ApiResult<(StatusCode, Json<VersionView>)> {
    current.require(Role::Editor)?;

    let request = read_upload(multipart).await?;
    let note = request.title.clone();
    let version = state
        .documents
        .add_version(&current.0, id, request, note)
        .await?;
    Ok((StatusCode::CREATED, Json(VersionView::from(&version))))
}

/// `GET /api/v1/versions/{id}/original`
///
/// Streams the immutable original as an attachment.
pub async fn download_original(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<DocumentVersionId>,
) -> ApiResult<Response> {
    let content = state.documents.original(&current.0, id).await?;
    Ok(attachment(content, true))
}

/// `GET /api/v1/versions/{id}/pdf`
///
/// Returns the PDF to render: the original when it already is one, otherwise the
/// generated copy. Served inline so the browser viewer can display it.
pub async fn download_pdf(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<DocumentVersionId>,
) -> ApiResult<Response> {
    let content = state.documents.renderable_pdf(&current.0, id).await?;
    Ok(attachment(content, false))
}

/// `GET /api/v1/tags`
pub async fn list_tags(
    State(state): State<AppState>,
    _current: CurrentUser,
) -> ApiResult<Json<Vec<TagCountView>>> {
    let tags = state.tags.list_with_counts().await?;
    Ok(Json(
        tags.iter()
            .map(|(tag, count)| TagCountView {
                id: tag.id.to_string(),
                label: tag.label.as_str().to_owned(),
                document_count: *count,
            })
            .collect(),
    ))
}

/// A tag with how many documents carry it.
#[derive(Debug, Serialize)]
pub struct TagCountView {
    /// Identifier.
    pub id: String,
    /// Label as typed.
    pub label: String,
    /// How many documents carry it.
    pub document_count: u64,
}

// ------------------------------------------------------------------- helpers

/// Builds a file response.
///
/// `download` chooses between `attachment` and `inline`; the viewer needs inline,
/// while an explicit download should not navigate the page.
fn attachment(content: DocumentContent, download: bool) -> Response {
    let disposition = if download { "attachment" } else { "inline" };
    let filename = content.filename.for_content_disposition();

    let mut response = (StatusCode::OK, content.bytes).into_response();
    let headers = response.headers_mut();

    if let Ok(value) = header::HeaderValue::from_str(content.media_type.mime()) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    // The ASCII `filename` is a fallback; `filename*` carries the real name for
    // anything non-ASCII. Browsers prefer the latter when both are present.
    if let Ok(value) = header::HeaderValue::from_str(&format!(
        "{disposition}; filename=\"{}\"; filename*=UTF-8''{}",
        filename.replace(|c: char| !c.is_ascii_graphic() && c != ' ', "_"),
        percent_encode(&filename)
    )) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    // A content checksum is a perfect strong validator, so conditional requests
    // work without the server having to track modification times.
    if let Ok(value) = header::HeaderValue::from_str(&format!("\"{}\"", content.checksum.to_hex()))
    {
        headers.insert(header::ETAG, value);
    }
    if let Ok(value) = header::HeaderValue::from_str("private, max-age=0, must-revalidate") {
        headers.insert(header::CACHE_CONTROL, value);
    }
    // Belt and braces against a stored file being interpreted as something else.
    if let Ok(value) = header::HeaderValue::from_str("nosniff") {
        headers.insert(header::X_CONTENT_TYPE_OPTIONS, value);
    }

    response
}

/// Percent-encodes everything outside the RFC 5987 attribute character set.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(*byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Reads an upload from a multipart body.
///
/// Field order is not guaranteed by the format, so every part is read before any
/// of them is interpreted.
async fn read_upload(mut multipart: Multipart) -> ApiResult<UploadRequest> {
    let mut filename = None;
    let mut declared_media_type = None;
    let mut bytes = None;
    let mut category_id = None;
    let mut title = None;
    let mut tags = Vec::new();

    while let Some(field) =
        multipart
            .next_field()
            .await
            .map_err(|error| ApiError::MalformedRequest {
                code: "request_body_invalid",
                message: format!("could not read the upload: {error}"),
            })?
    {
        let name = field.name().unwrap_or_default().to_owned();
        match name.as_str() {
            "file" => {
                filename = field.file_name().map(str::to_owned);
                declared_media_type = field.content_type().map(str::to_owned);
                bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|error| ApiError::MalformedRequest {
                            code: "request_body_invalid",
                            message: format!("could not read the file part: {error}"),
                        })?
                        .to_vec(),
                );
            }
            "category_id" => {
                let raw = read_text(field).await?;
                if !raw.is_empty() {
                    category_id = Some(raw.parse::<CategoryId>().map_err(|_| {
                        ApiError::MalformedRequest {
                            code: "request_body_invalid",
                            message: "category_id must be an identifier".to_owned(),
                        }
                    })?);
                }
            }
            "title" => {
                let raw = read_text(field).await?;
                if !raw.is_empty() {
                    title = Some(raw);
                }
            }
            "tags" => {
                // Accepted either as one comma-separated part or as repeated parts,
                // because both are natural for a client to send.
                let raw = read_text(field).await?;
                tags.extend(
                    raw.split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned),
                );
            }
            // Unknown parts are ignored rather than rejected, so a client can send
            // extra form fields without the request failing.
            _ => {}
        }
    }

    let bytes = bytes.ok_or(ApiError::MalformedRequest {
        code: "request_body_invalid",
        message: "the upload must include a file part named \"file\"".to_owned(),
    })?;
    let filename = filename.ok_or(ApiError::MalformedRequest {
        code: "request_body_invalid",
        message: "the file part must carry a filename".to_owned(),
    })?;

    Ok(UploadRequest {
        filename,
        declared_media_type,
        bytes,
        category_id,
        title,
        tags,
        source_path: None,
    })
}

/// Reads a multipart field as trimmed text.
async fn read_text(field: axum::extract::multipart::Field<'_>) -> ApiResult<String> {
    Ok(field
        .text()
        .await
        .map_err(|error| ApiError::MalformedRequest {
            code: "request_body_invalid",
            message: format!("could not read a text part: {error}"),
        })?
        .trim()
        .to_owned())
}

/// Formats a timestamp as RFC 3339 in UTC.
fn format_time(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_default()
}
