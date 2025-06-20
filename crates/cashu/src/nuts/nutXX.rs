//! MiningShare

use std::fmt;
use std::str::FromStr;

use bitcoin_hashes::sha256::Hash;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use thiserror::Error;
#[cfg(feature = "mint")]
use uuid::Uuid;

use super::{BlindSignature, CurrencyUnit, MeltQuoteState, Mpp, PublicKey};
use crate::{Amount, BlindedMessage};

/// NUT023 Error
#[derive(Debug, Error)]
pub enum Error {
    /// Unknown Quote State
    #[error("Unknown Quote State")]
    UnknownState,
    /// Amount overflow
    #[error("Amount overflow")]
    AmountOverflow,
    /// Invalid Amount
    #[error("Invalid Request")]
    InvalidAmountRequest,
}

/// Mint quote request [NUT-04]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct MintQuoteMiningShareRequest {
    /// Amount
    pub amount: Amount,
    /// Unit wallet would like to pay with
    pub unit: CurrencyUnit,
    // TODO better to use a pubkey field?
    pub header_hash: Hash,
    /// Memo to create the invoice with
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// NUT-19 Pubkey
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<PublicKey>,
}

/// Mining share mint request [NUT-XX]
// encapsulate all data needed to create a quote in a single API call
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
#[serde(bound = "Q: Serialize + DeserializeOwned")]
pub struct MintMiningShareRequest<Q> {
    /// Quote id
    #[cfg_attr(feature = "swagger", schema(max_length = 1_000))]
    pub quote: Q,
    /// Outputs
    #[cfg_attr(feature = "swagger", schema(max_items = 1_000))]
    pub outputs: Vec<BlindedMessage>,
    /// Signature
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl<Q> MintMiningShareRequest<Q> {
    /// Total [`Amount`] of outputs
    pub fn total_amount(&self) -> Result<Amount, Error> {
        Amount::try_sum(
            self.outputs
                .iter()
                .map(|BlindedMessage { amount, .. }| *amount),
        )
        .map_err(|_| Error::AmountOverflow)
    }
}

/// Mining share mint response [NUT-XX]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
pub struct MintMiningShareResponse {
    /// Blinded Signatures
    pub signatures: Vec<BlindSignature>,
}

/// Possible states of a quote
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema), schema(as = MintQuoteState))]
pub enum QuoteState {
    /// Quote has not been paid
    #[default]
    Unpaid,
    /// Quote has been paid and wallet can mint
    Paid,
    /// Minting is in progress
    /// **Note:** This state is to be used internally but is not part of the
    /// nut.
    Pending,
    /// ecash issued for quote
    Issued,
}

impl fmt::Display for QuoteState {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Unpaid => write!(f, "UNPAID"),
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
            "PENDING" => Ok(Self::Pending),
            "PAID" => Ok(Self::Paid),
            "UNPAID" => Ok(Self::Unpaid),
            "ISSUED" => Ok(Self::Issued),
            _ => Err(Error::UnknownState),
        }
    }
}

/// Mint quote response [NUT-04]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
#[serde(bound = "Q: Serialize + DeserializeOwned")]
pub struct MintQuoteMiningShareResponse<Q> {
    /// Quote Id
    pub quote: Q,
    /// Payment request to fulfil
    pub request: String,
    /// Amount
    // REVIEW: This is now required in the spec, we should remove the option once all mints update
    pub amount: Option<Amount>,
    /// Unit
    // REVIEW: This is now required in the spec, we should remove the option once all mints update
    pub unit: Option<CurrencyUnit>,
    /// Quote State
    pub state: QuoteState,
    /// Unix timestamp until the quote is valid
    pub expiry: Option<u64>,
    /// NUT-19 Pubkey
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<PublicKey>,
}

impl<Q: ToString> MintQuoteMiningShareResponse<Q> {
    /// Convert the MintQuote with a quote type Q to a String
    pub fn to_string_id(&self) -> MintQuoteMiningShareResponse<String> {
        MintQuoteMiningShareResponse {
            quote: self.quote.to_string(),
            request: self.request.clone(),
            state: self.state,
            expiry: self.expiry,
            pubkey: self.pubkey,
            amount: self.amount,
            unit: self.unit.clone(),
        }
    }
}

