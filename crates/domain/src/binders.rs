#[derive(Debug, Clone)]
pub struct PrintableBinderDocument {
    pub title: String,
    pub category_path: String,
    pub version_number: i64,
    pub pdf_sha256: String,
    pub pdf_storage_key: String,
    pub pdf_content: Vec<u8>,
}

#[derive(Debug)]
pub struct GeneratedBinder {
    pub filename: String,
    pub content: Vec<u8>,
}
