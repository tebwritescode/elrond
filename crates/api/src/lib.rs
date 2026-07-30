//! Elrond HTTP layer.
//!
//! Owns the wire contract: routing, cookies, CSRF, rate limiting, security
//! headers, and error shape. Business rules live in `elrond-application`; this
//! crate only translates between HTTP and use cases.

pub mod config;
pub mod cookies;
pub mod error;
pub mod extract;
pub mod rate_limit;
pub mod routes;
pub mod state;
pub mod web;

pub use config::ApiConfig;
pub use error::{ApiError, ApiResult};
pub use routes::router;
pub use state::AppState;
