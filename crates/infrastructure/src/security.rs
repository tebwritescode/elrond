//! Credential hashing and session token generation.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{
    PasswordHash as PhcHash, PasswordHasher as _, PasswordVerifier, SaltString,
};
use argon2::{Algorithm, Argon2, Params, Version};
use async_trait::async_trait;
use elrond_application::ports::{
    HashingError, PasswordHash, PasswordHasher, SessionToken, SessionTokens, TokenFingerprint,
};
use sha2::{Digest, Sha256};

/// Argon2id credential hashing.
///
/// Parameters follow the OWASP Password Storage minimum for Argon2id. They are
/// stated explicitly rather than taken from `Argon2::default()` so a dependency
/// bump cannot silently weaken them, and so the cost is documented where an
/// operator will look for it.
#[derive(Debug, Clone)]
pub struct Argon2idHasher {
    params: Params,
}

impl Argon2idHasher {
    /// Memory cost in kibibytes (19 MiB).
    pub const MEMORY_KIB: u32 = 19 * 1024;
    /// Number of passes over memory.
    pub const ITERATIONS: u32 = 2;
    /// Degree of parallelism.
    pub const PARALLELISM: u32 = 1;

    /// Builds a hasher with Elrond's parameters.
    pub fn new() -> Self {
        let params = Params::new(Self::MEMORY_KIB, Self::ITERATIONS, Self::PARALLELISM, None)
            // The constants above are compile-time known and within Argon2's valid
            // ranges, so this cannot fail.
            .expect("Elrond's Argon2 parameters are valid");
        Self { params }
    }

    /// Constructs the underlying Argon2 context.
    fn argon2(&self) -> Argon2<'_> {
        Argon2::new(Algorithm::Argon2id, Version::V0x13, self.params.clone())
    }
}

impl Default for Argon2idHasher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PasswordHasher for Argon2idHasher {
    async fn hash(&self, password: String) -> Result<PasswordHash, HashingError> {
        let hasher = self.clone();
        // Argon2id is deliberately expensive. Running it on an async worker
        // would stall every other request sharing that thread, so it is moved to
        // the blocking pool.
        spawn_hashing(move || {
            let salt = SaltString::generate(&mut OsRng);
            let hash = hasher
                .argon2()
                .hash_password(password.as_bytes(), &salt)
                .map_err(|error| HashingError::Backend(Box::new(error)))?;
            Ok(PasswordHash::new(hash.to_string()))
        })
        .await
    }

    async fn verify(&self, password: String, hash: PasswordHash) -> Result<bool, HashingError> {
        let hasher = self.clone();
        spawn_hashing(move || {
            let parsed = PhcHash::new(hash.expose()).map_err(|_| HashingError::MalformedHash)?;
            match hasher
                .argon2()
                .verify_password(password.as_bytes(), &parsed)
            {
                Ok(()) => Ok(true),
                // A mismatch is an expected outcome, not a failure. Anything else
                // means the hash or the configuration is broken and must not be
                // reported as "wrong password".
                Err(argon2::password_hash::Error::Password) => Ok(false),
                Err(error) => Err(HashingError::Backend(Box::new(error))),
            }
        })
        .await
    }
}

/// Runs CPU-bound credential work on the blocking pool.
async fn spawn_hashing<T, F>(work: F) -> Result<T, HashingError>
where
    F: FnOnce() -> Result<T, HashingError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(error) => Err(HashingError::Backend(Box::new(error))),
    }
}

/// Length of generated session tokens in bytes before hex encoding.
///
/// 256 bits of entropy makes guessing infeasible and leaves no reason to add a
/// server-side counter or a shorter, riskier token.
const TOKEN_BYTES: usize = 32;

/// Session tokens drawn from the operating system CSPRNG.
#[derive(Debug, Clone, Copy, Default)]
pub struct RandomSessionTokens;

#[async_trait]
impl SessionTokens for RandomSessionTokens {
    fn generate(&self) -> SessionToken {
        let mut bytes = [0_u8; TOKEN_BYTES];
        OsRng.fill_bytes(&mut bytes);
        // Hex rather than base64 so the value is safe in a cookie without any
        // percent-encoding, and so it cannot contain a separator character.
        SessionToken::new(hex::encode(bytes))
    }

