//! Mint-side helpers for ehash.

use cdk::error::Error;
use cdk::mint::Mint;
use cdk::nuts::CurrencyUnit;
use cdk_common::nuts::nutXX::MintQuoteMiningShareResponse;
use cdk_common::quote_id::QuoteId;

use crate::types::{EhashError, EhashQuoteRequest, EhashQuoteResponse};

/// Create a new ehash quote using the mining-share mint flow.
pub async fn create_ehash_quote(
    mint: &Mint,
    request: EhashQuoteRequest,
) -> Result<EhashQuoteResponse<QuoteId>, Error> {
    if request.unit != CurrencyUnit::custom("EHASH") {
        return Err(Error::UnsupportedUnit);
    }

    let mining_request = request
        .try_into()
        .map_err(|_err: EhashError| Error::InvalidPaymentRequest)?;

    let quote = mint.create_mint_mining_share_quote(mining_request).await?;
    let mining_response: MintQuoteMiningShareResponse<QuoteId> = quote.try_into()?;

    Ok(mining_response.into())
}

/// Fetch an ehash quote by ID if it exists.
pub async fn get_ehash_quote(
    mint: &Mint,
    quote_id: &QuoteId,
) -> Result<Option<EhashQuoteResponse<QuoteId>>, Error> {
    let maybe_quote = mint.get_mint_mining_share_quote(quote_id).await?;
    Ok(maybe_quote.map(Into::into))
}
