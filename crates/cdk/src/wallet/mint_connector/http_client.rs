//! HTTP Mint client with pluggable transport
use std::collections::HashSet;
use std::sync::{Arc, RwLock as StdRwLock};

use async_trait::async_trait;
use cdk_common::auth::oidc::{OidcHttpResponse, OidcHttpTransport};
use cdk_common::nutxx::{MintQuoteByPubkeyRequest, MintQuoteByPubkeyResponse};
use cdk_common::{
    nut19, MeltQuoteCreateResponse, MeltQuoteRequest, MeltQuoteResponse, Method,
    MintQuoteBolt11Response, MintQuoteBolt12Response, MintQuoteCustomResponse,
    MintQuoteOnchainResponse, MintQuoteRequest, MintQuoteResponse, ProtectedEndpoint, RoutePath,
};
use cdk_http_client::HttpError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::instrument;
use url::Url;
use web_time::{Duration, Instant};

use super::transport::Transport;
use super::{Error, MintConnector};
use crate::error::ErrorResponse;
use crate::mint_url::MintUrl;
use crate::nuts::nut00::{KnownMethod, PaymentMethod};
use crate::nuts::nut22::MintAuthRequest;
use crate::nuts::{
    AuthToken, BatchCheckMintQuoteRequest, BatchMintRequest, CheckStateRequest, CheckStateResponse,
    Id, KeySet, KeysResponse, KeysetResponse, MeltOnchainRequest, MeltRequest, MintInfo,
    MintRequest, MintResponse, RestoreRequest, RestoreResponse, SwapRequest, SwapResponse,
};
use crate::wallet::auth::{AuthMintConnector, AuthWallet};
use crate::OidcClient;

type Cache = (u64, HashSet<(nut19::Method, nut19::Path)>);

/// Delay before the first retry of a failed request.
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Upper bound for the exponential backoff between retries.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(8);

fn payment_method_path_segment(method: &PaymentMethod) -> Result<&str, Error> {
    match method {
        PaymentMethod::Known(known) => Ok(known.as_str()),
        PaymentMethod::Custom(method) if PaymentMethod::is_valid_custom_method_name(method) => {
            Ok(method)
        }
        PaymentMethod::Custom(_) => Err(Error::InvalidPaymentMethod),
    }
}

fn fill_response_method(value: &mut serde_json::Value, method: &PaymentMethod) {
    if let serde_json::Value::Object(object) = value {
        object
            .entry("method".to_string())
            .or_insert_with(|| serde_json::Value::String(method.to_string()));
    }
}

fn fill_response_methods(value: &mut serde_json::Value, method: &PaymentMethod) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                fill_response_method(item, method);
            }
        }
        _ => fill_response_method(value, method),
    }
}

fn deserialize_with_route_method<R>(
    mut value: serde_json::Value,
    method: &PaymentMethod,
) -> Result<R, Error>
where
    R: DeserializeOwned,
{
    fill_response_methods(&mut value, method);
    serde_json::from_value(value).map_err(|e| Error::Custom(e.to_string()))
}

fn deserialize_quote_value<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, Error> {
    Ok(serde_json::from_value(value)?)
}

/// Reconstruct a [`MintQuoteResponse<String>`] from one element of a
/// [`MintQuoteByPubkeyResponse`] `quotes` array.
///
/// The mint flattens each quote to its bare NUT-04 response object rather than
/// `MintQuoteResponse`'s own externally tagged form (see `mint_quote_response_to_value` in
/// `cdk-axum`), so the enum's derived `Deserialize` cannot parse it directly — it expects a
/// single-key envelope like `{"Bolt11": {...}}`, not a flat object. Every concrete response type
/// embeds its own `method` field, so peeking at it here is enough to pick the right variant to
/// deserialize into: the exact inverse of the mint-side flattening.
///
/// A missing or non-string `method` is rejected as a protocol error rather than guessed at.
/// This is deliberately stricter than each concrete type's own serde default for `method`:
/// `MintQuoteBolt11Response` defaults a missing `method` to bolt11, and
/// `MintQuoteBolt12Response`/`MintQuoteOnchainResponse` likewise default to their own method —
/// but those defaults only kick in once *this* function has already committed to deserializing
/// into that specific type, and `MintQuoteCustomResponse::method` has no default at all. Falling
/// back to Bolt11 here, before that choice is made, would silently reinterpret a paid
/// bolt12/onchain/custom quote's accounting fields as an unpaid Bolt11 response.
fn mint_quote_value_to_response(
    value: serde_json::Value,
) -> Result<MintQuoteResponse<String>, Error> {
    let method_value = value.get("method").cloned().ok_or_else(|| {
        Error::Custom(format!(
            "mint quote {} response is missing a \"method\" field",
            value
                .get("quote")
                .and_then(|q| q.as_str())
                .unwrap_or("<unknown>")
        ))
    })?;
    let method: PaymentMethod = serde_json::from_value(method_value)?;

    match method {
        PaymentMethod::Known(KnownMethod::Bolt11) => {
            Ok(MintQuoteResponse::Bolt11(deserialize_quote_value(value)?))
        }
        PaymentMethod::Known(KnownMethod::Bolt12) => {
            Ok(MintQuoteResponse::Bolt12(deserialize_quote_value(value)?))
        }
        PaymentMethod::Known(KnownMethod::Onchain) => {
            Ok(MintQuoteResponse::Onchain(deserialize_quote_value(value)?))
        }
        PaymentMethod::Custom(name) => Ok(MintQuoteResponse::Custom {
            method: PaymentMethod::Custom(name),
            response: deserialize_quote_value(value)?,
        }),
    }
}

/// Http Client
#[derive(Debug, Clone)]
pub struct HttpClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    transport: Arc<T>,
    mint_url: MintUrl,
    cache_support: Arc<StdRwLock<Cache>>,
    auth_wallet: Arc<RwLock<Option<AuthWallet>>>,
}

#[derive(Debug, Clone)]
struct OidcTransportClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    transport: Arc<T>,
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<T> OidcHttpTransport for OidcTransportClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    async fn get(&self, url: &str) -> Result<OidcHttpResponse, HttpError> {
        let url = Url::parse(url).map_err(|e| HttpError::Other(e.to_string()))?;
        let response = self.transport.http_get_raw(url, None).await?;
        let status = response.status();
        let body = response.bytes().await?;
        Ok(OidcHttpResponse::new(status, body))
    }

    async fn post_form(
        &self,
        url: &str,
        params: Vec<(String, String)>,
    ) -> Result<OidcHttpResponse, HttpError> {
        let url = Url::parse(url).map_err(|e| HttpError::Other(e.to_string()))?;
        let response = self
            .transport
            .http_post_form_raw(url, None, &params)
            .await?;
        let status = response.status();
        let body = response.bytes().await?;
        Ok(OidcHttpResponse::new(status, body))
    }
}

