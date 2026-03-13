//! Ehash helper types and endpoints.

#![warn(missing_docs)]

pub mod axum;
pub mod mint;
/// Ehash payment processor implementation.
pub mod payment;
/// Ehash request/response types.
pub mod types;
#[cfg(feature = "wallet")]
pub mod wallet;

pub use payment::EhashPaymentProcessor;
pub use types::{EhashBatchEntry, EhashQuoteRequest, EhashQuoteResponse};
