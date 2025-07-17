//! Hashpool quote lookup functionality

use serde::{Deserialize, Serialize};

use crate::nuts::{MintQuoteState, PaymentMethod, PublicKey};

/// Quote state filter for lookup operations
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MintQuoteStateFilter {
    /// Return all quotes regardless of state
    All,
    /// Return only quotes that can be minted (PAID state)
    OnlyPaid,
    /// Return only unpaid quotes
    OnlyUnpaid,
    /// Return only already issued quotes
    OnlyIssued,
    /// Return only quotes in a specific state
    Specific(MintQuoteState),
}

impl Default for MintQuoteStateFilter {
    fn default() -> Self {
        Self::OnlyPaid // Sensible default for most use cases
    }
}

/// Quote lookup request
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostMintQuoteLookupRequest {
    /// Public keys to lookup quotes for
    pub pubkeys: Vec<PublicKey>,
    /// State filter - explicitly specifies which quotes to return
    #[serde(default)]
    pub state_filter: MintQuoteStateFilter,
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