impl<T> HttpClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    fn map_http_error(err: HttpError) -> Error {
        match err {
            HttpError::Status { status, message } => {
                match serde_json::from_str::<ErrorResponse>(&message) {
                    Ok(err_response) => err_response.into(),
                    Err(_) => Error::HttpError(Some(status), message),
                }
            }
            HttpError::Timeout => Error::Timeout,
            HttpError::Connection(message)
            | HttpError::Serialization(message)
            | HttpError::Proxy(message)
            | HttpError::Build(message)
            | HttpError::Other(message) => Error::HttpError(None, message),
        }
    }

    async fn transport_http_get<R>(&self, url: Url, auth: Option<AuthToken>) -> Result<R, Error>
    where
        R: DeserializeOwned,
    {
        self.transport
            .http_get(url, auth)
            .await
            .map_err(Self::map_http_error)
    }

    async fn transport_http_post<P, R>(
        &self,
        url: Url,
        auth: Option<AuthToken>,
        payload: &P,
    ) -> Result<R, Error>
    where
        P: Serialize + Send + Sync,
        R: DeserializeOwned,
    {
        self.transport
            .http_post(url, auth, payload)
            .await
            .map_err(Self::map_http_error)
    }

    /// Create new [`HttpClient`] with a provided transport implementation.
    pub fn with_transport(
        mint_url: MintUrl,
        transport: T,
        auth_wallet: Option<AuthWallet>,
    ) -> Self {
        Self {
            transport: transport.into(),
            mint_url,
            auth_wallet: Arc::new(RwLock::new(auth_wallet)),
            cache_support: Default::default(),
        }
    }

    /// Create new [`HttpClient`]
    pub fn new(mint_url: MintUrl, auth_wallet: Option<AuthWallet>) -> Self
    where
        T: Default,
    {
        Self {
            transport: T::default().into(),
            mint_url,
            auth_wallet: Arc::new(RwLock::new(auth_wallet)),
            cache_support: Default::default(),
        }
    }

    /// Get auth token for a protected endpoint
    #[instrument(skip(self))]
    pub async fn get_auth_token(
        &self,
        method: Method,
        path: RoutePath,
    ) -> Result<Option<AuthToken>, Error> {
        let auth_wallet = self.auth_wallet.read().await;
        match auth_wallet.as_ref() {
            Some(auth_wallet) => {
                let endpoint = ProtectedEndpoint::new(method, path);
                auth_wallet.get_auth_for_request(&endpoint).await
            }
            None => Ok(None),
        }
    }

    /// Create new [`HttpClient`] with a proxy for specific TLDs.
    /// Specifying `None` for `host_matcher` will use the proxy for all
    /// requests.
    pub fn with_proxy(
        mint_url: MintUrl,
        proxy: Url,
        host_matcher: Option<&str>,
        accept_invalid_certs: bool,
    ) -> Result<Self, Error>
    where
        T: Default,
    {
        let mut transport = T::default();
        transport
            .with_proxy(proxy, host_matcher, accept_invalid_certs)
            .map_err(Self::map_http_error)?;

        Ok(Self {
            transport: transport.into(),
            mint_url,
            auth_wallet: Arc::new(RwLock::new(None)),
            cache_support: Default::default(),
        })
    }

    /// Generic implementation of a retriable http request
    ///
    /// The retry only happens if the mint supports replay through the Caching of NUT-19.
    #[inline(always)]
    async fn retriable_http_request<P, R>(
        &self,
        method: nut19::Method,
        path: nut19::Path,
        auth_token: Option<AuthToken>,
        payload: &P,
    ) -> Result<R, Error>
    where
        P: Serialize + Send + Sync,
        R: DeserializeOwned,
    {
        let started = Instant::now();

        let retriable_window = self
            .cache_support
            .read()
            .map(|cache_support| {
                cache_support
                    .1
                    .get(&(method, path.clone()))
                    .map(|_| cache_support.0)
            })
            .unwrap_or_default()
            .map(Duration::from_secs)
            .unwrap_or_default();

        let transport = self.transport.clone();
        let mut retry_delay = INITIAL_RETRY_DELAY;
        loop {
            let url = match &path {
                nut19::Path::Swap => self.mint_url.join_paths(&["v1", "swap"])?,
                nut19::Path::Custom(custom_path) => {
                    // Custom paths should be in the format "/v1/mint/{method}" or "/v1/melt/{method}"
                    // Remove leading slash if present
                    let path_str = custom_path.trim_start_matches('/');
                    let parts: Vec<&str> = path_str.split('/').collect();
                    self.mint_url.join_paths(&parts)?
                }
            };

            let result = match method {
                nut19::Method::Get => transport
                    .http_get(url, auth_token.clone())
                    .await
                    .map_err(Self::map_http_error),
                nut19::Method::Post => transport
                    .http_post(url, auth_token.clone(), payload)
                    .await
                    .map_err(Self::map_http_error),
            };

            if result.is_ok() {
                return result;
            }

            match result.as_ref() {
                Err(Error::HttpError(status_code, _)) => {
                    let status_code = status_code.to_owned().unwrap_or_default();
                    if (400..=499).contains(&status_code) {
                        // 4xx errors won't be 'solved' by retrying
                        return result;
                    }

                    // retry request, if possible
                    tracing::error!("Failed http_request {:?}", result.as_ref().err());

                    let remaining = retriable_window.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        return result;
                    }

                    // Back off before retrying so a struggling mint is not hammered,
                    // without ever sleeping past the end of the replay window.
                    sleep(retry_delay.min(remaining)).await;
                    retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
                }
                Err(_) => return result,
                _ => unreachable!(),
            };
        }
    }
}

