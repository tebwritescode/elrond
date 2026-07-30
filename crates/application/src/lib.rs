//! Elrond application layer.
//!
//! Use cases live here, expressed against traits ("ports") that the
//! infrastructure crate implements. Nothing in this crate knows whether storage
//! is SQLite or PostgreSQL, or whether the transport is HTTP.

pub mod auth;
pub mod binders;
pub mod categories;
pub mod documents;
pub mod error;
pub mod ports;

#[cfg(feature = "testing")]
pub mod testing;

pub use error::{ApplicationError, ApplicationResult};