#[cfg(feature = "mint")]
impl From<MintQuoteMiningShareResponse<Uuid>> for MintQuoteMiningShareResponse<String> {
    fn from(value: MintQuoteMiningShareResponse<Uuid>) -> Self {
        Self {
            quote: value.quote.to_string(),
            request: value.request,
            state: value.state,
            expiry: value.expiry,
            pubkey: value.pubkey,
            amount: value.amount,
            unit: value.unit.clone(),
        }
    }
}

// use cdk_common::mint::MintQuote;
// #[cfg(feature = "mint")]
// impl From<MintQuote> for MintQuoteMiningShareResponse<Uuid> {
//     fn from(mint_quote: MintQuote) -> MintQuoteMiningShareResponse<Uuid> {
//         MintQuoteMiningShareResponse {
//             quote: mint_quote.id,
//             request: mint_quote.request,
//             state: mint_quote.state,
//             expiry: Some(mint_quote.expiry),
//             pubkey: mint_quote.pubkey,
//         }
//     }
// }

// /// MiningShare melt quote request [NUT-XX]
// #[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
// #[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
// pub struct MeltQuoteMiningShareRequest {
//     /// Bolt11 invoice to be paid
//     #[cfg_attr(feature = "swagger", schema(value_type = String))]
//     pub request: Bolt11Invoice,
//     /// Unit wallet would like to pay with
//     pub unit: CurrencyUnit,
//     // /// Payment Options
//     // pub options: Option<MeltOptions>,
// }

// /// Melt Options
// #[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize)]
// #[serde(untagged)]
// #[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
// pub enum MeltOptions {
//     /// Mpp Options
//     Mpp {
//         /// MPP
//         mpp: Mpp,
//     },
//     /// Amountless options
//     Amountless {
//         /// Amountless
//         amountless: Amountless,
//     },
// }

// impl MeltOptions {
//     /// Create new [`MeltOptions::Mpp`]
//     pub fn new_mpp<A>(amount: A) -> Self
//     where
//         A: Into<Amount>,
//     {
//         Self::Mpp {
//             mpp: Mpp {
//                 amount: amount.into(),
//             },
//         }
//     }

//     /// Create new [`MeltOptions::Amountless`]
//     pub fn new_amountless<A>(amount_msat: A) -> Self
//     where
//         A: Into<Amount>,
//     {
//         Self::Amountless {
//             amountless: Amountless {
//                 amount_msat: amount_msat.into(),
//             },
//         }
//     }

//     /// Payment amount
//     pub fn amount_msat(&self) -> Amount {
//         match self {
//             Self::Mpp { mpp } => mpp.amount,
//             Self::Amountless { amountless } => amountless.amount_msat,
//         }
//     }
// }

// /// Amountless payment
// #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
// #[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
// pub struct Amountless {
//     /// Amount to pay in msat
//     pub amount_msat: Amount,
// }

// impl MeltQuoteMiningShareRequest {
//     /// Amount from [`MeltQuoteMiningShareRequest`]
//     ///
//     /// Amount can either be defined in the bolt11 invoice,
//     /// in the request for an amountless bolt11 or in MPP option.
//     pub fn amount_msat(&self) -> Result<Amount, Error> {
//         let MeltQuoteMiningShareRequest {
//             request,
//             unit: _,
//             // options,
//             ..
//         } = self;

//         // match options {
//         //     None => Ok(request
//         //         .amount_milli_satoshis()
//         //         .ok_or(Error::InvalidAmountRequest)?
//         //         .into()),
//         //     Some(MeltOptions::Mpp { mpp }) => Ok(mpp.amount),
//         //     Some(MeltOptions::Amountless { amountless }) => {
//         //         let amount = amountless.amount_msat;
//         //         if let Some(amount_msat) = request.amount_milli_satoshis() {
//         //             if amount != amount_msat.into() {
//         //                 return Err(Error::InvalidAmountRequest);
//         //             }
//         //         }
//         //         Ok(amount)
//         //     }
//         // }
//     }
// }

