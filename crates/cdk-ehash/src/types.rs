use bitcoin::hashes::sha256;
use cdk::nuts::{CurrencyUnit, Id, MintQuoteState, PublicKey};
use cdk::Amount;
use cdk_common::nuts::nutXX::MintQuoteMiningShareRequest;
use cdk_common::nuts::nutXX::MintQuoteMiningShareResponse;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

/// Errors for ehash helpers.
#[derive(Debug, Error)]
pub enum EhashError {
    /// Header hash was invalid.
    #[error("invalid header hash")]
    InvalidHeaderHash,
}

/// Request a new ehash mint quote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EhashQuoteRequest {
    /// Amount to mint.
    pub amount: Amount,
    /// Currency unit.
    pub unit: CurrencyUnit,
    /// Mining header hash (hex).
    pub header_hash: String,
    /// Optional description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Pubkey for NUT-20 signature validation.
    pub pubkey: PublicKey,
}

/// Ehash mint quote response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(bound = "Q: Serialize + DeserializeOwned")]
pub struct EhashQuoteResponse<Q> {
    /// Quote ID.
    pub quote: Q,
    /// Request identifier (header hash).
    pub request: String,
    /// Amount.
    pub amount: Option<Amount>,
    /// Currency unit.
    pub unit: Option<CurrencyUnit>,
    /// Quote state.
    pub state: MintQuoteState,
    /// Unix timestamp until which the quote is valid.
    pub expiry: Option<u64>,
    /// Pubkey for NUT-20.
    pub pubkey: PublicKey,
    /// Keyset ID for this quote.
    pub keyset_id: Id,
    /// Amount issued for this quote.
    pub amount_issued: Amount,
}

impl<Q: ToString> EhashQuoteResponse<Q> {
    /// Convert quote ID to string.
    pub fn to_string_id(&self) -> EhashQuoteResponse<String> {
        EhashQuoteResponse {
            quote: self.quote.to_string(),
            request: self.request.clone(),
            amount: self.amount,
            unit: self.unit.clone(),
            state: self.state,
            expiry: self.expiry,
            pubkey: self.pubkey,
            keyset_id: self.keyset_id,
            amount_issued: self.amount_issued,
        }
    }
}

/// Describes a single ehash quote to include in a batch mint request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EhashBatchEntry {
    /// Quote identifier to mint.
    pub quote_id: String,
    /// Amount to mint for this quote.
    pub amount: Amount,
    /// Keyset identifier that must sign the outputs.
    pub keyset_id: Id,
}

impl EhashBatchEntry {
    /// Create a new batch entry.
    pub fn new(quote_id: String, amount: Amount, keyset_id: Id) -> Self {
        Self {
            quote_id,
            amount,
            keyset_id,
        }
    }
}

fn parse_header_hash(header_hash: &str) -> Result<sha256::Hash, EhashError> {
    sha256::Hash::from_str(header_hash).map_err(|_| EhashError::InvalidHeaderHash)
}

impl TryFrom<EhashQuoteRequest> for MintQuoteMiningShareRequest {
    type Error = EhashError;

    fn try_from(value: EhashQuoteRequest) -> Result<Self, Self::Error> {
        let header_hash = parse_header_hash(&value.header_hash)?;

        Ok(MintQuoteMiningShareRequest {
            amount: value.amount,
            unit: value.unit,
            header_hash,
            description: value.description,
            pubkey: value.pubkey,
        })
    }
}

impl<Q> From<MintQuoteMiningShareResponse<Q>> for EhashQuoteResponse<Q> {
    fn from(value: MintQuoteMiningShareResponse<Q>) -> Self {
        let state = match value.state {
            cdk_common::nuts::nutXX::QuoteState::Unpaid => MintQuoteState::Unpaid,
            cdk_common::nuts::nutXX::QuoteState::Paid => MintQuoteState::Paid,
            cdk_common::nuts::nutXX::QuoteState::Issued => MintQuoteState::Issued,
        };

        Self {
            quote: value.quote,
            request: value.request,
            amount: value.amount,
            unit: value.unit,
            state,
            expiry: value.expiry,
            pubkey: value.pubkey,
            keyset_id: value.keyset_id,
            amount_issued: value.amount_issued,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::Hash;
    use cdk::nuts::Id;

    #[test]
    fn parse_header_hash_rejects_invalid() {
        let err = parse_header_hash("nothex").unwrap_err();
        assert!(matches!(err, EhashError::InvalidHeaderHash));
    }

    #[test]
    fn mining_share_conversion_sets_state() {
        let secret = bitcoin::secp256k1::SecretKey::from_slice(&[1u8; 32]).unwrap();
        let secp_pubkey = bitcoin::secp256k1::PublicKey::from_secret_key(&cdk::SECP256K1, &secret);
        let pubkey = PublicKey::from_slice(&secp_pubkey.serialize()).unwrap();
        let keyset_id = Id::from_bytes(&[0u8; 8]).unwrap();

        let response = MintQuoteMiningShareResponse {
            quote: "quote-id".to_string(),
            request: "aa".to_string(),
            amount: Some(Amount::from(1)),
            unit: Some(CurrencyUnit::custom("EHASH")),
            state: cdk_common::nuts::nutXX::QuoteState::Paid,
            expiry: Some(1),
            pubkey,
            keyset_id,
            amount_issued: Amount::ZERO,
        };

        let converted: EhashQuoteResponse<String> = response.into();
        assert_eq!(converted.state, MintQuoteState::Paid);
    }

    #[test]
    fn ehash_request_converts_to_mining_share() {
        let secret = bitcoin::secp256k1::SecretKey::from_slice(&[1u8; 32]).unwrap();
        let secp_pubkey = bitcoin::secp256k1::PublicKey::from_secret_key(&cdk::SECP256K1, &secret);
        let pubkey = PublicKey::from_slice(&secp_pubkey.serialize()).unwrap();
        let header_hash = sha256::Hash::hash(b"ehash").to_string();

        let request = EhashQuoteRequest {
            amount: Amount::from(2),
            unit: CurrencyUnit::custom("EHASH"),
            header_hash: header_hash.clone(),
            description: None,
            pubkey,
        };

        let mining_request = MintQuoteMiningShareRequest::try_from(request).unwrap();
        assert_eq!(mining_request.header_hash.to_string(), header_hash);
    }
}
