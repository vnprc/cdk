#![allow(clippy::unwrap_used)]

//! NUT-XX: in-process, end-to-end coverage of `Wallet::fetch_mint_quotes_by_pubkey` against a
//! real `Mint`.
//!
//! <https://github.com/cashubtc/nuts/blob/get-quotes-by-pubkeys/xx.md>
//!
//! This exercises the wallet's signing and request-assembly logic, the connector plumbing, and
//! the mint's own signature verification and quote lookup - the same components a real
//! deployment wires together, just without going over HTTP: `cdk` cannot dev-depend on
//! `cdk-axum` (which itself depends on `cdk`), so there is no way to stand up a real mint HTTP
//! server from this crate's own test suite. Coverage of the actual wire format - the mint's
//! flattened per-quote JSON and its reconstruction into `MintQuoteResponse` - lives instead in
//! the `MockTransport`-backed `test_post_mint_quote_by_pubkey_reconstructs_mixed_methods` in
//! `crates/cdk/src/wallet/mint_connector/http_client.rs`.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use bip39::Mnemonic;
use cdk::mint::{Mint, MintBuilder, MintMeltLimits};
use cdk::mint_url::MintUrl;
use cdk::nuts::nut00::KnownMethod;
use cdk::nuts::nutxx::MintQuoteByPubkeyRequest;
use cdk::nuts::{
    BatchCheckMintQuoteRequest, BatchMintRequest, CheckStateRequest, CheckStateResponse,
    CurrencyUnit, Id, KeySet, KeysetResponse, MeltRequest, MintInfo, MintQuoteBolt11Request,
    MintRequest, MintResponse, PaymentMethod, PublicKey, RestoreRequest, RestoreResponse,
    SecretKey, SwapRequest, SwapResponse,
};
use cdk::types::{FeeReserve, QuoteTTL};
use cdk::wallet::{AuthWallet, MintConnector, Wallet, WalletBuilder};
use cdk::{
    Amount, Error, MeltQuoteCreateResponse, MeltQuoteRequest, MeltQuoteResponse, MintQuoteRequest,
    MintQuoteResponse,
};
use cdk_fake_wallet::FakeWallet;

async fn test_mint() -> Mint {
    let db = Arc::new(cdk_sqlite::mint::memory::empty().await.unwrap());
    let mut builder = MintBuilder::new(db.clone());

    let backend = FakeWallet::new(
        FeeReserve {
            min_fee_reserve: 1.into(),
            percent_fee_reserve: 1.0,
        },
        HashMap::default(),
        HashSet::default(),
        2,
        CurrencyUnit::Sat,
    );

    builder
        .add_payment_processor(
            CurrencyUnit::Sat,
            PaymentMethod::Known(KnownMethod::Bolt11),
            MintMeltLimits::new(1, 10_000),
            Arc::new(backend),
        )
        .await
        .unwrap();

    let mnemonic = Mnemonic::generate(12).unwrap();
    builder = builder
        .with_name("nutxx wallet test mint".to_string())
        .with_description("nutxx wallet test mint".to_string())
        .with_urls(vec!["https://test-mint".to_string()]);

    let mint = builder
        .build_with_seed(db.clone(), &mnemonic.to_seed_normalized(""))
        .await
        .unwrap();
    mint.set_quote_ttl(QuoteTTL::new(10_000, 10_000))
        .await
        .unwrap();
    mint
}

/// Create a NUT-20 locked bolt11 mint quote owned by `pubkey`.
async fn locked_quote(mint: &Mint, pubkey: PublicKey) {
    mint.get_mint_quote(MintQuoteRequest::Bolt11(MintQuoteBolt11Request {
        amount: Amount::new(100, CurrencyUnit::Sat).into(),
        unit: CurrencyUnit::Sat,
        description: None,
        pubkey: Some(pubkey),
    }))
    .await
    .unwrap();
}

/// A minimal in-process `MintConnector` wrapping a `Mint` directly (no HTTP/JSON), implementing
/// only what `Wallet::fetch_mint_quotes_by_pubkey` needs: `get_mint_info` (to learn the mint's
/// NUT-06 pubkey) and `post_mint_quote_by_pubkey` (the lookup itself). Every other method is
/// unimplemented - this connector exists to exercise NUT-XX only.
struct DirectConnector(Mint);

impl fmt::Debug for DirectConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DirectConnector")
    }
}

#[async_trait]
impl MintConnector for DirectConnector {
    async fn resolve_dns_txt(&self, _domain: &str) -> Result<Vec<String>, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn fetch_lnurl_pay_request(
        &self,
        _url: &str,
    ) -> Result<cdk::wallet::LnurlPayResponse, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn fetch_lnurl_invoice(
        &self,
        _url: &str,
    ) -> Result<cdk::wallet::LnurlPayInvoiceResponse, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn get_mint_keys(&self) -> Result<Vec<KeySet>, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn get_mint_keyset(&self, _keyset_id: Id) -> Result<KeySet, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    /// `load_mint_info` (used by `fetch_mint_quotes_by_pubkey` to find the mint's NUT-06 pubkey)
    /// refreshes keysets alongside mint info as part of its normal metadata-cache flow. An
    /// empty keyset list keeps that refresh a no-op without needing a real `get_mint_keyset`.
    async fn get_mint_keysets(&self) -> Result<KeysetResponse, Error> {
        Ok(KeysetResponse {
            keysets: Vec::new(),
        })
    }

