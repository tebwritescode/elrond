use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryOverview {
    pub setup_required: bool,
    pub documents: i64,
    pub categories: i64,
    pub binders: i64,
    pub pending_reviews: i64,
    pub stirling_configured: bool,
}
