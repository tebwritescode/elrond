//! SQLite-backed account storage.

use async_trait::async_trait;
use elrond_application::ports::{
    Credentialed, NewUser, PasswordHash, RepositoryError, UserRepository,
};
use elrond_domain::{Role, User, UserId, Username};
use sqlx::sqlite::SqliteRow;
use sqlx::{Pool, Row, Sqlite};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::db::{Database, classify};

/// Columns selected whenever a full account is read, kept in one place so every
/// query and the row mapper cannot drift apart.
const USER_COLUMNS: &str = "id, username, role, is_active, created_at, updated_at, password_hash";

/// Accounts stored in SQLite.
#[derive(Debug, Clone)]
pub struct SqliteUserRepository {
    pool: Pool<Sqlite>,
}

impl SqliteUserRepository {
    /// Binds the repository to a connected database.
    pub fn new(database: &Database) -> Self {
        Self {
            pool: database.pool().clone(),
        }
    }
}

#[async_trait]
impl UserRepository for SqliteUserRepository {
    async fn count(&self) -> Result<u64, RepositoryError> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;
        // COUNT(*) is never negative, so clamping then reinterpreting the sign is
        // exact rather than lossy.
        Ok(count.max(0).cast_unsigned())
    }

    async fn find_credentialed_by_username(
        &self,
        username: &Username,
    ) -> Result<Option<Credentialed>, RepositoryError> {
        let row = sqlx::query(&format!(
            "SELECT {USER_COLUMNS} FROM users WHERE username = ?1"
        ))
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(RepositoryError::backend)?;

        row.as_ref().map(map_credentialed).transpose()
    }

    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, RepositoryError> {
        let row = sqlx::query(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?1"))
            .bind(id.into_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;

        row.as_ref()
            .map(|row| map_credentialed(row).map(|record| record.user))
            .transpose()
    }

    async fn insert(&self, new_user: NewUser) -> Result<User, RepositoryError> {
        sqlx::query(
            "INSERT INTO users (id, username, role, password_hash, is_active, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?5)",
        )
        .bind(new_user.id.into_uuid())
        .bind(new_user.username.as_str())
        .bind(new_user.role.as_str())
        .bind(new_user.password_hash.expose())
        .bind(new_user.created_at)
        .execute(&self.pool)
        .await
        .map_err(|error| classify(error, "user", "username"))?;

        Ok(User {
            id: new_user.id,
            username: new_user.username,
            role: new_user.role,
            is_active: true,
            created_at: new_user.created_at,
            updated_at: new_user.created_at,
        })
    }

    async fn list(&self) -> Result<Vec<User>, RepositoryError> {
        // UUIDv7 keys sort by creation time, so ordering by id gives oldest-first
        // without needing an index on created_at.
        let rows = sqlx::query(&format!("SELECT {USER_COLUMNS} FROM users ORDER BY id"))
            .fetch_all(&self.pool)
            .await
            .map_err(RepositoryError::backend)?;

        rows.iter()
            .map(|row| map_credentialed(row).map(|record| record.user))
            .collect()
    }
}

/// A stored row that could not be interpreted.
///
/// Surfaces as a backend error rather than a validation error, because the
/// caller did nothing wrong: the database contains something Elrond did not put
/// there.
#[derive(Debug, thiserror::Error)]
#[error("users.{column} holds a value this build cannot interpret ({reason})")]
struct CorruptUserRow {
    /// Offending column.
    column: &'static str,
    /// Why it could not be read.
    reason: &'static str,
}

/// Rebuilds an account and its credential from a row.
fn map_credentialed(row: &SqliteRow) -> Result<Credentialed, RepositoryError> {
    let id: Uuid = row.try_get("id").map_err(RepositoryError::backend)?;
    let username: String = row.try_get("username").map_err(RepositoryError::backend)?;
    let role: String = row.try_get("role").map_err(RepositoryError::backend)?;
    let is_active: bool = row.try_get("is_active").map_err(RepositoryError::backend)?;
    let created_at: OffsetDateTime = row
        .try_get("created_at")
        .map_err(RepositoryError::backend)?;
    let updated_at: OffsetDateTime = row
        .try_get("updated_at")
        .map_err(RepositoryError::backend)?;
    let password_hash: String = row
        .try_get("password_hash")
        .map_err(RepositoryError::backend)?;

    // Values are re-validated on the way out. The CHECK constraints and the
    // application layer should make this unreachable, but a hand-edited database
    // must fail loudly rather than produce a User that breaks domain invariants.
    let username = Username::parse(&username).map_err(|_| {
        RepositoryError::backend(CorruptUserRow {
            column: "username",
            reason: "not a valid username",
        })
    })?;
    let role: Role = role.parse().map_err(|_| {
        RepositoryError::backend(CorruptUserRow {
            column: "role",
            reason: "unknown role",
        })
    })?;

    Ok(Credentialed {
        user: User {
            id: UserId::from_uuid(id),
            username,
            role,
            is_active,
            created_at,
            updated_at,
        },
        password_hash: PasswordHash::new(password_hash),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repository() -> (Database, SqliteUserRepository) {
        let database = Database::connect_in_memory().await.expect("connects");
        let repository = SqliteUserRepository::new(&database);
        (database, repository)
    }

    fn sample(username: &str, role: Role) -> NewUser {
        NewUser {
            id: UserId::new(),
            username: Username::parse(username).expect("valid username"),
            role,
            password_hash: PasswordHash::new("$argon2id$placeholder".to_owned()),
            created_at: OffsetDateTime::from_unix_timestamp(1_767_225_600).expect("valid time"),
        }
    }

    #[tokio::test]
    async fn a_fresh_database_has_no_accounts() {
        let (_db, users) = repository().await;
        assert_eq!(users.count().await.expect("counts"), 0);
    }

    #[tokio::test]
    async fn an_inserted_account_round_trips() {
        let (_db, users) = repository().await;
        let inserted = users
            .insert(sample("records.admin", Role::Admin))
            .await
            .expect("insert succeeds");

        let found = users
            .find_by_id(inserted.id)
            .await
            .expect("query succeeds")
            .expect("account exists");
        assert_eq!(found, inserted);
        assert_eq!(users.count().await.expect("counts"), 1);
    }

    #[tokio::test]
    async fn lookup_by_username_returns_the_credential() {
        let (_db, users) = repository().await;
        let inserted = users
            .insert(sample("editor", Role::Editor))
            .await
            .expect("insert succeeds");

        let found = users
            .find_credentialed_by_username(&inserted.username)
            .await
            .expect("query succeeds")
            .expect("account exists");
        assert_eq!(found.user.id, inserted.id);
        assert_eq!(found.password_hash.expose(), "$argon2id$placeholder");
    }

    #[tokio::test]
    async fn a_missing_account_is_none_not_an_error() {
        let (_db, users) = repository().await;
        let username = Username::parse("nobody").expect("valid");
        assert!(
            users
                .find_credentialed_by_username(&username)
                .await
                .expect("query succeeds")
                .is_none()
        );
        assert!(
            users
                .find_by_id(UserId::new())
                .await
                .expect("query succeeds")
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_duplicate_username_is_a_unique_violation_not_a_backend_error() {
        let (_db, users) = repository().await;
        users
            .insert(sample("records.admin", Role::Admin))
            .await
            .expect("first insert succeeds");

        let error = users
            .insert(sample("records.admin", Role::Editor))
            .await
            .expect_err("second insert is refused");
        assert!(
            matches!(
                error,
                RepositoryError::UniqueViolation {
                    resource: "user",
                    field: "username"
                }
            ),
            "expected a unique violation, got {error:?}"
        );
    }

    #[tokio::test]
    async fn a_case_variant_username_cannot_create_a_second_account() {
        let (_db, users) = repository().await;
        users
            .insert(sample("archivist", Role::Editor))
            .await
            .expect("first insert succeeds");

        // Normalization happens in the domain, so both spellings hit the same
        // unique index rather than producing two look-alike accounts.
        let error = users
            .insert(sample("ARCHIVIST", Role::Editor))
            .await
            .expect_err("refused");
        assert!(matches!(error, RepositoryError::UniqueViolation { .. }));
    }

    #[tokio::test]
    async fn every_role_survives_a_storage_round_trip() {
        let (_db, users) = repository().await;
        for (index, role) in Role::ALL.into_iter().enumerate() {
            let inserted = users
                .insert(sample(&format!("user{index}"), role))
                .await
                .expect("insert succeeds");
            let found = users
                .find_by_id(inserted.id)
                .await
                .expect("query succeeds")
                .expect("exists");
            assert_eq!(found.role, role);
        }
    }

    #[tokio::test]
    async fn accounts_are_listed_oldest_first() {
        let (_db, users) = repository().await;
        let mut expected = Vec::new();
        for index in 0..5 {
            expected.push(
                users
                    .insert(sample(&format!("user{index}"), Role::Viewer))
                    .await
                    .expect("insert succeeds")
                    .id,
            );
        }

        let listed: Vec<_> = users
            .list()
            .await
            .expect("query succeeds")
            .into_iter()
            .map(|user| user.id)
            .collect();
        assert_eq!(listed, expected);
    }

    #[tokio::test]
    async fn timestamps_and_flags_persist_exactly() {
        let (_db, users) = repository().await;
        let new_user = sample("archivist", Role::Reviewer);
        let created_at = new_user.created_at;
        let inserted = users.insert(new_user).await.expect("insert succeeds");

        let found = users
            .find_by_id(inserted.id)
            .await
            .expect("query succeeds")
            .expect("exists");
        assert_eq!(found.created_at, created_at);
        assert_eq!(found.updated_at, created_at);
        assert!(found.is_active);
    }

    #[tokio::test]
    async fn a_hand_corrupted_row_fails_loudly() {
        let (db, users) = repository().await;
        let inserted = users
            .insert(sample("records.admin", Role::Admin))
            .await
            .expect("insert succeeds");

        // Bypass the application layer the way a manual database edit would.
        sqlx::query("UPDATE users SET username = 'not a username' WHERE id = ?1")
            .bind(inserted.id.into_uuid())
            .execute(db.pool())
            .await
            .expect("update applied");

        let error = users
            .find_by_id(inserted.id)
            .await
            .expect_err("mapping must reject an invalid username");
        assert!(matches!(error, RepositoryError::Backend(_)));
    }
}
