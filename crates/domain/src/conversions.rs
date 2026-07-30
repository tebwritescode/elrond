use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversionStatus {
    Queued,
    Processing,
    Ready,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ConversionJob {
    pub id: String,
    pub lease_token: String,
    pub document_version_id: String,
    pub original_filename: String,
    pub original_media_type: String,
    pub original_storage_key: String,
}

#[derive(Debug)]
pub struct PdfDerivative {
    pub sha256: String,
    pub content: Vec<u8>,
}
