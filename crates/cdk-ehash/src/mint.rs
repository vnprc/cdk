//! Mint-side helpers for ehash.

use cdk::error::Error;
use cdk::mint::{Mint, MintQuoteRequest, MintQuoteResponse};
use cdk::nuts::CurrencyUnit;
use cdk_common::quote_id::QuoteId;

use crate::types::{EhashError, EhashQuoteRequest, EhashQuoteResponse};

/// Create a new ehash quote using the custom payment method flow.
pub async fn create_ehash_quote(
    mint: &Mint,
    request: EhashQuoteRequest,
) -> Result<EhashQuoteResponse<QuoteId>, Error> {
    if request.unit != CurrencyUnit::custom("EHASH") {
        return Err(Error::UnsupportedUnit);
    }

    let custom_request = request
        .try_into()
        .map_err(|_err: EhashError| Error::InvalidPaymentRequest)?;

    let response = mint
        .get_mint_quote(MintQuoteRequest::Custom {
            method: "ehash".to_string(),
            request: custom_request,
        })
        .await?;

    match response {
        MintQuoteResponse::Custom { response, .. } => Ok(response.into()),
        _ => Err(Error::InvalidPaymentMethod),
    }
}

/// Fetch an ehash quote by ID if it exists.
pub async fn get_ehash_quote(
    mint: &Mint,
    quote_id: &QuoteId,
) -> Result<Option<EhashQuoteResponse<QuoteId>>, Error> {
    let response = mint.check_mint_quote(quote_id).await?;

    match response {
        MintQuoteResponse::Custom { method, response } if method == "ehash" => {
            Ok(Some(response.into()))
        }
        _ => Ok(None),
    }
}
