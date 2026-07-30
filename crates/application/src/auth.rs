//! Authentication and first-run setup use cases.

use std::sync::Arc;

use elrond_domain::{PasswordPolicy, Role, SessionId, User, UserId, Username};
use time::OffsetDateTime;

use crate::error::{ApplicationError, ApplicationResult};
use crate::ports::{
    Clock, NewSession, NewUser, PasswordHasher, SessionPolicy, SessionRepository, SessionToken,
    SessionTokens, UserRepository,
};

/// Whether the instance still needs its first administrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupState {
    /// No accounts exist; the setup endpoint is open.
    RequiresSetup,
    /// At least one account exists; the setup endpoint is closed.
    Ready,
}

impl SetupState {
    /// Whether first-run setup may still be performed.
    pub fn requires_setup(self) -> bool {
        matches!(self, Self::RequiresSetup)
    }
}

/// Input for creating the first administrator.
#[derive(Debug, Clone)]
pub struct FirstRunSetupInput {
    /// Raw username, validated by the use case.
    pub username: String,
    /// Raw password, validated then immediately hashed.
    pub password: String,
}

/// Input for signing in.
#[derive(Debug, Clone)]
pub struct SignInInput {
    /// Raw username.
    pub username: String,
    /// Raw password.
    pub password: String,
}

/// A newly established session.
///
/// The token is present exactly once, on the response that creates it.
#[derive(Debug)]
pub struct EstablishedSession {
    /// The signed-in account.
    pub user: User,
    /// Bearer token for the client cookie.
    pub token: SessionToken,
    /// Hard expiry of the session.
    pub expires_at: OffsetDateTime,
}

/// An authenticated caller resolved from a session token.
#[derive(Debug, Clone)]
pub struct Authenticated {
    /// The account behind the request.
    pub user: User,
    /// The session it arrived on, so it can be revoked on sign-out.
    pub session_id: SessionId,
}

impl Authenticated {
    /// Enforces a minimum role, returning [`ApplicationError::Forbidden`]
    /// otherwise.
    pub fn require_role(&self, required: Role) -> ApplicationResult<()> {
        if self.user.role.satisfies(required) {
            Ok(())
        } else {
            Err(ApplicationError::Forbidden { required })
        }
    }
}

/// Authentication use cases.
#[derive(Clone)]
pub struct AuthService {
    users: Arc<dyn UserRepository>,
    sessions: Arc<dyn SessionRepository>,
    hasher: Arc<dyn PasswordHasher>,
    tokens: Arc<dyn SessionTokens>,
    clock: Arc<dyn Clock>,
    policy: SessionPolicy,
}

impl AuthService {
    /// Wires the use cases to concrete adapters.
    pub fn new(
        users: Arc<dyn UserRepository>,
        sessions: Arc<dyn SessionRepository>,
        hasher: Arc<dyn PasswordHasher>,
        tokens: Arc<dyn SessionTokens>,
        clock: Arc<dyn Clock>,
        policy: SessionPolicy,
    ) -> Self {
        Self {
            users,
            sessions,
            hasher,
            tokens,
            clock,
            policy,
        }
    }

    /// Reports whether first-run setup is still pending.
    pub async fn setup_state(&self) -> ApplicationResult<SetupState> {
        let count = self.users.count().await?;
        Ok(if count == 0 {
            SetupState::RequiresSetup
        } else {
            SetupState::Ready
        })
    }

    /// Creates the first administrator and signs it in.
    ///
    /// Closed permanently once any account exists, so the endpoint cannot be
    /// used to mint a second administrator later.
    pub async fn complete_first_run_setup(
        &self,
        input: FirstRunSetupInput,
    ) -> ApplicationResult<EstablishedSession> {
        if self.setup_state().await? == SetupState::Ready {
            return Err(ApplicationError::SetupAlreadyCompleted);
        }

        let username = Username::parse(&input.username)?;
        PasswordPolicy::validate(&input.password)?;

        let password_hash = self.hasher.hash(input.password).await?;
        let user = self
            .users
            .insert(NewUser {
                id: UserId::new(),
                username,
                role: Role::Admin,
                password_hash,
                created_at: self.clock.now(),
            })
            .await?;

        tracing::info!(user_id = %user.id, "first administrator created");
        self.establish_session(user).await
    }