fn parse_lnurl_callback_url(url: &str) -> Result<Url, Error> {
    let parsed_url = Url::parse(url).map_err(|e| Error::Custom(format!("Invalid URL: {}", e)))?;

    if parsed_url.scheme() != "https" {
        return Err(Error::Custom(
            "LNURL callback URL must use HTTPS".to_string(),
        ));
    }

    if !parsed_url.username().is_empty() || parsed_url.password().is_some() {
        return Err(Error::Custom(
            "LNURL callback URL must not include credentials".to_string(),
        ));
    }

    if parsed_url.fragment().is_some() {
        return Err(Error::Custom(
            "LNURL callback URL must not include a fragment".to_string(),
        ));
    }

    match parsed_url.host() {
        Some(url::Host::Domain(host)) if host != "localhost" => Ok(parsed_url),
        Some(_) => Err(Error::Custom(
            "LNURL callback URL must use a public DNS host".to_string(),
        )),
        None => Err(Error::Custom(
            "LNURL callback URL must include a host".to_string(),
        )),
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<T> MintConnector for HttpClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    fn auth_connector(
        &self,
        mint_url: MintUrl,
        cat: Option<AuthToken>,
    ) -> Arc<dyn AuthMintConnector + Send + Sync> {
        Arc::new(AuthHttpClient::with_transport(
            mint_url,
            self.transport.as_ref().clone(),
            cat,
        ))
    }

    fn oidc_client(&self, openid_discovery: String, client_id: Option<String>) -> OidcClient {
        OidcClient::with_transport(
            openid_discovery,
            client_id,
            Arc::new(OidcTransportClient {
                transport: self.transport.clone(),
            }),
        )
    }

    async fn connect_websocket(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<
        (
            cdk_common::ws_client::WsSender,
            cdk_common::ws_client::WsReceiver,
        ),
        cdk_common::ws_client::WsError,
    > {
        self.transport.ws_connect(url, headers).await
    }

    #[cfg(all(feature = "bip353", not(target_arch = "wasm32")))]
    #[instrument(skip(self), fields(mint_url = %self.mint_url))]
    async fn resolve_dns_txt(&self, domain: &str) -> Result<Vec<String>, Error> {
        self.transport
            .as_ref()
            .resolve_dns_txt(domain)
            .await
            .map_err(Self::map_http_error)
    }

    /// Fetch Lightning address pay request data
    #[instrument(skip(self))]
    async fn fetch_lnurl_pay_request(
        &self,
        url: &str,
    ) -> Result<crate::lightning_address::LnurlPayResponse, Error> {
        let parsed_url =
            url::Url::parse(url).map_err(|e| Error::Custom(format!("Invalid URL: {}", e)))?;
        self.transport_http_get(parsed_url, None).await
    }

    /// Fetch invoice from Lightning address callback
    #[instrument(skip(self))]
    async fn fetch_lnurl_invoice(
        &self,
        url: &str,
    ) -> Result<crate::lightning_address::LnurlPayInvoiceResponse, Error> {
        let parsed_url = parse_lnurl_callback_url(url)?;
        self.transport_http_get(parsed_url, None).await
    }

    /// Get Active Mint Keys [NUT-01]
    #[instrument(skip(self), fields(mint_url = %self.mint_url))]
    async fn get_mint_keys(&self) -> Result<Vec<KeySet>, Error> {
        let url = self.mint_url.join_paths(&["v1", "keys"])?;

        Ok(self
            .transport_http_get::<KeysResponse>(url, None)
            .await?
            .keysets)
    }

    /// Get Keyset Keys [NUT-01]
    #[instrument(skip(self), fields(mint_url = %self.mint_url))]
    async fn get_mint_keyset(&self, keyset_id: Id) -> Result<KeySet, Error> {
        let url = self
            .mint_url
            .join_paths(&["v1", "keys", &keyset_id.to_string()])?;

        let keys_response = self.transport_http_get::<KeysResponse>(url, None).await?;

        Ok(keys_response
            .keysets
            .first()
            .ok_or(Error::UnknownKeySet)?
            .clone())
    }

    /// Get Keysets [NUT-02]
    #[instrument(skip(self), fields(mint_url = %self.mint_url))]
    async fn get_mint_keysets(&self) -> Result<KeysetResponse, Error> {
        let url = self.mint_url.join_paths(&["v1", "keysets"])?;
        self.transport_http_get(url, None).await
    }

    /// Mint Quote [NUT-04, NUT-23, NUT-25]
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_mint_quote(
        &self,
        request: MintQuoteRequest,
    ) -> Result<MintQuoteResponse<String>, Error> {
        let method = request.method();
        let method_name = payment_method_path_segment(&method)?;

        let url = self
            .mint_url
            .join_paths(&["v1", "mint", "quote", method_name])?;

        let auth_token = self
            .get_auth_token(Method::Post, RoutePath::MintQuote(method.to_string()))
            .await?;

        match &request {
            MintQuoteRequest::Bolt11(req) => {
                let response: cdk_common::nut23::MintQuoteBolt11Response<String> =
                    self.transport_http_post(url, auth_token, req).await?;
                Ok(MintQuoteResponse::Bolt11(response))
            }
            MintQuoteRequest::Bolt12(req) => {
                let response: cdk_common::nut25::MintQuoteBolt12Response<String> =
                    self.transport_http_post(url, auth_token, req).await?;
                Ok(MintQuoteResponse::Bolt12(response))
            }
            MintQuoteRequest::Onchain(req) => {
                let response: cdk_common::nut30::MintQuoteOnchainResponse<String> =
                    self.transport_http_post(url, auth_token, req).await?;
                Ok(MintQuoteResponse::Onchain(response))
            }
            MintQuoteRequest::Custom { request: req, .. } => {
                let value: serde_json::Value =
                    self.transport_http_post(url, auth_token, req).await?;
                let response: cdk_common::nut04::MintQuoteCustomResponse<String> =
                    deserialize_with_route_method(value, &method)?;
                Ok(MintQuoteResponse::Custom { method, response })
            }
        }
    }

    /// Mint Quote status with payment method
    #[instrument(skip(self), fields(mint_url = %self.mint_url))]
    async fn get_mint_quote_status(
        &self,
        method: PaymentMethod,
        quote_id: &str,
    ) -> Result<MintQuoteResponse<String>, Error> {
        match &method {
            PaymentMethod::Known(KnownMethod::Bolt11) => {
                let url = self
                    .mint_url
                    .join_paths(&["v1", "mint", "quote", "bolt11", quote_id])?;

                let auth_token = self
                    .get_auth_token(
                        Method::Get,
                        RoutePath::MintQuote(PaymentMethod::Known(KnownMethod::Bolt11).to_string()),
                    )
                    .await?;

                let response: MintQuoteBolt11Response<String> =
                    self.transport_http_get(url, auth_token).await?;

                Ok(MintQuoteResponse::Bolt11(response))
            }
            PaymentMethod::Known(KnownMethod::Bolt12) => {
                let url = self
                    .mint_url
                    .join_paths(&["v1", "mint", "quote", "bolt12", quote_id])?;

                let auth_token = self
                    .get_auth_token(
                        Method::Get,
                        RoutePath::MintQuote(PaymentMethod::Known(KnownMethod::Bolt12).to_string()),
                    )
                    .await?;

                let response: MintQuoteBolt12Response<String> =
                    self.transport_http_get(url, auth_token).await?;

                Ok(MintQuoteResponse::Bolt12(response))
            }
            PaymentMethod::Known(KnownMethod::Onchain) => {
                let url = self
                    .mint_url
                    .join_paths(&["v1", "mint", "quote", "onchain", quote_id])?;

                let auth_token = self
                    .get_auth_token(
                        Method::Get,
                        RoutePath::MintQuote(
                            PaymentMethod::Known(KnownMethod::Onchain).to_string(),
                        ),
                    )
                    .await?;

                let response: MintQuoteOnchainResponse<String> =
                    self.transport_http_get(url, auth_token).await?;

                Ok(MintQuoteResponse::Onchain(response))
            }
            PaymentMethod::Custom(_) => {
                let method_name = payment_method_path_segment(&method)?;
                let url =
                    self.mint_url
                        .join_paths(&["v1", "mint", "quote", method_name, quote_id])?;

                let auth_token = self
                    .get_auth_token(Method::Get, RoutePath::MintQuote(method_name.to_string()))
                    .await?;

                let value: serde_json::Value = self.transport_http_get(url, auth_token).await?;
                let response: MintQuoteCustomResponse<String> =
                    deserialize_with_route_method(value, &method)?;

                Ok(MintQuoteResponse::Custom { method, response })
            }
        }
    }

    /// Look up mint quotes locked to a set of NUT-20 public keys [NUT-XX]
    ///
    /// A malformed entry in the response - currently, a quote object missing a `"method"`
    /// field - is skipped rather than failing the whole lookup: this call can answer for many
    /// pubkeys at once, and one bad entry should not hide every other, perfectly valid quote
    /// from the caller (e.g. a poller reconciling a whole key set). Each skipped entry is logged
    /// via `tracing::warn!` with its quote id, when extractable, and the parse error.
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_mint_quote_by_pubkey(
        &self,
        request: MintQuoteByPubkeyRequest,
    ) -> Result<Vec<MintQuoteResponse<String>>, Error> {
        let url = self
            .mint_url
            .join_paths(&["v1", "mint", "quote", "pubkey"])?;
        let auth_token = self
            .get_auth_token(Method::Post, RoutePath::MintQuoteByPubkey)
            .await?;

        let response: MintQuoteByPubkeyResponse<serde_json::Value> =
            self.transport_http_post(url, auth_token, &request).await?;

        let mut quotes = Vec::with_capacity(response.quotes.len());
        for value in response.quotes {
            // Peek the id before `value` is consumed below, so a parse failure can still be
            // attributed to a specific quote in the warning.
            let quote_id = value
                .get("quote")
                .and_then(|q| q.as_str())
                .map(str::to_string);

            match mint_quote_value_to_response(value) {
                Ok(response) => quotes.push(response),
                Err(e) => {
                    tracing::warn!(
                        "Skipping malformed mint quote {} in pubkey lookup response: {}",
                        quote_id.as_deref().unwrap_or("<unknown>"),
                        e
                    );
                }
            }
        }

        Ok(quotes)
    }

    /// Mint Tokens [NUT-04]
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_mint(
        &self,
        method: &PaymentMethod,
        request: MintRequest<String>,
    ) -> Result<MintResponse, Error> {
        let method_name = payment_method_path_segment(method)?;
        let auth_token = self
            .get_auth_token(Method::Post, RoutePath::Mint(method.to_string()))
            .await?;

        let path = match method {
            PaymentMethod::Known(KnownMethod::Bolt11) => {
                nut19::Path::Custom("/v1/mint/bolt11".to_string())
            }
            PaymentMethod::Known(KnownMethod::Bolt12) => {
                nut19::Path::Custom("/v1/mint/bolt12".to_string())
            }
            PaymentMethod::Custom(_) => nut19::Path::custom_mint(method_name),
            PaymentMethod::Known(KnownMethod::Onchain) => {
                nut19::Path::Custom("/v1/mint/onchain".to_string())
            }
        };

        self.retriable_http_request(nut19::Method::Post, path, auth_token, &request)
            .await
    }

    /// Batch check mint quote status [NUT-29]
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_batch_check_mint_quote_status(
        &self,
        method: &PaymentMethod,
        request: BatchCheckMintQuoteRequest<String>,
    ) -> Result<Vec<MintQuoteResponse<String>>, Error> {
        let method_name = payment_method_path_segment(method)?;
        let url = self
            .mint_url
            .join_paths(&["v1", "mint", "quote", method_name, "check"])?;

        let auth_token = self
            .get_auth_token(Method::Post, RoutePath::MintQuote(method_name.to_string()))
            .await?;

        match method {
            PaymentMethod::Known(KnownMethod::Bolt11) => {
                let responses: Vec<MintQuoteBolt11Response<String>> =
                    self.transport_http_post(url, auth_token, &request).await?;
                Ok(responses
                    .into_iter()
                    .map(MintQuoteResponse::Bolt11)
                    .collect())
            }
            PaymentMethod::Known(KnownMethod::Bolt12) => {
                let responses: Vec<MintQuoteBolt12Response<String>> =
                    self.transport_http_post(url, auth_token, &request).await?;
                Ok(responses
                    .into_iter()
                    .map(MintQuoteResponse::Bolt12)
                    .collect())
            }
            PaymentMethod::Known(KnownMethod::Onchain) => {
                let responses: Vec<MintQuoteOnchainResponse<String>> =
                    self.transport_http_post(url, auth_token, &request).await?;
                Ok(responses
                    .into_iter()
                    .map(MintQuoteResponse::Onchain)
                    .collect())
            }
            PaymentMethod::Custom(method_name) => {
                let value: serde_json::Value =
                    self.transport_http_post(url, auth_token, &request).await?;
                let responses: Vec<MintQuoteCustomResponse<String>> =
                    deserialize_with_route_method(value, method)?;
                Ok(responses
                    .into_iter()
                    .map(|response| MintQuoteResponse::Custom {
                        method: PaymentMethod::Custom(method_name.clone()),
                        response,
                    })
                    .collect())
            }
        }
    }

    /// Batch mint tokens [NUT-29]
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_batch_mint(
        &self,
        method: &PaymentMethod,
        request: BatchMintRequest<String>,
    ) -> Result<MintResponse, Error> {
        let method_name = payment_method_path_segment(method)?;
        let auth_token = self
            .get_auth_token(Method::Post, RoutePath::Mint(method.to_string()))
            .await?;

        let path = nut19::Path::Custom(format!("/v1/mint/{method_name}/batch"));

        self.retriable_http_request(nut19::Method::Post, path, auth_token, &request)
            .await
    }

    /// Melt Quote [NUT-05]
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_melt_quote(
        &self,
        request: MeltQuoteRequest,
    ) -> Result<MeltQuoteCreateResponse<String>, Error> {
        let method = request.method();
        let method_name = payment_method_path_segment(&method)?;

        let url = self
            .mint_url
            .join_paths(&["v1", "melt", "quote", method_name])?;
        let auth_token = self
            .get_auth_token(Method::Post, RoutePath::MeltQuote(method.to_string()))
            .await?;

        match &request {
            MeltQuoteRequest::Bolt11(req) => {
                let response: cdk_common::nut23::MeltQuoteBolt11Response<String> =
                    self.transport_http_post(url, auth_token, req).await?;
                Ok(MeltQuoteCreateResponse::Bolt11(response))
            }
            MeltQuoteRequest::Bolt12(req) => {
                let response: cdk_common::nut25::MeltQuoteBolt12Response<String> =
                    self.transport_http_post(url, auth_token, req).await?;
                Ok(MeltQuoteCreateResponse::Bolt12(response))
            }
            MeltQuoteRequest::Onchain(req) => {
                let response: cdk_common::nut30::MeltQuoteOnchainResponse<String> =
                    self.transport_http_post(url, auth_token, req).await?;
                Ok(MeltQuoteCreateResponse::Onchain(response))
            }
            MeltQuoteRequest::Custom(req) => {
                let value: serde_json::Value =
                    self.transport_http_post(url, auth_token, req).await?;
                let response: cdk_common::nut05::MeltQuoteCustomResponse<String> =
                    deserialize_with_route_method(value, &method)?;
                Ok(MeltQuoteCreateResponse::Custom((method, response)))
            }
        }
    }

    /// Melt Quote Status
    #[instrument(skip(self), fields(mint_url = %self.mint_url))]
    async fn get_melt_quote_status(
        &self,
        method: PaymentMethod,
        quote_id: &str,
    ) -> Result<MeltQuoteResponse<String>, Error> {
        match &method {
            PaymentMethod::Known(KnownMethod::Bolt11) => {
                let url = self
                    .mint_url
                    .join_paths(&["v1", "melt", "quote", "bolt11", quote_id])?;

                let auth_token = self
                    .get_auth_token(
                        Method::Get,
                        RoutePath::MeltQuote(PaymentMethod::Known(KnownMethod::Bolt11).to_string()),
                    )
                    .await?;

                let response: cdk_common::nut23::MeltQuoteBolt11Response<String> =
                    self.transport_http_get(url, auth_token).await?;

                Ok(MeltQuoteResponse::Bolt11(response))
            }
            PaymentMethod::Known(KnownMethod::Bolt12) => {
                let url = self
                    .mint_url
                    .join_paths(&["v1", "melt", "quote", "bolt12", quote_id])?;

                let auth_token = self
                    .get_auth_token(
                        Method::Get,
                        RoutePath::MeltQuote(PaymentMethod::Known(KnownMethod::Bolt12).to_string()),
                    )
                    .await?;

                let response: cdk_common::nut25::MeltQuoteBolt12Response<String> =
                    self.transport_http_get(url, auth_token).await?;

                Ok(MeltQuoteResponse::Bolt12(response))
            }
            PaymentMethod::Known(KnownMethod::Onchain) => {
                let url = self
                    .mint_url
                    .join_paths(&["v1", "melt", "quote", "onchain", quote_id])?;

                let auth_token = self
                    .get_auth_token(
                        Method::Get,
                        RoutePath::MeltQuote(
                            PaymentMethod::Known(KnownMethod::Onchain).to_string(),
                        ),
                    )
                    .await?;

                let response: cdk_common::nut30::MeltQuoteOnchainResponse<String> =
                    self.transport_http_get(url, auth_token).await?;

                Ok(MeltQuoteResponse::Onchain(response))
            }
            PaymentMethod::Custom(_) => {
                let method_name = payment_method_path_segment(&method)?;
                let url =
                    self.mint_url
                        .join_paths(&["v1", "melt", "quote", method_name, quote_id])?;

                let auth_token = self
                    .get_auth_token(Method::Get, RoutePath::MeltQuote(method_name.to_string()))
                    .await?;

                let value: serde_json::Value = self.transport_http_get(url, auth_token).await?;
                let response: cdk_common::nut05::MeltQuoteCustomResponse<String> =
                    deserialize_with_route_method(value, &method)?;

                Ok(MeltQuoteResponse::Custom((method.clone(), response)))
            }
        }
    }

    /// Melt [NUT-05]
    /// [Nut-08] Lightning fee return if outputs defined
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_melt(
        &self,
        method: &PaymentMethod,
        request: MeltRequest<String>,
    ) -> Result<MeltQuoteResponse<String>, Error> {
        let method_name = payment_method_path_segment(method)?;
        let auth_token = self
            .get_auth_token(Method::Post, RoutePath::Melt(method.to_string()))
            .await?;

        let path = match method {
            PaymentMethod::Known(KnownMethod::Bolt11) => {
                nut19::Path::Custom("/v1/melt/bolt11".to_string())
            }
            PaymentMethod::Known(KnownMethod::Bolt12) => {
                nut19::Path::Custom("/v1/melt/bolt12".to_string())
            }
            PaymentMethod::Custom(_) => nut19::Path::custom_melt(method_name),
            PaymentMethod::Known(KnownMethod::Onchain) => {
                nut19::Path::Custom("/v1/melt/onchain".to_string())
            }
        };

        match method {
            PaymentMethod::Known(KnownMethod::Bolt11) => {
                let res: cdk_common::nuts::MeltQuoteBolt11Response<String> = self
                    .retriable_http_request(nut19::Method::Post, path, auth_token, &request)
                    .await?;
                Ok(MeltQuoteResponse::Bolt11(res))
            }
            PaymentMethod::Known(KnownMethod::Bolt12) => {
                let res: cdk_common::nuts::MeltQuoteBolt12Response<String> = self
                    .retriable_http_request(nut19::Method::Post, path, auth_token, &request)
                    .await?;
                Ok(MeltQuoteResponse::Bolt12(res))
            }
            PaymentMethod::Known(KnownMethod::Onchain) => {
                let request = MeltOnchainRequest {
                    quote: request.quote_id().clone(),
                    fee_index: request
                        .selected_fee_index()
                        .ok_or(Error::InvalidPaymentRequest)?,
                    inputs: request.inputs().clone(),
                    outputs: request.outputs().clone(),
                };
                let res: cdk_common::nuts::MeltQuoteOnchainResponse<String> = self
                    .retriable_http_request(nut19::Method::Post, path, auth_token, &request)
                    .await?;
                Ok(MeltQuoteResponse::Onchain(res))
            }
            PaymentMethod::Custom(_) => {
                let value: serde_json::Value = self
                    .retriable_http_request(nut19::Method::Post, path, auth_token, &request)
                    .await?;
                let res: cdk_common::nuts::MeltQuoteCustomResponse<String> =
                    deserialize_with_route_method(value, method)?;
                Ok(MeltQuoteResponse::Custom((method.clone(), res)))
            }
        }
    }

    /// Swap Token [NUT-03]
    #[instrument(skip(self, swap_request), fields(mint_url = %self.mint_url))]
    async fn post_swap(&self, swap_request: SwapRequest) -> Result<SwapResponse, Error> {
        let auth_token = self.get_auth_token(Method::Post, RoutePath::Swap).await?;

        self.retriable_http_request(
            nut19::Method::Post,
            nut19::Path::Swap,
            auth_token,
            &swap_request,
        )
        .await
    }

    /// Helper to get mint info
    async fn get_mint_info(&self) -> Result<MintInfo, Error> {
        let url = self.mint_url.join_paths(&["v1", "info"])?;
        let info: MintInfo = self.transport_http_get(url, None).await?;

        if let Ok(mut cache_support) = self.cache_support.write() {
            *cache_support = (
                info.nuts.nut19.ttl.unwrap_or(300),
                info.nuts
                    .nut19
                    .cached_endpoints
                    .clone()
                    .into_iter()
                    .map(|cached_endpoint| (cached_endpoint.method, cached_endpoint.path))
                    .collect(),
            );
        }

        Ok(info)
    }

    async fn get_auth_wallet(&self) -> Option<AuthWallet> {
        self.auth_wallet.read().await.clone()
    }

    async fn set_auth_wallet(&self, wallet: Option<AuthWallet>) {
        *self.auth_wallet.write().await = wallet;
    }

    /// Spendable check [NUT-07]
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_check_state(
        &self,
        request: CheckStateRequest,
    ) -> Result<CheckStateResponse, Error> {
        let url = self.mint_url.join_paths(&["v1", "checkstate"])?;
        let auth_token = self
            .get_auth_token(Method::Post, RoutePath::Checkstate)
            .await?;

        self.transport_http_post(url, auth_token, &request).await
    }

    /// Restore request [NUT-13]
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_restore(&self, request: RestoreRequest) -> Result<RestoreResponse, Error> {
        let url = self.mint_url.join_paths(&["v1", "restore"])?;
        let auth_token = self
            .get_auth_token(Method::Post, RoutePath::Restore)
            .await?;

        self.transport_http_post(url, auth_token, &request).await
    }
}

/// Http Client

#[derive(Debug, Clone)]
pub struct AuthHttpClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    transport: Arc<T>,
    mint_url: MintUrl,
    cat: Arc<RwLock<AuthToken>>,
}

impl<T> AuthHttpClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    async fn transport_http_get<R>(&self, url: Url, auth: Option<AuthToken>) -> Result<R, Error>
    where
        R: DeserializeOwned,
    {
        self.transport
            .http_get(url, auth)
            .await
            .map_err(HttpClient::<T>::map_http_error)
    }

    async fn transport_http_post<P, R>(
        &self,
        url: Url,
        auth: Option<AuthToken>,
        payload: &P,
    ) -> Result<R, Error>
    where
        P: Serialize + Send + Sync,
        R: DeserializeOwned,
    {
        self.transport
            .http_post(url, auth, payload)
            .await
            .map_err(HttpClient::<T>::map_http_error)
    }

    /// Create new [`AuthHttpClient`]
    pub fn new(mint_url: MintUrl, cat: Option<AuthToken>) -> Self
    where
        T: Default,
    {
        Self {
            transport: T::default().into(),
            mint_url,
            cat: Arc::new(RwLock::new(
                cat.unwrap_or(AuthToken::ClearAuth("".to_string())),
            )),
        }
    }
    /// Create new [`AuthHttpClient`] with a provided transport implementation.
    pub fn with_transport(mint_url: MintUrl, transport: T, cat: Option<AuthToken>) -> Self {
        Self {
            transport: transport.into(),
            mint_url,
            cat: Arc::new(RwLock::new(
                cat.unwrap_or(AuthToken::ClearAuth("".to_string())),
            )),
        }
    }

    /// Create new [`AuthHttpClient`] with a proxy for specific TLDs.
    /// Specifying `None` for `host_matcher` will use the proxy for all
    /// requests.
    pub fn with_proxy(
        mint_url: MintUrl,
        proxy: Url,
        host_matcher: Option<&str>,
        accept_invalid_certs: bool,
        cat: Option<AuthToken>,
    ) -> Result<Self, Error>
    where
        T: Default,
    {
        let mut transport = T::default();
        transport
            .with_proxy(proxy, host_matcher, accept_invalid_certs)
            .map_err(HttpClient::<T>::map_http_error)?;

        Ok(Self::with_transport(mint_url, transport, cat))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl<T> AuthMintConnector for AuthHttpClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    async fn get_auth_token(&self) -> Result<AuthToken, Error> {
        Ok(self.cat.read().await.clone())
    }

    async fn set_auth_token(&self, token: AuthToken) -> Result<(), Error> {
        *self.cat.write().await = token;
        Ok(())
    }

    /// Get Mint Info [NUT-06]
    async fn get_mint_info(&self) -> Result<MintInfo, Error> {
        let url = self.mint_url.join_paths(&["v1", "info"])?;
        let mint_info: MintInfo = self.transport_http_get::<MintInfo>(url, None).await?;

        Ok(mint_info)
    }

    /// Get Auth Keyset Keys [NUT-22]
    #[instrument(skip(self), fields(mint_url = %self.mint_url))]
    async fn get_mint_blind_auth_keyset(&self, keyset_id: Id) -> Result<KeySet, Error> {
        let url =
            self.mint_url
                .join_paths(&["v1", "auth", "blind", "keys", &keyset_id.to_string()])?;

        let mut keys_response = self.transport_http_get::<KeysResponse>(url, None).await?;

        let keyset = keys_response
            .keysets
            .drain(0..1)
            .next()
            .ok_or_else(|| Error::UnknownKeySet)?;

        Ok(keyset)
    }

    /// Get Auth Keysets [NUT-22]
    #[instrument(skip(self), fields(mint_url = %self.mint_url))]
    async fn get_mint_blind_auth_keysets(&self) -> Result<KeysetResponse, Error> {
        let url = self
            .mint_url
            .join_paths(&["v1", "auth", "blind", "keysets"])?;

        self.transport_http_get(url, None).await
    }

    /// Mint Tokens [NUT-22]
    #[instrument(skip(self, request), fields(mint_url = %self.mint_url))]
    async fn post_mint_blind_auth(&self, request: MintAuthRequest) -> Result<MintResponse, Error> {
        let url = self.mint_url.join_paths(&["v1", "auth", "blind", "mint"])?;
        self.transport_http_post(url, Some(self.cat.read().await.clone()), &request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::str::FromStr;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use cdk_common::MintQuoteState;
    use cdk_http_client::{HttpError, RawResponse};
    use serde::de::DeserializeOwned;

    use super::*;
    use crate::nuts::nut04::MintQuoteCustomRequest;
    use crate::nuts::nut05::MeltQuoteCustomRequest;

    /// A mock transport that captures the serialized POST payload and returns
    /// a canned JSON response. Follows the same canned-response pattern as
    /// `MockMintConnector` in `wallet/test_utils.rs`.
    #[derive(Clone, Default)]
    struct MockTransport {
        /// The last payload serialized by `http_post`, captured as JSON.
        captured_payload: Arc<Mutex<Option<serde_json::Value>>>,
        /// Canned JSON string returned by `http_post`.
        post_response: Arc<Mutex<Option<String>>>,
        /// Canned JSON string returned by `http_get`.
        get_response: Arc<Mutex<Option<String>>>,
        /// URLs passed to `http_get`.
        get_urls: Arc<Mutex<Vec<String>>>,
        /// URLs passed to `http_post`.
        post_urls: Arc<Mutex<Vec<String>>>,
    }

    impl fmt::Debug for MockTransport {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("MockTransport").finish()
        }
    }

    #[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
    #[cfg_attr(not(target_arch = "wasm32"), async_trait)]
    impl Transport for MockTransport {
        fn with_proxy(
            &mut self,
            _proxy: Url,
            _host_matcher: Option<&str>,
            _accept_invalid_certs: bool,
        ) -> Result<(), HttpError> {
            Ok(())
        }

        #[cfg(all(feature = "bip353", not(target_arch = "wasm32")))]
        async fn resolve_dns_txt(&self, _domain: &str) -> Result<Vec<String>, HttpError> {
            Ok(vec![])
        }

        async fn http_get<R>(&self, _url: Url, _auth: Option<AuthToken>) -> Result<R, HttpError>
        where
            R: DeserializeOwned,
        {
            self.get_urls.lock().expect("lock").push(_url.to_string());
            let json = self
                .get_response
                .lock()
                .expect("lock")
                .clone()
                .expect("no mock response set");
            serde_json::from_str(&json).map_err(|e| HttpError::Serialization(e.to_string()))
        }

        async fn http_get_raw(
            &self,
            url: Url,
            _auth: Option<AuthToken>,
        ) -> Result<RawResponse, HttpError> {
            self.get_urls.lock().expect("lock").push(url.to_string());
            let json = self
                .get_response
                .lock()
                .expect("lock")
                .clone()
                .expect("no mock response set");
            Ok(RawResponse::new(200, json.into_bytes()))
        }

        async fn http_post<P, R>(
            &self,
            _url: Url,
            _auth_token: Option<AuthToken>,
            payload: &P,
        ) -> Result<R, HttpError>
        where
            P: serde::Serialize + Send + Sync,
            R: DeserializeOwned,
        {
            self.post_urls.lock().expect("lock").push(_url.to_string());
            // Capture the serialized payload for test assertions
            let value = serde_json::to_value(payload)
                .map_err(|e| HttpError::Serialization(e.to_string()))?;
            *self.captured_payload.lock().expect("lock") = Some(value);

            // Return the canned response
            let json = self
                .post_response
                .lock()
                .expect("lock")
                .clone()
                .expect("no mock response set");
            serde_json::from_str(&json).map_err(|e| HttpError::Serialization(e.to_string()))
        }

        async fn http_post_form_raw<P>(
            &self,
            url: Url,
            _auth_token: Option<AuthToken>,
            payload: &P,
        ) -> Result<RawResponse, HttpError>
        where
            P: serde::Serialize + Send + Sync,
        {
            self.post_urls.lock().expect("lock").push(url.to_string());
            let value = serde_json::to_value(payload)
                .map_err(|e| HttpError::Serialization(e.to_string()))?;
            *self.captured_payload.lock().expect("lock") = Some(value);

            let json = self
                .post_response
                .lock()
                .expect("lock")
                .clone()
                .expect("no mock response set");
            Ok(RawResponse::new(200, json.into_bytes()))
        }
    }

    /// Regression test: `post_mint_quote` must send only the
    /// `MintQuoteCustomRequest` as the JSON body for custom payment methods,
    /// not the `(PaymentMethod, MintQuoteCustomRequest)` tuple which
    /// serializes as a JSON array.
    #[tokio::test]
    async fn test_post_mint_quote_custom_sends_request_object() {
        let canned_json = serde_json::json!({
            "quote": "test-quote-id",
            "request": "paypal://pay?id=123",
            "amount": 1000,
            "amount_paid": 0,
            "amount_issued": 0,
            "updated_at": 0,
            "unit": "sat",
            "expiry": 9999999
        })
        .to_string();

        let transport = MockTransport {
            captured_payload: Arc::new(Mutex::new(None)),
            post_response: Arc::new(Mutex::new(Some(canned_json))),
            get_response: Arc::new(Mutex::new(None)),
            get_urls: Arc::new(Mutex::new(Vec::new())),
            post_urls: Arc::new(Mutex::new(Vec::new())),
        };
        let captured = transport.captured_payload.clone();

        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);

        let request = MintQuoteRequest::Custom {
            method: PaymentMethod::Custom("paypal".to_string()),
            request: MintQuoteCustomRequest {
                amount: Some(cdk_common::Amount::from(1000)),
                unit: cdk_common::CurrencyUnit::Sat,
                description: None,
                pubkey: None,
                extra: serde_json::Value::Null,
            },
        };

        let response = client
            .post_mint_quote(request)
            .await
            .expect("post_mint_quote should succeed");

        match response {
            MintQuoteResponse::Custom { method, response } => {
                assert_eq!(method, PaymentMethod::Custom("paypal".to_string()));
                assert_eq!(response.method, PaymentMethod::Custom("paypal".to_string()));
            }
            _ => panic!("expected custom response"),
        }

        // Verify the payload sent to the transport was a JSON object (not an array)
        let payload = captured
            .lock()
            .expect("lock")
            .clone()
            .expect("payload was captured");
        assert!(
            payload.is_object(),
            "Custom mint quote body sent to transport must be a JSON object, got: {payload}"
        );

        // Verify the payload deserializes as MintQuoteCustomRequest
        let parsed: Result<MintQuoteCustomRequest, _> = serde_json::from_value(payload.clone());
        assert!(
            parsed.is_ok(),
            "Transport payload must deserialize as MintQuoteCustomRequest: {:?}",
            parsed.err()
        );

        // Verify the actual field values round-tripped correctly
        let parsed = parsed.expect("already checked");
        assert_eq!(parsed.amount, Some(cdk_common::Amount::from(1000)));
        assert_eq!(parsed.unit, cdk_common::CurrencyUnit::Sat);
    }

    #[tokio::test]
    async fn test_invalid_custom_method_is_rejected_before_transport() {
        let transport = MockTransport::default();
        let get_urls = transport.get_urls.clone();
        let post_urls = transport.post_urls.clone();
        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);
        let invalid_method = PaymentMethod::Custom("../../v1/swap".to_string());

        let result = client
            .post_mint_quote(MintQuoteRequest::Custom {
                method: invalid_method.clone(),
                request: MintQuoteCustomRequest {
                    amount: Some(cdk_common::Amount::from(1000)),
                    unit: cdk_common::CurrencyUnit::Sat,
                    description: None,
                    pubkey: None,
                    extra: serde_json::Value::Null,
                },
            })
            .await;
        assert!(matches!(result, Err(Error::InvalidPaymentMethod)));

        let result = client
            .get_mint_quote_status(invalid_method.clone(), "test-quote-id")
            .await;
        assert!(matches!(result, Err(Error::InvalidPaymentMethod)));

        let result = client
            .post_mint(
                &invalid_method,
                MintRequest {
                    quote: "test-quote-id".to_string(),
                    outputs: Vec::new(),
                    signature: None,
                },
            )
            .await;
        assert!(matches!(result, Err(Error::InvalidPaymentMethod)));

        let result = client
            .post_batch_check_mint_quote_status(
                &invalid_method,
                BatchCheckMintQuoteRequest {
                    quotes: vec!["test-quote-id".to_string()],
                },
            )
            .await;
        assert!(matches!(result, Err(Error::InvalidPaymentMethod)));

        let result = client
            .post_batch_mint(
                &invalid_method,
                BatchMintRequest {
                    quotes: vec!["test-quote-id".to_string()],
                    quote_amounts: None,
                    outputs: Vec::new(),
                    signatures: None,
                },
            )
            .await;
        assert!(matches!(result, Err(Error::InvalidPaymentMethod)));

        let result = client
            .post_melt_quote(MeltQuoteRequest::Custom(MeltQuoteCustomRequest {
                method: "../../v1/swap".to_string(),
                request: "custom-payment-request".to_string(),
                unit: cdk_common::CurrencyUnit::Sat,
                amount: None,
                extra: serde_json::Value::Null,
            }))
            .await;
        assert!(matches!(result, Err(Error::InvalidPaymentMethod)));

        let result = client
            .get_melt_quote_status(invalid_method.clone(), "test-quote-id")
            .await;
        assert!(matches!(result, Err(Error::InvalidPaymentMethod)));

        let result = client
            .post_melt(
                &invalid_method,
                MeltRequest::new("test-quote-id".to_string(), Vec::new(), None),
            )
            .await;
        assert!(matches!(result, Err(Error::InvalidPaymentMethod)));

        assert!(
            get_urls.lock().expect("lock").is_empty(),
            "invalid custom method must be rejected before GET transport"
        );
        assert!(
            post_urls.lock().expect("lock").is_empty(),
            "invalid custom method must be rejected before POST transport"
        );
    }

    #[tokio::test]
    async fn test_get_mint_quote_custom_derives_state_from_amounts() {
        let canned_json = serde_json::json!({
            "quote": "test-quote-id",
            "request": "paypal://pay?id=123",
            "amount": 1000,
            "amount_paid": 1000,
            "amount_issued": 0,
            "unit": "sat"
        })
        .to_string();

        let transport = MockTransport {
            get_response: Arc::new(Mutex::new(Some(canned_json))),
            ..Default::default()
        };
        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);

        let response = client
            .get_mint_quote_status(PaymentMethod::Custom("paypal".to_string()), "test-quote-id")
            .await
            .expect("custom quote status");

        assert_eq!(response.state(), Some(MintQuoteState::Paid));
        match response {
            MintQuoteResponse::Custom { method, response } => {
                assert_eq!(method, PaymentMethod::Custom("paypal".to_string()));
                assert_eq!(response.method, PaymentMethod::Custom("paypal".to_string()));
                assert_eq!(response.amount_paid, cdk_common::Amount::from(1000));
                assert_eq!(response.amount_issued, cdk_common::Amount::ZERO);
            }
            _ => panic!("expected custom response"),
        }
    }

    #[tokio::test]
    async fn test_batch_check_mint_quote_custom_parses_custom_responses() {
        let canned_json = serde_json::json!([
            {
                "quote": "test-quote-id",
                "request": "paypal://pay?id=123",
                "amount": 1000,
                "amount_paid": 1000,
                "amount_issued": 1000,
                "unit": "sat"
            }
        ])
        .to_string();

        let transport = MockTransport {
            post_response: Arc::new(Mutex::new(Some(canned_json))),
            ..Default::default()
        };
        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);

        let responses = client
            .post_batch_check_mint_quote_status(
                &PaymentMethod::Custom("paypal".to_string()),
                BatchCheckMintQuoteRequest {
                    quotes: vec!["test-quote-id".to_string()],
                },
            )
            .await
            .expect("custom batch quote status");

        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].state(), Some(MintQuoteState::Issued));
        match &responses[0] {
            MintQuoteResponse::Custom { method, response } => {
                assert_eq!(method, &PaymentMethod::Custom("paypal".to_string()));
                assert_eq!(response.method, PaymentMethod::Custom("paypal".to_string()));
            }
            _ => panic!("expected custom response"),
        }
    }

    /// The mint answers `/v1/mint/quote/pubkey` with quotes flattened to their bare NUT-04
    /// object (see `mint_quote_response_to_value` in `cdk-axum`), not wrapped in
    /// `MintQuoteResponse`'s own externally tagged envelope. A response mixing a known method
    /// (bolt11) and a custom method (paypal) must still reconstruct both correctly, and the
    /// request must go to the method-agnostic path with no `{method}` segment in it.
    #[tokio::test]
    async fn test_post_mint_quote_by_pubkey_reconstructs_mixed_methods() {
        let canned_json = serde_json::json!({
            "quotes": [
                {
                    "quote": "bolt11-quote-id",
                    "request": "lnbc1...",
                    "amount": 1000,
                    "unit": "sat",
                    "method": "bolt11",
                    "amount_paid": 1000,
                    "amount_issued": 0,
                    "updated_at": 42,
                    "state": "PAID",
                    "expiry": 9999999999_u64
                },
                {
                    "quote": "custom-quote-id",
                    "request": "paypal://pay?id=123",
                    "method": "paypal",
                    "amount": 500,
                    "amount_paid": 0,
                    "amount_issued": 0,
                    "updated_at": 7,
                    "unit": "sat",
                    "expiry": 9999999999_u64
                }
            ]
        })
        .to_string();

        let transport = MockTransport {
            post_response: Arc::new(Mutex::new(Some(canned_json))),
            ..Default::default()
        };
        let post_urls = transport.post_urls.clone();
        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);

        let secret_key = crate::nuts::SecretKey::generate();
        let request = MintQuoteByPubkeyRequest {
            pubkeys: vec![secret_key.public_key()],
            pubkey_signatures: vec![secret_key.sign(b"test-message").expect("sign")],
            only_mintable: false,
        };

        let responses = client
            .post_mint_quote_by_pubkey(request)
            .await
            .expect("post_mint_quote_by_pubkey should succeed");

        assert_eq!(responses.len(), 2);
        match &responses[0] {
            MintQuoteResponse::Bolt11(r) => {
                assert_eq!(r.quote, "bolt11-quote-id");
                assert_eq!(r.state, MintQuoteState::Paid);
            }
            other => panic!("expected bolt11 response, got {other:?}"),
        }
        match &responses[1] {
            MintQuoteResponse::Custom { method, response } => {
                assert_eq!(method, &PaymentMethod::Custom("paypal".to_string()));
                assert_eq!(response.quote, "custom-quote-id");
            }
            other => panic!("expected custom response, got {other:?}"),
        }

        // No `{method}` path segment: this endpoint is method-agnostic.
        assert_eq!(
            post_urls.lock().expect("lock").as_slice(),
            ["https://mint.example.com/v1/mint/quote/pubkey"]
        );
    }

    /// Unit contract of `mint_quote_value_to_response` itself: a quote with no `"method"` field
    /// must be rejected, not silently mislabeled as bolt11. Before the fix, it defaulted a
    /// missing method to `PaymentMethod::BOLT11`, which would parse a paid
    /// bolt12/onchain/custom quote as an *unpaid* `MintQuoteResponse::Bolt11` — type confusion
    /// the caller has no way to detect. This contract holds regardless of what
    /// `post_mint_quote_by_pubkey` does with the error (see
    /// `test_post_mint_quote_by_pubkey_skips_malformed_quotes` for that, batch-level behavior).
    #[test]
    fn test_mint_quote_value_to_response_rejects_missing_method() {
        let value = serde_json::json!({
            "quote": "no-method-quote-id",
            "request": "lnbc1...",
            "amount": 1000,
            "amount_paid": 1000,
            "amount_issued": 0,
            "updated_at": 42,
            "unit": "sat",
            "expiry": 9999999999_u64
        });

        let result = mint_quote_value_to_response(value);

        match result {
            Err(Error::Custom(msg)) => {
                assert!(
                    msg.contains("no-method-quote-id"),
                    "error should name the offending quote id, got: {msg}"
                );
            }
            other => panic!(
                "a method-less quote must be rejected, not silently treated as bolt11: {other:?}"
            ),
        }
    }

    /// A malformed entry (here, missing `"method"`) must not discard the rest of the batch: a
    /// poller reconciling many quotes at once would otherwise see nothing at all for a pubkey
    /// because of one bad quote, while that bad quote persists forever. The malformed entry is
    /// dropped and the well-formed ones are still returned.
    #[tokio::test]
    async fn test_post_mint_quote_by_pubkey_skips_malformed_quotes() {
        let canned_json = serde_json::json!({
            "quotes": [
                {
                    "quote": "good-bolt11-quote-id",
                    "request": "lnbc1...",
                    "amount": 1000,
                    "unit": "sat",
                    "method": "bolt11",
                    "amount_paid": 1000,
                    "amount_issued": 0,
                    "updated_at": 42,
                    "state": "PAID",
                    "expiry": 9999999999_u64
                },
                {
                    "quote": "no-method-quote-id",
                    "request": "lnbc1...",
                    "amount": 1000,
                    "amount_paid": 1000,
                    "amount_issued": 0,
                    "updated_at": 42,
                    "unit": "sat",
                    "expiry": 9999999999_u64
                }
            ]
        })
        .to_string();

        let transport = MockTransport {
            post_response: Arc::new(Mutex::new(Some(canned_json))),
            ..Default::default()
        };
        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);

        let secret_key = crate::nuts::SecretKey::generate();
        let request = MintQuoteByPubkeyRequest {
            pubkeys: vec![secret_key.public_key()],
            pubkey_signatures: vec![secret_key.sign(b"test-message").expect("sign")],
            only_mintable: false,
        };

        let responses = client
            .post_mint_quote_by_pubkey(request)
            .await
            .expect("a malformed entry must not fail the whole lookup");

        assert_eq!(responses.len(), 1);
        match &responses[0] {
            MintQuoteResponse::Bolt11(r) => assert_eq!(r.quote, "good-bolt11-quote-id"),
            other => panic!("expected the good bolt11 response, got {other:?}"),
        }
    }

    /// A `"bolt12"` object must reconstruct as `MintQuoteResponse::Bolt12`, not the pre-fix
    /// default of Bolt11.
    #[tokio::test]
    async fn test_post_mint_quote_by_pubkey_reconstructs_bolt12() {
        let pubkey = crate::nuts::SecretKey::generate().public_key();
        let canned_json = serde_json::json!({
            "quotes": [
                {
                    "quote": "bolt12-quote-id",
                    "request": "lno1...",
                    "method": "bolt12",
                    "amount": 500,
                    "unit": "sat",
                    "expiry": 9999999999_u64,
                    "pubkey": pubkey.to_hex(),
                    "amount_paid": 500,
                    "amount_issued": 0,
                    "updated_at": 5
                }
            ]
        })
        .to_string();

        let transport = MockTransport {
            post_response: Arc::new(Mutex::new(Some(canned_json))),
            ..Default::default()
        };
        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);

        let secret_key = crate::nuts::SecretKey::generate();
        let request = MintQuoteByPubkeyRequest {
            pubkeys: vec![secret_key.public_key()],
            pubkey_signatures: vec![secret_key.sign(b"test-message").expect("sign")],
            only_mintable: false,
        };

        let responses = client
            .post_mint_quote_by_pubkey(request)
            .await
            .expect("post_mint_quote_by_pubkey should succeed");

        assert_eq!(responses.len(), 1);
        match &responses[0] {
            MintQuoteResponse::Bolt12(r) => {
                assert_eq!(r.quote, "bolt12-quote-id");
                assert_eq!(r.amount_paid, cdk_common::Amount::from(500));
            }
            other => panic!("expected bolt12 response, got {other:?}"),
        }
    }

    /// An `"onchain"` object must reconstruct as `MintQuoteResponse::Onchain`, not the pre-fix
    /// default of Bolt11.
    #[tokio::test]
    async fn test_post_mint_quote_by_pubkey_reconstructs_onchain() {
        let pubkey = crate::nuts::SecretKey::generate().public_key();
        let canned_json = serde_json::json!({
            "quotes": [
                {
                    "quote": "onchain-quote-id",
                    "request": "bc1qexample",
                    "method": "onchain",
                    "unit": "sat",
                    "pubkey": pubkey.to_hex(),
                    "amount_paid": 750,
                    "amount_issued": 0,
                    "updated_at": 3
                }
            ]
        })
        .to_string();

        let transport = MockTransport {
            post_response: Arc::new(Mutex::new(Some(canned_json))),
            ..Default::default()
        };
        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);

        let secret_key = crate::nuts::SecretKey::generate();
        let request = MintQuoteByPubkeyRequest {
            pubkeys: vec![secret_key.public_key()],
            pubkey_signatures: vec![secret_key.sign(b"test-message").expect("sign")],
            only_mintable: false,
        };

        let responses = client
            .post_mint_quote_by_pubkey(request)
            .await
            .expect("post_mint_quote_by_pubkey should succeed");

        assert_eq!(responses.len(), 1);
        match &responses[0] {
            MintQuoteResponse::Onchain(r) => {
                assert_eq!(r.quote, "onchain-quote-id");
                assert_eq!(r.amount_paid, cdk_common::Amount::from(750));
            }
            other => panic!("expected onchain response, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_post_melt_quote_custom_derives_missing_method_from_route() {
        let canned_json = serde_json::json!({
            "quote": "test-melt-quote-id",
            "amount": 1000,
            "fee_reserve": 10,
            "state": "UNPAID",
            "expiry": 9999999,
            "request": "paypal://pay?id=123",
            "unit": "sat"
        })
        .to_string();

        let transport = MockTransport {
            post_response: Arc::new(Mutex::new(Some(canned_json))),
            ..Default::default()
        };
        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);

        let response = client
            .post_melt_quote(MeltQuoteRequest::Custom(MeltQuoteCustomRequest {
                method: "paypal".to_string(),
                request: "paypal://pay?id=123".to_string(),
                unit: cdk_common::CurrencyUnit::Sat,
                amount: None,
                extra: serde_json::Value::Null,
            }))
            .await
            .expect("custom melt quote");

        match response {
            MeltQuoteCreateResponse::Custom((method, response)) => {
                assert_eq!(method, PaymentMethod::Custom("paypal".to_string()));
                assert_eq!(response.method, PaymentMethod::Custom("paypal".to_string()));
            }
            _ => panic!("expected custom response"),
        }
    }

    #[tokio::test]
    async fn test_fetch_lnurl_invoice_rejects_loopback_url_before_transport() {
        let transport = MockTransport::default();
        let get_urls = transport.get_urls.clone();
        let mint_url = MintUrl::from_str("https://mint.example.com").expect("parse url");
        let client = HttpClient::with_transport(mint_url, transport, None);

        let result = client
            .fetch_lnurl_invoice("http://127.0.0.1:8332/?amount=1000")
            .await;

        assert!(
            result.is_err(),
            "fetch_lnurl_invoice must reject loopback URLs to prevent SSRF"
        );
        assert!(
            get_urls.lock().expect("lock").is_empty(),
            "invalid LNURL callback must be rejected before transport"
        );
    }
}
