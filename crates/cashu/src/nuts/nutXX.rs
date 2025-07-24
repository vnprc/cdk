//! Mining Share NUT (NUT-XX)
//!
//! Mining share functionality for bitcoin mining pools

use std::fmt;
use std::str::FromStr;

use bitcoin::hashes::{sha256, Hash};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
#[cfg(feature = "mint")]
use uuid::Uuid;

use super::{BlindSignature, BlindedMessage, CurrencyUnit, PublicKey};
use crate::Amount;

/// Mining share NUT error
#[derive(Debug, Error)]
pub enum Error {
    /// Unknown Quote State
    #[error("Unknown Quote State")]
    UnknownState,
    /// Amount overflow
    #[error("Amount overflow")]
    AmountOverflow,
    /// Invalid Request
    #[error("Invalid Request")]
    InvalidRequest,
    /// Invalid header hash
    #[error("Invalid header hash")]
    InvalidHeaderHash,
    /// Quote expired
    #[error("Quote expired")]
    QuoteExpired,
    /// Invalid amount
    #[error("Invalid amount: must be positive and not exceed 256")]
    InvalidAmount,
    /// No outputs provided
    #[error("No outputs provided in request")]
    NoOutputs,
    /// No blinded messages provided
    #[error("No blinded messages provided in request")]
    NoBlindedMessages,
}

/// Mining share mint quote request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct MintQuoteMiningShareRequest {
    /// Amount to mint
    pub amount: Amount,
    /// Currency unit
    pub unit: CurrencyUnit,
    /// Mining share hash (block header hash)
    pub header_hash: sha256::Hash,
    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    // TODO make mandatory
    /// Optional pubkey for NUT-20 signature validation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<PublicKey>,
    /// Blinded messages for minting
    pub blinded_messages: Vec<BlindedMessage>,
}

impl MintQuoteMiningShareRequest {
    /// Validate the mining share request
    pub fn validate(&self) -> Result<(), Error> {
        // Valid amounts are between 1 and 256 inclusive
        if self.amount == Amount::ZERO || self.amount > Amount::from(256) {
            return Err(Error::InvalidAmount);
        }

        // Header hash validation - ensure it's not all zeros
        if self.header_hash.to_byte_array().iter().all(|&b| b == 0) {
            return Err(Error::InvalidHeaderHash);
        }

        // Ensure blinded messages are provided
        if self.blinded_messages.is_empty() {
            return Err(Error::NoBlindedMessages);
        }

        Ok(())
    }
}

/// Mining share mint quote response
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
#[serde(bound = "Q: Serialize + DeserializeOwned")]
pub struct MintQuoteMiningShareResponse<Q> {
    /// Quote ID
    pub quote: Q,
    /// Request identifier (header hash)
    pub request: String,
    /// Amount
    pub amount: Option<Amount>,
    /// Currency unit
    pub unit: Option<CurrencyUnit>,
    /// Unix timestamp until which the quote is valid
    pub expiry: Option<u64>,
    // TODO make mandatory
    /// Optional pubkey for NUT-20
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<PublicKey>,
}

impl<Q: ToString> MintQuoteMiningShareResponse<Q> {
    /// Convert quote ID to string
    pub fn to_string_id(&self) -> MintQuoteMiningShareResponse<String> {
        MintQuoteMiningShareResponse {
            quote: self.quote.to_string(),
            request: self.request.clone(),
            expiry: self.expiry,
            pubkey: self.pubkey,
            amount: self.amount,
            unit: self.unit.clone(),
        }
    }

    /// Check if quote has expired
    pub fn is_expired(&self) -> bool {
        if let Some(expiry) = self.expiry {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now > expiry
        } else {
            false
        }
    }

    /// Validate the response
    pub fn validate(&self) -> Result<(), Error> {
        if self.is_expired() {
            return Err(Error::QuoteExpired);
        }

        if let Some(amount) = self.amount {
            if amount == Amount::ZERO {
                return Err(Error::InvalidAmount);
            }
        }

        Ok(())
    }
}

#[cfg(feature = "mint")]
impl From<MintQuoteMiningShareResponse<Uuid>> for MintQuoteMiningShareResponse<String> {
    fn from(value: MintQuoteMiningShareResponse<Uuid>) -> Self {
        Self {
            quote: value.quote.to_string(),
            request: value.request,
            expiry: value.expiry,
            pubkey: value.pubkey,
            amount: value.amount,
            unit: value.unit,
        }
    }
}

/// Mining share mint response
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct MintMiningShareResponse {
    /// Blind signatures
    pub signatures: Vec<BlindSignature>,
}

/// Quote state for mining shares
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub enum QuoteState {
    /// Quote has been paid and tokens can be minted
    #[default]
    Paid,
    /// Minting is in progress (internal state)
    Pending,
    /// Tokens have been issued for this quote
    Issued,
}

impl fmt::Display for QuoteState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Paid => write!(f, "PAID"),
            Self::Pending => write!(f, "PENDING"),
            Self::Issued => write!(f, "ISSUED"),
        }
    }
}

impl FromStr for QuoteState {
    type Err = Error;

    fn from_str(state: &str) -> Result<Self, Self::Err> {
        match state {
            "PAID" => Ok(Self::Paid),
            "PENDING" => Ok(Self::Pending),
            "ISSUED" => Ok(Self::Issued),
            _ => Err(Error::UnknownState),
        }
    }
}

/// Mining share melt quote request (for future use)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct MeltQuoteMiningShareRequest {
    /// Currency unit
    pub unit: CurrencyUnit,
    /// Difficulty amount to melt
    pub amount: Amount,
    /// Request identifier for associating response with request
    pub request_id: Uuid,
    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Mining share melt quote response (for future use)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
#[serde(bound = "Q: Serialize + DeserializeOwned")]
pub struct MeltQuoteMiningShareResponse<Q> {
    /// Quote ID
    pub quote: Q,
    /// Difficulty amount to melt
    pub amount: Amount,
    /// Fee reserve
    pub fee_reserve: Amount,
    /// Quote state
    pub state: QuoteState,
    /// Unix timestamp until which the quote is valid
    pub expiry: Option<u64>,
    /// Request identifier for associating response with request
    pub request_id: Uuid,
}

impl<Q: ToString> MeltQuoteMiningShareResponse<Q> {
    /// Convert quote ID to string
    pub fn to_string_id(&self) -> MeltQuoteMiningShareResponse<String> {
        MeltQuoteMiningShareResponse {
            quote: self.quote.to_string(),
            amount: self.amount,
            fee_reserve: self.fee_reserve,
            state: self.state,
            expiry: self.expiry,
            request_id: self.request_id,
        }
    }
}

#[cfg(feature = "mint")]
impl From<MeltQuoteMiningShareResponse<Uuid>> for MeltQuoteMiningShareResponse<String> {
    fn from(value: MeltQuoteMiningShareResponse<Uuid>) -> Self {
        Self {
            quote: value.quote.to_string(),
            amount: value.amount,
            fee_reserve: value.fee_reserve,
            state: value.state,
            expiry: value.expiry,
            request_id: value.request_id,
        }
    }
}