    /// Verifies credentials and starts a session.
    pub async fn sign_in(&self, input: SignInInput) -> ApplicationResult<EstablishedSession> {
        // An unparseable username is reported as bad credentials rather than as a
        // validation error, so the endpoint gives away nothing about which names
        // are registered.
        let Ok(username) = Username::parse(&input.username) else {
            return Err(ApplicationError::InvalidCredentials);
        };

        let Some(candidate) = self.users.find_credentialed_by_username(&username).await? else {
            // Hash the supplied password anyway. Returning early here would make
            // "unknown username" measurably faster than "wrong password" and turn
            // response latency into an account-enumeration oracle.
            let _ = self.hasher.hash(input.password).await;
            return Err(ApplicationError::InvalidCredentials);
        };

        let matches = self
            .hasher
            .verify(input.password, candidate.password_hash)
            .await?;
        if !matches {
            return Err(ApplicationError::InvalidCredentials);
        }

        // Checked after verification so a deactivated account is not revealed to
        // someone who does not already know its password.
        if !candidate.user.can_authenticate() {
            return Err(ApplicationError::AccountDisabled);
        }

        tracing::info!(user_id = %candidate.user.id, "sign-in succeeded");
        self.establish_session(candidate.user).await
    }

    /// Resolves a bearer token to an authenticated caller.
    ///
    /// Enforces both the idle timeout and the absolute lifetime, and revokes
    /// sessions that fail either check so a stale cookie cannot be retried.
    pub async fn authenticate(&self, token: &SessionToken) -> ApplicationResult<Authenticated> {
        let fingerprint = self.tokens.fingerprint(token);
        let Some(session) = self.sessions.find_by_fingerprint(&fingerprint).await? else {
            return Err(ApplicationError::NotAuthenticated);
        };

        let now = self.clock.now();
        let idle_deadline = session.last_seen_at + self.policy.idle_timeout;
        if now >= session.expires_at || now >= idle_deadline {
            // Clean up eagerly rather than waiting for the sweeper.
            self.sessions.delete(session.id).await?;
            return Err(ApplicationError::NotAuthenticated);
        }

        let Some(user) = self.users.find_by_id(session.user_id).await? else {
            // The account was removed while the session lived on.
            self.sessions.delete(session.id).await?;
            return Err(ApplicationError::NotAuthenticated);
        };

        if !user.can_authenticate() {
            self.sessions.delete_for_user(user.id).await?;
            return Err(ApplicationError::AccountDisabled);
        }

        self.sessions.touch(session.id, now).await?;
        Ok(Authenticated {
            user,
            session_id: session.id,
        })
    }

    /// Lists every account, oldest first.
    ///
    /// Authorization is the caller's responsibility; the HTTP layer gates this
    /// behind the admin role.
    pub async fn list_users(&self) -> ApplicationResult<Vec<User>> {
        Ok(self.users.list().await?)
    }

    /// Revokes a single session. Idempotent.
    pub async fn sign_out(&self, session_id: SessionId) -> ApplicationResult<()> {
        self.sessions.delete(session_id).await?;
        Ok(())
    }

    /// Revokes every session for an account. Returns how many were revoked.
    pub async fn sign_out_everywhere(&self, user_id: UserId) -> ApplicationResult<u64> {
        Ok(self.sessions.delete_for_user(user_id).await?)
    }

    /// Removes expired sessions. Returns how many were purged.
    pub async fn purge_expired_sessions(&self) -> ApplicationResult<u64> {
        Ok(self.sessions.delete_expired(self.clock.now()).await?)
    }

