use serde::Serialize;

use crate::conversions::ConversionStatus;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub category_id: Option<String>,
    pub category_name: Option<String>,
    pub tags: Vec<String>,
    pub version_number: i64,
    pub original_filename: String,
    pub has_pdf: bool,
    pub conversion_status: ConversionStatus,
    pub conversion_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct DocumentContent {
    pub filename: String,
    pub media_type: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorySummary {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub document_count: i64,
}
