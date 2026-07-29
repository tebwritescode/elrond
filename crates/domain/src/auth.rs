use serde::Serialize;

#[derive(Debug, Clone)]
pub struct InitialAdmin {
    pub id: String,
    pub username: String,
    pub password_hash: String,
}

#[derive(Debug, Clone)]
pub struct NewSession {
    pub token_hash: String,
    pub user_id: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct UserCredentials {
    pub id: String,
    pub username: String,
    pub password_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthenticatedUser {
    pub id: String,
    pub username: String,
    pub role: String,
}
