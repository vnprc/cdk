use bitcoin::hashes::sha256;
use cdk::nuts::nut04::{MintQuoteCustomRequest, MintQuoteCustomResponse};
use cdk::nuts::{CurrencyUnit, Id, MintQuoteState, PublicKey};
use cdk::Amount;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    pub pubkey: Option<PublicKey>,
    /// Keyset ID for this quote (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyset_id: Option<Id>,
    /// Amount issued for this quote (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount_issued: Option<Amount>,
    /// Optional share height metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_height: Option<u64>,
    /// Optional pool identifier metadata.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_id: Option<String>,
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
            share_height: self.share_height,
            pool_id: self.pool_id.clone(),
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

fn read_extra<T: DeserializeOwned>(extra: &Value, key: &str) -> Option<T> {
    extra
        .get(key)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

impl TryFrom<EhashQuoteRequest> for MintQuoteCustomRequest {
    type Error = EhashError;

    fn try_from(value: EhashQuoteRequest) -> Result<Self, Self::Error> {
        let header_hash = parse_header_hash(&value.header_hash)?;

        Ok(MintQuoteCustomRequest {
            amount: value.amount,
            unit: value.unit,
            description: value.description,
            pubkey: Some(value.pubkey),
            extra: serde_json::json!({
                "header_hash": header_hash.to_string(),
            }),
        })
    }
}

impl<Q> From<MintQuoteCustomResponse<Q>> for EhashQuoteResponse<Q> {
    fn from(value: MintQuoteCustomResponse<Q>) -> Self {
        let extra = value.extra.clone();

        Self {
            quote: value.quote,
            request: value.request,
            amount: value.amount,
            unit: value.unit,
            state: value.state,
            expiry: value.expiry,
            pubkey: value.pubkey,
            keyset_id: read_extra(&extra, "keyset_id"),
            amount_issued: read_extra(&extra, "amount_issued"),
            share_height: read_extra(&extra, "share_height"),
            pool_id: read_extra(&extra, "pool_id"),
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
    fn custom_quote_conversion_sets_state() {
        let secret = bitcoin::secp256k1::SecretKey::from_slice(&[1u8; 32]).unwrap();
        let secp_pubkey = bitcoin::secp256k1::PublicKey::from_secret_key(&cdk::SECP256K1, &secret);
        let pubkey = PublicKey::from_slice(&secp_pubkey.serialize()).unwrap();
        let keyset_id = Id::from_bytes(&[0u8; 8]).unwrap();

        let response = MintQuoteCustomResponse {
            quote: "quote-id".to_string(),
            request: "aa".to_string(),
            amount: Some(Amount::from(1)),
            unit: Some(CurrencyUnit::custom("EHASH")),
            state: MintQuoteState::Paid,
            expiry: Some(1),
            pubkey: Some(pubkey),
            extra: serde_json::json!({
                "keyset_id": keyset_id,
                "amount_issued": Amount::ZERO,
                "share_height": 100u64,
                "pool_id": "pool-1",
            }),
        };

        let converted: EhashQuoteResponse<String> = response.into();
        assert_eq!(converted.state, MintQuoteState::Paid);
        assert_eq!(converted.keyset_id, Some(keyset_id));
        assert_eq!(converted.amount_issued, Some(Amount::ZERO));
        assert_eq!(converted.share_height, Some(100));
        assert_eq!(converted.pool_id, Some("pool-1".to_string()));
    }

    #[test]
    fn ehash_request_converts_to_custom() {
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

        let custom_request = MintQuoteCustomRequest::try_from(request).unwrap();
        assert_eq!(
            custom_request.extra.get("header_hash").unwrap().as_str().unwrap(),
            header_hash
        );
    }
}
