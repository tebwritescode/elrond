use std::time::Duration;

use async_trait::async_trait;
use elrond_application::{ConversionError, DocumentConverter};
use reqwest::{Client, multipart};

pub struct StirlingPdfConverter {
    client: Client,
    base_url: String,
    api_key: Option<String>,
}

impl StirlingPdfConverter {
    pub fn new(base_url: &str, api_key: Option<String>) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(180))
                .build()?,
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key: api_key.filter(|key| !key.trim().is_empty()),
        })
    }
}

#[async_trait]
impl DocumentConverter for StirlingPdfConverter {
    async fn convert_to_pdf(
        &self,
        filename: &str,
        media_type: &str,
        content: Vec<u8>,
    ) -> Result<Vec<u8>, ConversionError> {
        let is_image = media_type.starts_with("image/");
        let endpoint = if is_image {
            "/api/v1/convert/img/pdf"
        } else {
            "/api/v1/convert/file/pdf"
        };
        let file = multipart::Part::bytes(content)
            .file_name(filename.to_owned())
            .mime_str(media_type)
            .map_err(converter_error)?;
        let mut form = multipart::Form::new().part("fileInput", file);
        if is_image {
            form = form
                .text("fitOption", "maintainAspectRatio")
                .text("colorType", "color")
                .text("autoRotate", "false");
        }
        let mut request = self
            .client
            .post(format!("{}{endpoint}", self.base_url))
            .multipart(form);
        if let Some(api_key) = &self.api_key {
            request = request.header("X-API-KEY", api_key);
        }
        let mut response = request.send().await.map_err(converter_error)?;
        if !response.status().is_success() {
            return Err(ConversionError::Converter(format!(
                "Stirling-PDF returned HTTP {}",
                response.status()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > 150 * 1024 * 1024)
        {
            return Err(ConversionError::Converter(
                "Stirling-PDF returned an oversized response".into(),
            ));
        }
        let mut pdf = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(converter_error)? {
            if pdf.len().saturating_add(chunk.len()) > 150 * 1024 * 1024 {
                return Err(ConversionError::Converter(
                    "Stirling-PDF returned an oversized response".into(),
                ));
            }
            pdf.extend_from_slice(&chunk);
        }
        Ok(pdf)
    }
}

fn converter_error(error: impl std::error::Error) -> ConversionError {
    ConversionError::Converter(error.to_string())
}