    async fn post_mint_quote(
        &self,
        _request: MintQuoteRequest,
    ) -> Result<MintQuoteResponse<String>, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn post_mint(
        &self,
        _method: &PaymentMethod,
        _request: MintRequest<String>,
    ) -> Result<MintResponse, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn post_batch_check_mint_quote_status(
        &self,
        _method: &PaymentMethod,
        _request: BatchCheckMintQuoteRequest<String>,
    ) -> Result<Vec<MintQuoteResponse<String>>, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn post_batch_mint(
        &self,
        _method: &PaymentMethod,
        _request: BatchMintRequest<String>,
    ) -> Result<MintResponse, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn post_melt_quote(
        &self,
        _request: MeltQuoteRequest,
    ) -> Result<MeltQuoteCreateResponse<String>, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn get_mint_quote_status(
        &self,
        _method: PaymentMethod,
        _quote_id: &str,
    ) -> Result<MintQuoteResponse<String>, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    /// The one call this test actually exercises: forward to the real mint in-process and
    /// convert its `QuoteId`-keyed response to the wallet-facing `String`-keyed one, exactly as
    /// `cdk-integration-tests`' `DirectMintConnection` does for the other quote endpoints.
    async fn post_mint_quote_by_pubkey(
        &self,
        request: MintQuoteByPubkeyRequest,
    ) -> Result<Vec<MintQuoteResponse<String>>, Error> {
        let responses = self
            .0
            .get_mint_quote_by_pubkey(request.pubkeys, request.pubkey_signatures)
            .await?;

        Ok(responses.into_iter().map(Into::into).collect())
    }

    async fn get_melt_quote_status(
        &self,
        _method: PaymentMethod,
        _quote_id: &str,
    ) -> Result<MeltQuoteResponse<String>, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn post_melt(
        &self,
        _method: &PaymentMethod,
        _request: MeltRequest<String>,
    ) -> Result<MeltQuoteResponse<String>, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn post_swap(&self, _request: SwapRequest) -> Result<SwapResponse, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    /// The other call this test exercises: the wallet needs the mint's real NUT-06 pubkey to
    /// build a signature the mint will actually accept.
    async fn get_mint_info(&self) -> Result<MintInfo, Error> {
        self.0.mint_info().await
    }

    async fn post_check_state(
        &self,
        _request: CheckStateRequest,
    ) -> Result<CheckStateResponse, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn post_restore(&self, _request: RestoreRequest) -> Result<RestoreResponse, Error> {
        unimplemented!("not exercised by the NUT-XX lookup test")
    }

    async fn get_auth_wallet(&self) -> Option<AuthWallet> {
        None
    }

    async fn set_auth_wallet(&self, _wallet: Option<AuthWallet>) {}
}

async fn test_wallet(connector: DirectConnector) -> Wallet {
    let db = Arc::new(cdk_sqlite::wallet::memory::empty().await.unwrap());
    let seed = Mnemonic::generate(12).unwrap().to_seed_normalized("");

    WalletBuilder::new()
        .mint_url(MintUrl::from_str("https://test-mint").unwrap())
        .unit(CurrencyUnit::Sat)
        .localstore(db)
        .seed(seed)
        .client(connector)
        .build()
        .unwrap()
}

/// The wallet signs its own lookup challenge, the mint verifies it, and the wallet gets back
/// the quote it locked to its own key - the full round trip a reconciling caller relies on.
/// The lookup also stores the quote locally with its signing key stamped, since callers like
/// the sweeper rely on this method to populate the wallet database, not just report results.
#[tokio::test]
async fn wallet_looks_up_its_own_nut20_locked_quote() {
    let mint = test_mint().await;
    let secret_key = SecretKey::generate();
    locked_quote(&mint, secret_key.public_key()).await;

    let wallet = test_wallet(DirectConnector(mint)).await;

    let quotes = wallet
        .fetch_mint_quotes_by_pubkey(std::slice::from_ref(&secret_key))
        .await
        .expect("lookup should succeed");

    assert_eq!(quotes.len(), 1);
    assert_eq!(
        quotes[0].payment_method,
        PaymentMethod::Known(KnownMethod::Bolt11)
    );
    assert_eq!(quotes[0].secret_key, Some(secret_key));

    // The lookup must have persisted the quote, not just returned it in memory.
    let stored = wallet
        .localstore
        .get_mint_quote(&quotes[0].id)
        .await
        .expect("localstore read")
        .expect("quote should be stored locally after lookup");
    assert_eq!(stored, quotes[0]);
}

/// A key with no locked quotes gets back an empty list, not an error - the mint's signature
/// check passes (the wallet signed correctly) and simply finds nothing for that key.
#[tokio::test]
async fn wallet_lookup_is_empty_for_a_key_with_no_quotes() {
    let mint = test_mint().await;
    let wallet = test_wallet(DirectConnector(mint)).await;

    let unrelated_key = SecretKey::generate();
    let quotes = wallet
        .fetch_mint_quotes_by_pubkey(&[unrelated_key])
        .await
        .expect("lookup should succeed");

    assert!(quotes.is_empty());
}
