//! In-memory adapters for testing use cases without SQLite or Argon2.
//!
//! Gated behind the `testing` feature so none of this reaches a release binary.
//! The fakes deliberately implement the same ports as the real adapters, which
//! means a use-case test that passes here is exercising the real control flow.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use elrond_domain::{SessionId, User, UserId, Username};
use time::{Duration, OffsetDateTime};

use crate::auth::AuthService;
use crate::ports::{
    Clock, Credentialed, HashingError, NewSession, NewUser, PasswordHash, PasswordHasher,
    RepositoryError, SessionPolicy, SessionRecord, SessionRepository, SessionToken, SessionTokens,
    TokenFingerprint, UserRepository,
};

/// A clock the test drives by hand.
#[derive(Debug)]
pub struct FakeClock {
    now: Mutex<OffsetDateTime>,
}

impl FakeClock {
    /// Starts at a fixed, arbitrary instant so tests are reproducible.
    pub fn new() -> Self {
        Self {
            now: Mutex::new(
                OffsetDateTime::from_unix_timestamp(1_767_225_600).expect("valid timestamp"),
            ),
        }
    }

    /// Moves time forward.
    pub fn advance(&self, by: Duration) {
        let mut guard = self.now.lock().expect("clock lock");
        *guard += by;
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> OffsetDateTime {
        *self.now.lock().expect("clock lock")
    }
}

/// Accounts held in a vector.
#[derive(Debug, Default)]
pub struct InMemoryUserRepository {
    rows: Mutex<Vec<Credentialed>>,
}

impl InMemoryUserRepository {
    /// Creates an empty repository.
    pub fn new() -> Self {
        Self::default()
    }

    /// Flips an account's active flag, simulating an administrator action.
    pub fn set_active(&self, id: UserId, is_active: bool) {
        let mut rows = self.rows.lock().expect("user lock");
        if let Some(row) = rows.iter_mut().find(|row| row.user.id == id) {
            row.user.is_active = is_active;
        }
    }
}

#[async_trait]
impl UserRepository for InMemoryUserRepository {
    async fn count(&self) -> Result<u64, RepositoryError> {
        Ok(self.rows.lock().expect("user lock").len() as u64)
    }

    async fn find_credentialed_by_username(
        &self,
        username: &Username,
    ) -> Result<Option<Credentialed>, RepositoryError> {
        Ok(self
            .rows
            .lock()
            .expect("user lock")
            .iter()
            .find(|row| row.user.username == *username)
            .cloned())
    }

    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, RepositoryError> {
        Ok(self
            .rows
            .lock()
            .expect("user lock")
            .iter()
            .find(|row| row.user.id == id)
            .map(|row| row.user.clone()))
    }

    async fn insert(&self, new_user: NewUser) -> Result<User, RepositoryError> {
        let mut rows = self.rows.lock().expect("user lock");
        if rows
            .iter()
            .any(|row| row.user.username == new_user.username)
        {
            return Err(RepositoryError::UniqueViolation {
                resource: "user",
                field: "username",
            });
        }
        let user = User {
            id: new_user.id,
            username: new_user.username,
            role: new_user.role,
            is_active: true,
            created_at: new_user.created_at,
            updated_at: new_user.created_at,
        };
        rows.push(Credentialed {
            user: user.clone(),
            password_hash: new_user.password_hash,
        });
        Ok(user)
    }

    async fn list(&self) -> Result<Vec<User>, RepositoryError> {
        let mut users: Vec<User> = self
            .rows
            .lock()
            .expect("user lock")
            .iter()
            .map(|row| row.user.clone())
            .collect();
        users.sort_by_key(|user| user.id);
        Ok(users)
    }
}

/// Sessions held in a vector.
#[derive(Debug, Default)]
pub struct InMemorySessionRepository {
    rows: Mutex<Vec<(TokenFingerprint, SessionRecord)>>,
}

impl InMemorySessionRepository {
    /// Creates an empty repository.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SessionRepository for InMemorySessionRepository {
    async fn insert(&self, session: NewSession) -> Result<(), RepositoryError> {
        self.rows.lock().expect("session lock").push((
            session.token_fingerprint,
            SessionRecord {
                id: session.id,
                user_id: session.user_id,
                created_at: session.created_at,
                last_seen_at: session.created_at,
                expires_at: session.expires_at,
            },
        ));
        Ok(())
    }