    fn fingerprint(&self, token: &SessionToken) -> TokenFingerprint {
        // A plain SHA-256 is correct here, unlike for passwords: the token is
        // already 256 bits of uniform randomness, so there is nothing for an
        // attacker to brute-force and no benefit to a slow KDF.
        let digest = Sha256::digest(token.expose().as_bytes());
        TokenFingerprint::new(hex::encode(digest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_password_verifies_against_its_own_hash() {
        let hasher = Argon2idHasher::new();
        let hash = hasher
            .hash("correct horse battery".to_owned())
            .await
            .expect("hashing succeeds");
        assert!(
            hasher
                .verify("correct horse battery".to_owned(), hash)
                .await
                .expect("verification runs")
        );
    }

    #[tokio::test]
    async fn a_wrong_password_is_a_clean_false_not_an_error() {
        let hasher = Argon2idHasher::new();
        let hash = hasher
            .hash("correct horse battery".to_owned())
            .await
            .expect("hashing succeeds");
        assert!(
            !hasher
                .verify("wrong password entirely".to_owned(), hash)
                .await
                .expect("verification runs without erroring")
        );
    }

    #[tokio::test]
    async fn the_same_password_hashes_differently_every_time() {
        let hasher = Argon2idHasher::new();
        let first = hasher
            .hash("same password here".to_owned())
            .await
            .expect("ok");
        let second = hasher
            .hash("same password here".to_owned())
            .await
            .expect("ok");
        assert_ne!(
            first.expose(),
            second.expose(),
            "a fresh salt must be used per hash"
        );
    }

    #[tokio::test]
    async fn the_stored_hash_advertises_argon2id() {
        let hasher = Argon2idHasher::new();
        let hash = hasher
            .hash("some long password".to_owned())
            .await
            .expect("ok");
        let phc = hash.expose();
        assert!(phc.starts_with("$argon2id$"), "unexpected format: {phc}");
        assert!(phc.contains(&format!("m={}", Argon2idHasher::MEMORY_KIB)));
        assert!(phc.contains(&format!("t={}", Argon2idHasher::ITERATIONS)));
        assert!(phc.contains(&format!("p={}", Argon2idHasher::PARALLELISM)));
    }

    #[tokio::test]
    async fn a_malformed_stored_hash_is_reported_distinctly() {
        let hasher = Argon2idHasher::new();
        let error = hasher
            .verify(
                "any password at all".to_owned(),
                PasswordHash::new("garbage".to_owned()),
            )
            .await
            .expect_err("malformed hashes are errors, not mismatches");
        assert!(matches!(error, HashingError::MalformedHash));
    }

    #[test]
    fn hashes_and_tokens_are_redacted_in_debug_output() {
        let hash = PasswordHash::new("$argon2id$secret".to_owned());
        assert!(!format!("{hash:?}").contains("secret"));
        assert!(!format!("{hash}").contains("secret"));

        let token = SessionToken::new("deadbeef".to_owned());
        assert!(!format!("{token:?}").contains("deadbeef"));
        assert!(!format!("{token}").contains("deadbeef"));
    }

    #[test]
    fn tokens_are_unique_and_full_length() {
        let tokens = RandomSessionTokens;
        let mut seen = std::collections::HashSet::new();
        for _ in 0..256 {
            let token = tokens.generate();
            assert_eq!(token.expose().len(), TOKEN_BYTES * 2);
            assert!(token.expose().chars().all(|c| c.is_ascii_hexdigit()));
            assert!(seen.insert(token.expose().to_owned()), "token repeated");
        }
    }

    #[test]
    fn fingerprints_are_deterministic_and_do_not_contain_the_token() {
        let tokens = RandomSessionTokens;
        let token = tokens.generate();
        let first = tokens.fingerprint(&token);
        let second = tokens.fingerprint(&token);
        assert_eq!(first, second);
        assert_ne!(first.as_str(), token.expose());
        assert!(!first.as_str().contains(token.expose()));
    }

    #[test]
    fn different_tokens_produce_different_fingerprints() {
        let tokens = RandomSessionTokens;
        let a = tokens.fingerprint(&tokens.generate());
        let b = tokens.fingerprint(&tokens.generate());
        assert_ne!(a, b);
    }
}
