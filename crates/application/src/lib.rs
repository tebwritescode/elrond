use std::sync::Arc;

use async_trait::async_trait;
use elrond_domain::library::LibraryOverview;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("the library repository could not complete the operation")]
    Repository(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait]
pub trait LibraryRepository: Send + Sync {
    async fn overview(
        &self,
        stirling_configured: bool,
    ) -> Result<LibraryOverview, ApplicationError>;
}

#[derive(Clone)]
pub struct LibraryService {
    repository: Arc<dyn LibraryRepository>,
    stirling_configured: bool,
}

impl LibraryService {
    pub fn new(repository: Arc<dyn LibraryRepository>, stirling_configured: bool) -> Self {
        Self {
            repository,
            stirling_configured,
        }
    }

    pub async fn overview(&self) -> Result<LibraryOverview, ApplicationError> {
        self.repository.overview(self.stirling_configured).await
    }
}
