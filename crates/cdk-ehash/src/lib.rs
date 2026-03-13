//! Ehash helper types and endpoints.

#![warn(missing_docs)]

pub mod axum;
pub mod mint;
/// Ehash request/response types.
pub mod types;
#[cfg(feature = "wallet")]
pub mod wallet;

pub use types::{EhashBatchEntry, EhashQuoteRequest, EhashQuoteResponse};
