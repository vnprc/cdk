//! Hashpool quote lookup functionality

use serde::{Deserialize, Serialize};

use crate::nuts::{PaymentMethod, PublicKey};

/// Quote lookup request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostMintQuoteLookupRequest {
    /// Public keys to lookup quotes for
    pub pubkeys: Vec<PublicKey>,
}

/// Quote lookup response
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostMintQuoteLookupResponse {
    /// Matching quotes
    pub quotes: Vec<MintQuoteLookupItem>,
}

/// Individual mint quote lookup item
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MintQuoteLookupItem {
    /// Public key associated with this quote
    pub pubkey: PublicKey,
    /// Quote ID
    pub quote: String,
    /// Payment method
    pub method: PaymentMethod,
    /// Quote amount
    pub amount: crate::Amount,
    /// Keyset ID for this quote (from blinded messages for mining shares)
    pub keyset_id: crate::nuts::Id,
}
