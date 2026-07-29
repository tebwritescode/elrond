//! SQLite-backed session storage.

use async_trait::async_trait;
use elrond_application::ports::{
    NewSession, RepositoryError, SessionRecord, SessionRepository, TokenFingerprint,
};
use elrond_domain::{SessionId, UserId};
use sqlx::sqlite::SqliteRow;
use sqlx::{Pool, Row, Sqlite};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::Database;

/// Sessions stored in SQLite.
#[derive(Debug, Clone)]
pub struct SqliteSessionRepository {
    pool: Pool<Sqlite>,
}

impl SqliteSessionRepository {
    /// Binds the repository to a connected database.
    pub fn new(database: &Database) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }
}

#[async_trait]
impl SessionRepository for SqliteSessionRepository {
    async fn insert(&self, session: NewSession) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO sessions (id, user_id, token_fingerprint, created_at, last_seen_at, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
        )
        .bind(session.id.into_uuid())
        .bind(session.user_id.into_uuid())
        .bind(session.token_fingerprint.as_str())
        .bind(session.created_at)
        .bind(session.expires_at)
        .execute(&self.pool)
        .await
        .map_err(|error| {
            if let sqlx::Error::Database(ref database_error) = error {
                if database_error.is_unique_violation() {
                    return RepositoryError::UniqueViolation {
                        resource: "session",
                        field: "token",
                    };
                }
            }
            RepositoryError::backend(error)
        })?;
        Ok(())
    }

    async fn find_by_fingerprint(
        &self,
        fingerprint: &TokenFingerprint,
    ) -> Result<Option<SessionRecord>, RepositoryError> {
        let row = sqlx::query(
            "SELECT id, user_id, created_at, last_seen_at, expires_at
             FROM sessions WHERE token_fingerprint = ?1",
        )
        .bind(fingerprint.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        row.as_ref().map(map_session).transpose()
    }

    async fn touch(&self, id: SessionId, seen_at: OffsetDateTime) -> Result<(), RepositoryError> {
        sqlx::query("UPDATE sessions SET last_seen_at = ?2 WHERE id = ?1")
            .bind(id.into_uuid())
            .bind(seen_at)
            .execute(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }

    async fn delete(&self, id: SessionId) -> Result<(), RepositoryError> {
        // Deleting an absent session is not an error; sign-out must be idempotent
        // so a double-submitted form cannot produce a confusing failure.
        sqlx::query("DELETE FROM sessions WHERE id = ?1")
            .bind(id.into_uuid())
            .execute(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;
        Ok(())
    }

    async fn delete_for_user(&self, user_id: UserId) -> Result<u64, RepositoryError> {
        let result = sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
            .bind(user_id.into_uuid())
            .execute(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;
        Ok(result.rows_affected())
    }

    async fn delete_expired(&self, now: OffsetDateTime) -> Result<u64, RepositoryError> {
        let result = sqlx::query("DELETE FROM sessions WHERE expires_at <= ?1")
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;
        Ok(result.rows_affected())
    }
}

/// Rebuilds a session record from a row.
fn map_session(row: &SqliteRow) -> Result<SessionRecord, RepositoryError> {
    let id: Uuid = row.try_get("id").map_err(RepositoryError::backend)?;
    let user_id: Uuid = row.try_get("user_id").map_err(RepositoryError::backend)?;
    let created_at: OffsetDateTime = row.try_get("created_at").map_err(RepositoryError::backend)?;
    let last_seen_at: OffsetDateTime = row
        .try_get("last_seen_at")
        .map_err(RepositoryError::backend)?;
    let expires_at: OffsetDateTime = row.try_get("expires_at").map_err(RepositoryError::backend)?;

    Ok(SessionRecord {
        id: SessionId::from_uuid(id),
        user_id: UserId::from_uuid(user_id),
        created_at,
        last_seen_at,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use elrond_application::ports::{NewUser, PasswordHash, UserRepository};
    use elrond_domain::{DisplayName, EmailAddress, Role};
    use time::Duration;

    use super::*;
    use crate::users::SqliteUserRepository;

    /// Sessions have a foreign key to users, so a real account is needed first.
    async fn fixture() -> (Database, SqliteSessionRepository, UserId, OffsetDateTime) {
        let database = Database::connect_in_memory().await.expect("connects");
        let users = SqliteUserRepository::new(&database);
        let now = OffsetDateTime::from_unix_timestamp(1_767_225_600).expect("valid time");
        let user = users
            .insert(NewUser {
                id: UserId::new(),
                email: EmailAddress::parse("admin@example.org").expect("valid"),
                display_name: DisplayName::parse("Records Team").expect("valid"),
                role: Role::Admin,
                password_hash: PasswordHash::new("$argon2id$placeholder".to_owned()),
                created_at: now,
            })
            .await
            .expect("user inserted");
        let sessions = SqliteSessionRepository::new(&database);
        (database, sessions, user.id, now)
    }

    fn new_session(user_id: UserId, fingerprint: &str, now: OffsetDateTime) -> NewSession {
        NewSession {
            id: SessionId::new(),
            user_id,
            token_fingerprint: TokenFingerprint::new(fingerprint.to_owned()),
            created_at: now,
            expires_at: now + Duration::days(30),
        }
    }

    #[tokio::test]
    async fn a_session_round_trips_by_fingerprint() {
        let (_db, sessions, user_id, now) = fixture().await;
        let session = new_session(user_id, "fingerprint-a", now);
        let id = session.id;
        let fingerprint = session.token_fingerprint.clone();
        sessions.insert(session).await.expect("insert succeeds");

        let found = sessions
            .find_by_fingerprint(&fingerprint)
            .await
            .expect("query succeeds")
            .expect("session exists");
        assert_eq!(found.id, id);
        assert_eq!(found.user_id, user_id);
        assert_eq!(found.created_at, now);
        // last_seen_at starts equal to created_at so the idle window opens at
        // session creation rather than at the first subsequent request.
        assert_eq!(found.last_seen_at, now);
    }

    #[tokio::test]
    async fn an_unknown_fingerprint_is_none() {
        let (_db, sessions, _user_id, _now) = fixture().await;
        assert!(
            sessions
                .find_by_fingerprint(&TokenFingerprint::new("absent".to_owned()))
                .await
                .expect("query succeeds")
                .is_none()
        );
    }

    #[tokio::test]
    async fn touch_advances_only_last_seen_at() {
        let (_db, sessions, user_id, now) = fixture().await;
        let session = new_session(user_id, "fingerprint-b", now);
        let id = session.id;
        let fingerprint = session.token_fingerprint.clone();
        let expires_at = session.expires_at;
        sessions.insert(session).await.expect("insert succeeds");

        let later = now + Duration::hours(3);
        sessions.touch(id, later).await.expect("touch succeeds");

        let found = sessions
            .find_by_fingerprint(&fingerprint)
            .await
            .expect("query succeeds")
            .expect("exists");
        assert_eq!(found.last_seen_at, later);
        assert_eq!(found.created_at, now, "creation time must not move");
        assert_eq!(
            found.expires_at, expires_at,
            "activity must not extend the hard expiry"
        );
    }

    #[tokio::test]
    async fn deleting_a_session_is_idempotent() {
        let (_db, sessions, user_id, now) = fixture().await;
        let session = new_session(user_id, "fingerprint-c", now);
        let id = session.id;
        sessions.insert(session).await.expect("insert succeeds");

        sessions.delete(id).await.expect("first delete succeeds");
        sessions
            .delete(id)
            .await
            .expect("deleting an absent session is not an error");
    }

    #[tokio::test]
    async fn all_sessions_for_an_account_can_be_revoked() {
        let (_db, sessions, user_id, now) = fixture().await;
        for index in 0..4 {
            sessions
                .insert(new_session(user_id, &format!("fingerprint-{index}"), now))
                .await
                .expect("insert succeeds");
        }

        assert_eq!(
            sessions
                .delete_for_user(user_id)
                .await
                .expect("revocation succeeds"),
            4
        );
        assert_eq!(
            sessions
                .delete_for_user(user_id)
                .await
                .expect("second call is a no-op"),
            0
        );
    }

    #[tokio::test]
    async fn only_expired_sessions_are_purged() {
        let (_db, sessions, user_id, now) = fixture().await;
        let mut expiring = new_session(user_id, "short-lived", now);
        expiring.expires_at = now + Duration::hours(1);
        sessions.insert(expiring).await.expect("insert succeeds");
        sessions
            .insert(new_session(user_id, "long-lived", now))
            .await
            .expect("insert succeeds");

        let purged = sessions
            .delete_expired(now + Duration::hours(2))
            .await
            .expect("purge succeeds");
        assert_eq!(purged, 1);

        assert!(
            sessions
                .find_by_fingerprint(&TokenFingerprint::new("long-lived".to_owned()))
                .await
                .expect("query succeeds")
                .is_some(),
            "an unexpired session must survive the sweep"
        );
    }

    #[tokio::test]
    async fn a_duplicate_fingerprint_is_a_unique_violation() {
        let (_db, sessions, user_id, now) = fixture().await;
        sessions
            .insert(new_session(user_id, "collision", now))
            .await
            .expect("insert succeeds");

        let error = sessions
            .insert(new_session(user_id, "collision", now))
            .await
            .expect_err("the fingerprint index must reject a duplicate");
        assert!(matches!(
            error,
            RepositoryError::UniqueViolation {
                resource: "session",
                field: "token"
            }
        ));
    }

    #[tokio::test]
    async fn a_session_cannot_reference_a_missing_account() {
        let (_db, sessions, _user_id, now) = fixture().await;
        let error = sessions
            .insert(new_session(UserId::new(), "orphan", now))
            .await
            .expect_err("the foreign key must be enforced");
        assert!(matches!(error, RepositoryError::Backend(_)));
    }
}
