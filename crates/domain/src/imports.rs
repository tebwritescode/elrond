use serde::Serialize;

#[derive(Debug)]
pub struct PreparedImport {
    pub categories: Vec<Vec<String>>,
    pub documents: Vec<PreparedDocument>,
    pub unsupported_skipped: usize,
}

#[derive(Debug)]
pub struct PreparedDocument {
    pub category_path: Vec<String>,
    pub filename: String,
    pub title: String,
    pub media_type: String,
    pub sha256: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub categories_created: usize,
    pub documents_imported: usize,
    pub duplicates_skipped: usize,
    pub unsupported_skipped: usize,
}