    /// Mints a token and persists the matching session record.
    async fn establish_session(&self, user: User) -> ApplicationResult<EstablishedSession> {
        let token = self.tokens.generate();
        let fingerprint = self.tokens.fingerprint(&token);
        let now = self.clock.now();
        let expires_at = now + self.policy.absolute_lifetime;

        self.sessions
            .insert(NewSession {
                id: SessionId::new(),
                user_id: user.id,
                token_fingerprint: fingerprint,
                created_at: now,
                expires_at,
            })
            .await?;

        Ok(EstablishedSession {
            user,
            token,
            expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use time::Duration;

    use super::*;
    use crate::testing::TestEnvironment;

    /// The passphrase used throughout these tests.
    const PASSPHRASE: &str = "correct horse battery";

    fn setup_input() -> FirstRunSetupInput {
        FirstRunSetupInput {
            username: "  Records.Admin  ".to_owned(),
            password: PASSPHRASE.to_owned(),
        }
    }

    #[tokio::test]
    async fn a_fresh_instance_requires_setup() {
        let env = TestEnvironment::new();
        assert_eq!(
            env.auth().setup_state().await.expect("query succeeds"),
            SetupState::RequiresSetup
        );
    }

    #[tokio::test]
    async fn first_run_setup_creates_an_admin_and_signs_it_in() {
        let env = TestEnvironment::new();
        let established = env
            .auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");

        assert_eq!(established.user.role, Role::Admin);
        assert_eq!(established.user.username.as_str(), "records.admin");
        assert!(!established.token.expose().is_empty());

        assert_eq!(
            env.auth().setup_state().await.expect("query succeeds"),
            SetupState::Ready
        );
    }

    #[tokio::test]
    async fn setup_cannot_run_twice() {
        let env = TestEnvironment::new();
        env.auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("first setup succeeds");

        let error = env
            .auth()
            .complete_first_run_setup(FirstRunSetupInput {
                username: "second.admin".to_owned(),
                ..setup_input()
            })
            .await
            .expect_err("second setup is refused");
        assert_eq!(error.code(), "setup_already_completed");
    }

    #[tokio::test]
    async fn setup_rejects_a_weak_password() {
        let env = TestEnvironment::new();
        let error = env
            .auth()
            .complete_first_run_setup(FirstRunSetupInput {
                password: "short".to_owned(),
                ..setup_input()
            })
            .await
            .expect_err("password policy applies");
        assert_eq!(error.code(), "field_too_short");
        // The failed attempt must not leave a half-created instance behind.
        assert!(
            env.auth()
                .setup_state()
                .await
                .expect("query succeeds")
                .requires_setup()
        );
    }

    #[tokio::test]
    async fn setup_rejects_a_malformed_username() {
        let env = TestEnvironment::new();
        let error = env
            .auth()
            .complete_first_run_setup(FirstRunSetupInput {
                username: "no".to_owned(),
                ..setup_input()
            })
            .await
            .expect_err("username policy applies");
        assert_eq!(error.field(), Some("username"));
    }

    #[tokio::test]
    async fn sign_in_accepts_correct_credentials_case_insensitively() {
        let env = TestEnvironment::new();
        env.auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");

        let established = env
            .auth()
            .sign_in(SignInInput {
                username: "RECORDS.ADMIN".to_owned(),
                password: PASSPHRASE.to_owned(),
            })
            .await
            .expect("sign-in succeeds");
        assert_eq!(established.user.username.as_str(), "records.admin");
    }

    #[tokio::test]
    async fn wrong_password_and_unknown_account_are_indistinguishable() {
        let env = TestEnvironment::new();
        env.auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");

        let wrong_password = env
            .auth()
            .sign_in(SignInInput {
                username: "records.admin".to_owned(),
                password: "not the password".to_owned(),
            })
            .await
            .expect_err("refused");
        let unknown_account = env
            .auth()
            .sign_in(SignInInput {
                username: "nobody".to_owned(),
                password: "not the password".to_owned(),
            })
            .await
            .expect_err("refused");
        let malformed_username = env
            .auth()
            .sign_in(SignInInput {
                username: "!!".to_owned(),
                password: "not the password".to_owned(),
            })
            .await
            .expect_err("refused");

        assert_eq!(wrong_password.code(), "invalid_credentials");
        assert_eq!(unknown_account.code(), "invalid_credentials");
        assert_eq!(malformed_username.code(), "invalid_credentials");
    }

    #[tokio::test]
    async fn a_valid_token_authenticates() {
        let env = TestEnvironment::new();
        let established = env
            .auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");

        let authenticated = env
            .auth()
            .authenticate(&established.token)
            .await
            .expect("token resolves");
        assert_eq!(authenticated.user.id, established.user.id);
    }

    #[tokio::test]
    async fn an_unknown_token_is_rejected() {
        let env = TestEnvironment::new();
        let error = env
            .auth()
            .authenticate(&SessionToken::new("fabricated".to_owned()))
            .await
            .expect_err("refused");
        assert_eq!(error.code(), "not_authenticated");
    }

    #[tokio::test]
    async fn signing_out_revokes_the_token() {
        let env = TestEnvironment::new();
        let established = env
            .auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");
        let authenticated = env
            .auth()
            .authenticate(&established.token)
            .await
            .expect("token resolves");

        env.auth()
            .sign_out(authenticated.session_id)
            .await
            .expect("sign-out succeeds");

        let error = env
            .auth()
            .authenticate(&established.token)
            .await
            .expect_err("token no longer works");
        assert_eq!(error.code(), "not_authenticated");
    }

    #[tokio::test]
    async fn an_idle_session_expires() {
        let env = TestEnvironment::new();
        let established = env
            .auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");

        env.clock()
            .advance(SessionPolicy::default().idle_timeout + Duration::seconds(1));

        let error = env
            .auth()
            .authenticate(&established.token)
            .await
            .expect_err("idle session is refused");
        assert_eq!(error.code(), "not_authenticated");
    }

    #[tokio::test]
    async fn activity_extends_the_idle_window() {
        let env = TestEnvironment::new();
        let established = env
            .auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");
        let idle_timeout = SessionPolicy::default().idle_timeout;

        // Stay active either side of the idle boundary.
        for _ in 0..3 {
            env.clock().advance(idle_timeout - Duration::minutes(1));
            env.auth()
                .authenticate(&established.token)
                .await
                .expect("still authenticated");
        }
    }

    #[tokio::test]
    async fn the_absolute_lifetime_wins_over_activity() {
        let env = TestEnvironment::new();
        let established = env
            .auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");
        let policy = SessionPolicy::default();

        // Keep the session warm right up to the hard limit.
        let step = policy.idle_timeout - Duration::minutes(1);
        let mut elapsed = Duration::ZERO;
        while elapsed + step < policy.absolute_lifetime {
            env.clock().advance(step);
            elapsed += step;
            env.auth()
                .authenticate(&established.token)
                .await
                .expect("still authenticated");
        }

        env.clock().advance(policy.absolute_lifetime);
        let error = env
            .auth()
            .authenticate(&established.token)
            .await
            .expect_err("hard expiry cannot be extended by activity");
        assert_eq!(error.code(), "not_authenticated");
    }

    #[tokio::test]
    async fn expired_sessions_are_purged() {
        let env = TestEnvironment::new();
        env.auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");

        assert_eq!(
            env.auth()
                .purge_expired_sessions()
                .await
                .expect("purge succeeds"),
            0
        );

        env.clock()
            .advance(SessionPolicy::default().absolute_lifetime + Duration::seconds(1));
        assert_eq!(
            env.auth()
                .purge_expired_sessions()
                .await
                .expect("purge succeeds"),
            1
        );
    }

    #[tokio::test]
    async fn a_deactivated_account_loses_its_sessions() {
        let env = TestEnvironment::new();
        let established = env
            .auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");

        env.deactivate(established.user.id);

        let error = env
            .auth()
            .authenticate(&established.token)
            .await
            .expect_err("refused");
        assert_eq!(error.code(), "account_disabled");
        // Revocation is a side effect, so a retry cannot succeed either.
        assert_eq!(
            env.auth()
                .authenticate(&established.token)
                .await
                .expect_err("still refused")
                .code(),
            "not_authenticated"
        );
    }

    #[tokio::test]
    async fn role_requirements_are_enforced() {
        let env = TestEnvironment::new();
        let established = env
            .auth()
            .complete_first_run_setup(setup_input())
            .await
            .expect("setup succeeds");
        let authenticated = env
            .auth()
            .authenticate(&established.token)
            .await
            .expect("token resolves");

        assert!(authenticated.require_role(Role::Admin).is_ok());
        assert!(authenticated.require_role(Role::Viewer).is_ok());

        let viewer = Authenticated {
            user: User {
                role: Role::Viewer,
                ..authenticated.user.clone()
            },
            session_id: authenticated.session_id,
        };
        let error = viewer
            .require_role(Role::Editor)
            .expect_err("viewers cannot write");
        assert_eq!(error.code(), "forbidden");
    }
}
