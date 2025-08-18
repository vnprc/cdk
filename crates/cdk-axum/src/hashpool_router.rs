//! Hashpool router for quote lookup endpoints

use axum::extract::{Json, State};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use tracing::instrument;

use cdk::hashpool::{PostMintQuoteLookupRequest, PostMintQuoteLookupResponse};
#[cfg(feature = "auth")]
use cdk::nuts::nut21::{Method, ProtectedEndpoint, RoutePath};

#[cfg(feature = "auth")]
use crate::auth::AuthHeader;
use crate::router_handlers::into_response;
use crate::MintState;

#[cfg_attr(feature = "swagger", utoipa::path(
    post,
    context_path = "/v1",
    path = "/mint/quote/lookup",
    request_body(content = PostMintQuoteLookupRequest, description = "Lookup request", content_type = "application/json"),
    responses(
        (status = 200, description = "Successful response", body = PostMintQuoteLookupResponse, content_type = "application/json"),
        (status = 500, description = "Server error", body = cdk::error::ErrorResponse, content_type = "application/json")
    )
))]
/// Lookup mint quotes by NUT-20 locking pubkeys
///
/// Retrieve mint quote information by providing the public key(s) used to lock the mint quotes.
#[instrument(skip_all, fields(pubkey_count = ?payload.pubkeys.len()))]
pub async fn post_mint_quote_lookup(
    #[cfg(feature = "auth")] auth: AuthHeader,
    State(state): State<MintState>,
    Json(payload): Json<PostMintQuoteLookupRequest>,
) -> Result<Json<PostMintQuoteLookupResponse>, Response> {
    #[cfg(feature = "auth")]
    state
        .mint
        .verify_auth(
            auth.into(),
            &ProtectedEndpoint::new(Method::Post, RoutePath::MintQuoteLookup),
        )
        .await
        .map_err(into_response)?;

    let quotes = state
        .mint
        .lookup_mint_quotes_by_pubkeys(&payload.pubkeys, payload.state_filter)
        .await
        .map_err(into_response)?;

    Ok(Json(PostMintQuoteLookupResponse { quotes }))
}

/// Create hashpool router with quote lookup endpoints
pub fn create_hashpool_router(state: MintState) -> Router<MintState> {
    Router::new()
        .route("/mint/quote/lookup", post(post_mint_quote_lookup))
        .with_state(state)
}
