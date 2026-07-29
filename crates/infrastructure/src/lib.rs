//! Elrond infrastructure layer.
//!
//! Concrete adapters for the ports declared in `elrond-application`. Everything
//! that knows about SQLite, Argon2, the operating system clock, or the random
//! number generator lives here and nowhere else.

pub mod clock;
pub mod db;
pub mod security;
pub mod sessions;
pub mod users;

pub use clock::SystemClock;
pub use db::{Database, DatabaseError, DatabaseSettings};
pub use security::{Argon2idHasher, RandomSessionTokens};
pub use sessions::SqliteSessionRepository;
pub use users::SqliteUserRepository;