/// Melt quote response [NUT-05]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "swagger", derive(utoipa::ToSchema))]
#[serde(bound = "Q: Serialize")]
pub struct MeltQuoteMiningShareResponse<Q> {
    /// Quote Id
    pub quote: Q,
    /// The amount that needs to be provided
    pub amount: Amount,
    /// The fee reserve that is required
    pub fee_reserve: Amount,
    /// Whether the request haas be paid
    // TODO: To be deprecated
    /// Deprecated
    pub paid: Option<bool>,
    /// Quote State
    pub state: MeltQuoteState,
    /// Unix timestamp until the quote is valid
    pub expiry: u64,
    /// Payment preimage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_preimage: Option<String>,
    /// Change
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<Vec<BlindSignature>>,
    /// Payment request to fulfill
    // REVIEW: This is now required in the spec, we should remove the option once all mints update
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// Unit
    // REVIEW: This is now required in the spec, we should remove the option once all mints update
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<CurrencyUnit>,
}

impl<Q: ToString> MeltQuoteMiningShareResponse<Q> {
    /// Convert a `MeltQuoteMiningShareResponse` with type Q (generic/unknown) to a
    /// `MeltQuoteMiningShareResponse` with `String`
    pub fn to_string_id(self) -> MeltQuoteMiningShareResponse<String> {
        MeltQuoteMiningShareResponse {
            quote: self.quote.to_string(),
            amount: self.amount,
            fee_reserve: self.fee_reserve,
            paid: self.paid,
            state: self.state,
            expiry: self.expiry,
            payment_preimage: self.payment_preimage,
            change: self.change,
            request: self.request,
            unit: self.unit,
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
            paid: value.paid,
            state: value.state,
            expiry: value.expiry,
            payment_preimage: value.payment_preimage,
            change: value.change,
            request: value.request,
            unit: value.unit,
        }
    }
}

// A custom deserializer is needed until all mints
// update some will return without the required state.
impl<'de, Q: DeserializeOwned> Deserialize<'de> for MeltQuoteMiningShareResponse<Q> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;

        let quote: Q = serde_json::from_value(
            value
                .get("quote")
                .ok_or(serde::de::Error::missing_field("quote"))?
                .clone(),
        )
        .map_err(|_| serde::de::Error::custom("Invalid quote if string"))?;

        let amount = value
            .get("amount")
            .ok_or(serde::de::Error::missing_field("amount"))?
            .as_u64()
            .ok_or(serde::de::Error::missing_field("amount"))?;
        let amount = Amount::from(amount);

        let fee_reserve = value
            .get("fee_reserve")
            .ok_or(serde::de::Error::missing_field("fee_reserve"))?
            .as_u64()
            .ok_or(serde::de::Error::missing_field("fee_reserve"))?;

        let fee_reserve = Amount::from(fee_reserve);

        let paid: Option<bool> = value.get("paid").and_then(|p| p.as_bool());

        let state: Option<String> = value
            .get("state")
            .and_then(|s| serde_json::from_value(s.clone()).ok());

        let (state, paid) = match (state, paid) {
            (None, None) => return Err(serde::de::Error::custom("State or paid must be defined")),
            (Some(state), _) => {
                let state: MeltQuoteState = MeltQuoteState::from_str(&state)
                    .map_err(|_| serde::de::Error::custom("Unknown state"))?;
                let paid = state == MeltQuoteState::Paid;

                (state, paid)
            }
            (None, Some(paid)) => {
                let state = if paid {
                    MeltQuoteState::Paid
                } else {
                    MeltQuoteState::Unpaid
                };
                (state, paid)
            }
        };

        let expiry = value
            .get("expiry")
            .ok_or(serde::de::Error::missing_field("expiry"))?
            .as_u64()
            .ok_or(serde::de::Error::missing_field("expiry"))?;

        let payment_preimage: Option<String> = value
            .get("payment_preimage")
            .and_then(|p| serde_json::from_value(p.clone()).ok());

        let change: Option<Vec<BlindSignature>> = value
            .get("change")
            .and_then(|b| serde_json::from_value(b.clone()).ok());

        let request: Option<String> = value
            .get("request")
            .and_then(|r| serde_json::from_value(r.clone()).ok());

        let unit: Option<CurrencyUnit> = value
            .get("unit")
            .and_then(|u| serde_json::from_value(u.clone()).ok());

        Ok(Self {
            quote,
            amount,
            fee_reserve,
            paid: Some(paid),
            state,
            expiry,
            payment_preimage,
            change,
            request,
            unit,
        })
    }
}
