//! Types for `cashu-crab`

use serde::{Deserialize, Serialize};

use crate::nuts::{CurrencyUnit, Id, Proofs};
use crate::{Amount, Bolt11Invoice};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofsStatus {
    pub spendable: Proofs,
    pub spent: Proofs,
}

/// Melt response with proofs
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Melted {
    pub paid: bool,
    pub preimage: Option<String>,
    pub change: Option<Proofs>,
}

/// Possible states of an invoice
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub enum InvoiceStatus {
    Unpaid,
    Paid,
    Expired,
    InFlight,
}

/// Mint Quote Info
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct MintQuote {
    pub id: String,
    pub amount: Amount,
    pub unit: CurrencyUnit,
    pub request: Bolt11Invoice,
    pub paid: bool,
    pub expiry: u64,
}

/// Melt Quote Info
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct MeltQuote {
    pub id: String,
    pub amount: Amount,
    pub request: Bolt11Invoice,
    pub unit: CurrencyUnit,
    pub fee_reserve: Amount,
    pub paid: bool,
    pub expiry: u64,
}

/// Keyset id
#[derive(Debug, Hash, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeysetInfo {
    pub id: Id,
    pub valid_from: u64,
    pub valid_to: Option<u64>,
    pub derivation_path: String,
    pub max_order: u8,
}
