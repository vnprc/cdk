//! Axum routes for ehash endpoints.

use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use cdk::error::{ErrorCode, ErrorResponse};
use cdk::mint::Mint;
use cdk::nuts::{MintRequest, MintResponse, PaymentMethod};
use cdk_common::mint::BatchMintRequest;
use cdk_common::quote_id::QuoteId;
use tracing::instrument;

use crate::mint::{create_ehash_quote, get_ehash_quote};
use crate::types::{EhashQuoteRequest, EhashQuoteResponse};

#[derive(Clone)]
struct EhashState {
    mint: Arc<Mint>,
}

/// Create a router that exposes ehash endpoints.
pub fn create_ehash_router(mint: Arc<Mint>) -> Router {
    let state = EhashState { mint };

    Router::new()
        .route("/v1/mint/quote/ehash", post(post_mint_ehash_quote))
        .route("/v1/mint/quote/ehash/{quote_id}", get(get_mint_ehash_quote))
        .route("/v1/mint/ehash", post(post_mint_ehash))
        .route("/v1/mint/ehash/batch", post(post_mint_ehash_batch))
        .with_state(state)
}

#[instrument(skip_all, fields(amount = %payload.amount, unit = %payload.unit))]
async fn post_mint_ehash_quote(
    State(state): State<EhashState>,
    Json(payload): Json<EhashQuoteRequest>,
) -> Result<Json<EhashQuoteResponse<String>>, Response> {
    let response = create_ehash_quote(&state.mint, payload)
        .await
        .map_err(into_response)?;

    Ok(Json(response.to_string_id()))
}

#[instrument(skip_all, fields(quote_id = %quote_id))]
async fn get_mint_ehash_quote(
    State(state): State<EhashState>,
    Path(quote_id): Path<String>,
) -> Result<Json<EhashQuoteResponse<String>>, Response> {
    let quote_id = QuoteId::from_str(&quote_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid quote ID").into_response())?;

    let maybe_quote = get_ehash_quote(&state.mint, &quote_id)
        .await
        .map_err(into_response)?;

    let Some(quote) = maybe_quote else {
        let error = ErrorResponse::new(
            ErrorCode::Unknown(4040),
            format!("Ehash quote {quote_id} not found"),
        );
        return Err((StatusCode::NOT_FOUND, Json(error)).into_response());
    };

    Ok(Json(quote.to_string_id()))
}

#[instrument(skip_all, fields(quote_id = %payload.quote))]
async fn post_mint_ehash(
    State(state): State<EhashState>,
    Json(payload): Json<MintRequest<String>>,
) -> Result<Json<MintResponse>, Response> {
    let MintRequest {
        quote,
        outputs,
        signature,
    } = payload;

    let quote = QuoteId::from_str(&quote)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid quote ID").into_response())?;

    let request = MintRequest {
        quote,
        outputs,
        signature,
    };

    let response = state
        .mint
        .process_mint_request(request)
        .await
        .map_err(into_response)?;

    Ok(Json(response))
}

#[instrument(skip_all, fields(quote_count = ?payload.quote.len()))]
async fn post_mint_ehash_batch(
    State(state): State<EhashState>,
    Json(payload): Json<BatchMintRequest>,
) -> Result<Json<MintResponse>, Response> {
    let response = state
        .mint
        .process_batch_mint_request(payload, PaymentMethod::MiningShare)
        .await
        .map_err(into_response)?;

    Ok(Json(response))
}

fn into_response<T>(error: T) -> Response
where
    T: Into<ErrorResponse>,
{
    let err_response: ErrorResponse = error.into();
    let status_code = match err_response.code {
        ErrorCode::TokenAlreadySpent
        | ErrorCode::TokenPending
        | ErrorCode::QuoteNotPaid
        | ErrorCode::QuoteExpired
        | ErrorCode::QuotePending
        | ErrorCode::KeysetNotFound
        | ErrorCode::KeysetInactive
        | ErrorCode::BlindedMessageAlreadySigned
        | ErrorCode::UnsupportedUnit
        | ErrorCode::TokensAlreadyIssued
        | ErrorCode::MintingDisabled
        | ErrorCode::InvoiceAlreadyPaid
        | ErrorCode::TokenNotVerified
        | ErrorCode::TransactionUnbalanced
        | ErrorCode::AmountOutofLimitRange
        | ErrorCode::WitnessMissingOrInvalid
        | ErrorCode::DuplicateSignature
        | ErrorCode::DuplicateInputs
        | ErrorCode::DuplicateOutputs
        | ErrorCode::MultipleUnits
        | ErrorCode::UnitMismatch
        | ErrorCode::ClearAuthRequired
        | ErrorCode::BlindAuthRequired => StatusCode::BAD_REQUEST,

        ErrorCode::ClearAuthFailed | ErrorCode::BlindAuthFailed => StatusCode::UNAUTHORIZED,

        ErrorCode::LightningError | ErrorCode::Unknown(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };

    (status_code, Json(err_response)).into_response()
}
