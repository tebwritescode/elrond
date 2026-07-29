use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use async_trait::async_trait;
use elrond_domain::{
    auth::{AuthenticatedUser, InitialAdmin, NewSession, UserCredentials},
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
    #[error("the username or password is incorrect")]
    InvalidCredentials,
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
    async fn credentials_by_username(
        &self,
        username: &str,
    ) -> Result<Option<UserCredentials>, AuthError>;
    async fn create_session(&self, session: NewSession) -> Result<(), AuthError>;
    async fn user_by_session(
        &self,
        token_hash: &str,
        now: i64,
    ) -> Result<Option<AuthenticatedUser>, AuthError>;
    async fn delete_session(&self, token_hash: &str) -> Result<(), AuthError>;
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

        let user_id = Uuid::new_v4().to_string();
        let session = Self::new_session(&user_id)?;

        self.repository
            .create_initial_admin(
                InitialAdmin {
                    id: user_id.clone(),
                    username: username.to_owned(),
                    password_hash,
                },
                session.stored,
            )
            .await?;

        Ok(CreatedSession {
            token: session.token,
            username: username.to_owned(),
            max_age_seconds: Self::SESSION_DURATION_SECONDS,
        })
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<CreatedSession, AuthError> {
        let credentials = self
            .repository
            .credentials_by_username(username.trim())
            .await?
            .ok_or(AuthError::InvalidCredentials)?;
        let parsed_hash = PasswordHash::new(&credentials.password_hash)
            .map_err(|_| AuthError::InvalidCredentials)?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| AuthError::InvalidCredentials)?;

        let session = Self::new_session(&credentials.id)?;
        self.repository.create_session(session.stored).await?;

        Ok(CreatedSession {
            token: session.token,
            username: credentials.username,
            max_age_seconds: Self::SESSION_DURATION_SECONDS,
        })
    }

    pub async fn current_user(
        &self,
        session_token: &str,
    ) -> Result<Option<AuthenticatedUser>, AuthError> {
        self.repository
            .user_by_session(&hash_session_token(session_token), unix_timestamp()?)
            .await
    }

    pub async fn logout(&self, session_token: &str) -> Result<(), AuthError> {
        self.repository
            .delete_session(&hash_session_token(session_token))
            .await
    }

    fn new_session(user_id: &str) -> Result<GeneratedSession, AuthError> {
        let mut session_bytes = [0_u8; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut session_bytes)
            .map_err(|_| AuthError::SessionGeneration)?;
        let token = hex::encode(session_bytes);

        Ok(GeneratedSession {
            stored: NewSession {
                token_hash: hash_session_token(&token),
                user_id: user_id.to_owned(),
                expires_at: unix_timestamp()? + Self::SESSION_DURATION_SECONDS,
            },
            token,
        })
    }
}

struct GeneratedSession {
    token: String,
    stored: NewSession,
}

fn hash_session_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn unix_timestamp() -> Result<i64, AuthError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthError::SessionGeneration)?
        .as_secs() as i64)
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

        async fn credentials_by_username(
            &self,
            _username: &str,
        ) -> Result<Option<UserCredentials>, AuthError> {
            Ok(None)
        }

        async fn create_session(&self, _session: NewSession) -> Result<(), AuthError> {
            Ok(())
        }

        async fn user_by_session(
            &self,
            _token_hash: &str,
            _now: i64,
        ) -> Result<Option<AuthenticatedUser>, AuthError> {
            Ok(None)
        }

        async fn delete_session(&self, _token_hash: &str) -> Result<(), AuthError> {
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