    async fn find_by_fingerprint(
        &self,
        fingerprint: &TokenFingerprint,
    ) -> Result<Option<SessionRecord>, RepositoryError> {
        Ok(self
            .rows
            .lock()
            .expect("session lock")
            .iter()
            .find(|(stored, _)| stored == fingerprint)
            .map(|(_, record)| record.clone()))
    }

    async fn touch(&self, id: SessionId, seen_at: OffsetDateTime) -> Result<(), RepositoryError> {
        let mut rows = self.rows.lock().expect("session lock");
        if let Some((_, record)) = rows.iter_mut().find(|(_, record)| record.id == id) {
            record.last_seen_at = seen_at;
        }
        Ok(())
    }

    async fn delete(&self, id: SessionId) -> Result<(), RepositoryError> {
        self.rows
            .lock()
            .expect("session lock")
            .retain(|(_, record)| record.id != id);
        Ok(())
    }

    async fn delete_for_user(&self, user_id: UserId) -> Result<u64, RepositoryError> {
        let mut rows = self.rows.lock().expect("session lock");
        let before = rows.len();
        rows.retain(|(_, record)| record.user_id != user_id);
        Ok((before - rows.len()) as u64)
    }

    async fn delete_expired(&self, now: OffsetDateTime) -> Result<u64, RepositoryError> {
        let mut rows = self.rows.lock().expect("session lock");
        let before = rows.len();
        rows.retain(|(_, record)| record.expires_at > now);
        Ok((before - rows.len()) as u64)
    }
}

/// A hasher that is fast and reversible, for tests only.
///
/// Real Argon2id costs roughly 100 ms per call by design, which would make the
/// session-expiry tests take minutes.
#[derive(Debug, Default)]
pub struct FakePasswordHasher;

const FAKE_HASH_PREFIX: &str = "$fake$";

#[async_trait]
impl PasswordHasher for FakePasswordHasher {
    async fn hash(&self, password: String) -> Result<PasswordHash, HashingError> {
        Ok(PasswordHash::new(format!("{FAKE_HASH_PREFIX}{password}")))
    }

    async fn verify(&self, password: String, hash: PasswordHash) -> Result<bool, HashingError> {
        let stored = hash
            .expose()
            .strip_prefix(FAKE_HASH_PREFIX)
            .ok_or(HashingError::MalformedHash)?;
        Ok(stored == password)
    }
}

/// Deterministic, unique session tokens.
#[derive(Debug, Default)]
pub struct CountingSessionTokens {
    next: AtomicU64,
}

#[async_trait]
impl SessionTokens for CountingSessionTokens {
    fn generate(&self) -> SessionToken {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        SessionToken::new(format!("test-token-{n}"))
    }

    fn fingerprint(&self, token: &SessionToken) -> TokenFingerprint {
        TokenFingerprint::new(format!("fingerprint-of-{}", token.expose()))
    }
}

/// A fully wired [`AuthService`] backed by fakes, plus handles to drive them.
pub struct TestEnvironment {
    auth: AuthService,
    clock: Arc<FakeClock>,
    users: Arc<InMemoryUserRepository>,
}

impl TestEnvironment {
    /// Builds an environment using the default session policy.
    pub fn new() -> Self {
        Self::with_policy(SessionPolicy::default())
    }

    /// Builds an environment with a custom session policy.
    pub fn with_policy(policy: SessionPolicy) -> Self {
        let clock = Arc::new(FakeClock::new());
        let users = Arc::new(InMemoryUserRepository::new());
        let sessions = Arc::new(InMemorySessionRepository::new());
        let auth = AuthService::new(
            users.clone(),
            sessions,
            Arc::new(FakePasswordHasher),
            Arc::new(CountingSessionTokens::default()),
            clock.clone(),
            policy,
        );
        Self { auth, clock, users }
    }

    /// The service under test.
    pub fn auth(&self) -> &AuthService {
        &self.auth
    }

    /// The clock, for advancing time.
    pub fn clock(&self) -> &FakeClock {
        &self.clock
    }

    /// Deactivates an account.
    pub fn deactivate(&self, id: UserId) {
        self.users.set_active(id, false);
    }
}

impl Default for TestEnvironment {
    fn default() -> Self {
        Self::new()
    }
}
