//! Elrond infrastructure layer.
//!
//! Concrete adapters for the ports declared in `elrond-application`. Everything
//! that knows about SQLite, Argon2, the operating system clock, or the random
//! number generator lives here and nowhere else.

pub mod blobs;
pub mod categories;
pub mod clock;
pub mod db;
pub mod documents;
pub mod search;
pub mod security;
pub mod sessions;
pub mod tags;
pub mod users;

pub use blobs::{FilesystemBlobStore, MagicByteInspector};
pub use categories::SqliteCategoryRepository;
pub use clock::SystemClock;
pub use db::{Database, DatabaseError, DatabaseSettings};
pub use documents::SqliteDocumentRepository;
pub use search::SqliteSearchIndex;
pub use security::{Argon2idHasher, RandomSessionTokens};
pub use sessions::SqliteSessionRepository;
pub use tags::SqliteTagRepository;
pub use users::SqliteUserRepository;
