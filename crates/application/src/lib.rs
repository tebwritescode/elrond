use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use async_trait::async_trait;
use elrond_domain::{
    auth::{InitialAdmin, NewSession},
    library::LibraryOverview,
};
use rand::TryRngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("the library repository could not complete the operation")]
    Repository(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("the username must contain 3 to 64 letters, numbers, dots, dashes, or underscores")]
    InvalidUsername,
    #[error("the password must contain 12 to 128 characters")]
    InvalidPassword,
    #[error("first-run setup has already been completed")]
    SetupCompleted,
    #[error("password hashing failed")]
    PasswordHash,
    #[error("secure session generation failed")]
    SessionGeneration,
    #[error("account storage failed")]
    Repository(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait]
pub trait LibraryRepository: Send + Sync {
    async fn overview(
        &self,
        stirling_configured: bool,
    ) -> Result<LibraryOverview, ApplicationError>;
}

#[async_trait]
pub trait AuthRepository: Send + Sync {
    async fn create_initial_admin(
        &self,
        admin: InitialAdmin,
        session: NewSession,
    ) -> Result<(), AuthError>;
}

#[derive(Clone)]
pub struct LibraryService {
    repository: Arc<dyn LibraryRepository>,
    stirling_configured: bool,
}

pub struct CreatedSession {
    pub token: String,
    pub username: String,
    pub max_age_seconds: i64,
}

#[derive(Clone)]
pub struct AuthService {
    repository: Arc<dyn AuthRepository>,
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

impl AuthService {
    const SESSION_DURATION_SECONDS: i64 = 7 * 24 * 60 * 60;

    pub fn new(repository: Arc<dyn AuthRepository>) -> Self {
        Self { repository }
    }

    pub async fn create_initial_admin(
        &self,
        username: &str,
        password: &str,
    ) -> Result<CreatedSession, AuthError> {
        let username = username.trim();
        if !(3..=64).contains(&username.len())
            || !username
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character))
        {
            return Err(AuthError::InvalidUsername);
        }
        if !(12..=128).contains(&password.chars().count()) {
            return Err(AuthError::InvalidPassword);
        }

        let mut rng = rand::rngs::OsRng;
        let mut salt_bytes = [0_u8; 16];
        rng.try_fill_bytes(&mut salt_bytes)
            .map_err(|_| AuthError::PasswordHash)?;
        let salt = SaltString::encode_b64(&salt_bytes).map_err(|_| AuthError::PasswordHash)?;
        let password_hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| AuthError::PasswordHash)?
            .to_string();

        let mut session_bytes = [0_u8; 32];
        rng.try_fill_bytes(&mut session_bytes)
            .map_err(|_| AuthError::SessionGeneration)?;
        let token = hex::encode(session_bytes);
        let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
        let user_id = Uuid::new_v4().to_string();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AuthError::SessionGeneration)?
            .as_secs() as i64;

        self.repository
            .create_initial_admin(
                InitialAdmin {
                    id: user_id.clone(),
                    username: username.to_owned(),
                    password_hash,
                },
                NewSession {
                    token_hash,
                    user_id,
                    expires_at: now + Self::SESSION_DURATION_SECONDS,
                },
            )
            .await?;

        Ok(CreatedSession {
            token,
            username: username.to_owned(),
            max_age_seconds: Self::SESSION_DURATION_SECONDS,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct RecordingAuthRepository {
        created: Mutex<Option<(InitialAdmin, NewSession)>>,
    }

    #[async_trait]
    impl AuthRepository for RecordingAuthRepository {
        async fn create_initial_admin(
            &self,
            admin: InitialAdmin,
            session: NewSession,
        ) -> Result<(), AuthError> {
            self.created
                .lock()
                .expect("test repository lock should remain available")
                .replace((admin, session));
            Ok(())
        }
    }

    #[tokio::test]
    async fn rejects_invalid_initial_credentials() {
        let repository = Arc::new(RecordingAuthRepository::default());
        let service = AuthService::new(repository);

        assert!(matches!(
            service.create_initial_admin("a", &"x".repeat(12)).await,
            Err(AuthError::InvalidUsername)
        ));
        assert!(matches!(
            service.create_initial_admin("admin", "short").await,
            Err(AuthError::InvalidPassword)
        ));
    }

    #[tokio::test]
    async fn hashes_password_and_session_material_before_storage() {
        let repository = Arc::new(RecordingAuthRepository::default());
        let service = AuthService::new(repository.clone());

        let created = service
            .create_initial_admin("admin", &"x".repeat(12))
            .await
            .expect("valid setup should succeed");
        let stored = repository
            .created
            .lock()
            .expect("test repository lock should remain available")
            .clone()
            .expect("credentials should be recorded");

        assert_eq!(stored.0.username, "admin");
        assert!(stored.0.password_hash.starts_with("$argon2id$"));
        assert_eq!(created.token.len(), 64);
        assert_eq!(stored.1.token_hash.len(), 64);
        assert_ne!(created.token, stored.1.token_hash);
        assert_eq!(stored.0.id, stored.1.user_id);
    }
}
