//! Wallet-side ehash helpers.

use cdk::error::Error;
use cdk::mint_url::MintUrl;
use cdk::nuts::{MintRequest, MintResponse};
use cdk::wallet::mint_connector::transport::Transport;
use cdk_common::mint::BatchMintRequest;
use cdk_common::quote_id::QuoteId;

use crate::types::{EhashQuoteRequest, EhashQuoteResponse};

/// Simple wallet client for ehash endpoints.
#[derive(Debug, Clone)]
pub struct EhashWalletClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    mint_url: MintUrl,
    transport: T,
}

impl<T> EhashWalletClient<T>
where
    T: Transport + Send + Sync + 'static,
{
    /// Create a new ehash wallet client.
    pub fn new(mint_url: MintUrl, transport: T) -> Self {
        Self {
            mint_url,
            transport,
        }
    }

    /// Create an ehash mint quote.
    pub async fn create_ehash_quote(
        &self,
        request: EhashQuoteRequest,
    ) -> Result<EhashQuoteResponse<String>, Error> {
        let url = self
            .mint_url
            .join_paths(&["v1", "mint", "quote", "ehash"])?;

        let response: EhashQuoteResponse<String> =
            self.transport.http_post(url, None, &request).await?;

        Ok(response)
    }

    /// Check the status of an ehash quote.
    pub async fn check_ehash_quote(
        &self,
        quote_id: &QuoteId,
    ) -> Result<EhashQuoteResponse<String>, Error> {
        let url =
            self.mint_url
                .join_paths(&["v1", "mint", "quote", "ehash", &quote_id.to_string()])?;

        self.transport.http_get(url, None).await
    }

    /// Mint tokens for an ehash quote.
    pub async fn mint_ehash(&self, request: MintRequest<String>) -> Result<MintResponse, Error> {
        let url = self.mint_url.join_paths(&["v1", "mint", "ehash"])?;
        self.transport.http_post(url, None, &request).await
    }

    /// Batch mint tokens for ehash quotes.
    pub async fn batch_mint_ehash(&self, request: BatchMintRequest) -> Result<MintResponse, Error> {
        let url = self
            .mint_url
            .join_paths(&["v1", "mint", "ehash", "batch"])?;
        self.transport.http_post(url, None, &request).await
    }
}
