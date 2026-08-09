#![cfg(test)]
#![allow(missing_docs)]
#![allow(clippy::missing_panics_doc)]

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use bip39::Mnemonic;
use bitcoin::bip32::DerivationPath;
use cdk_common::database::WalletDatabase;
use cdk_common::mint_url::MintUrl;
use cdk_common::nut00::KnownMethod;
use cdk_common::nuts::{
    CurrencyUnit, Id, KeySet, KeySetInfo, Keys, KeysetResponse, MeltMethodSettings, MintInfo,
    MintMethodSettings, MintVersion, MppMethodSettings, Proof, PublicKey, SpendingConditions,
};
use cdk_common::nutxx::MintQuoteByPubkeyRequest;
use cdk_common::wallet::{
    MeltQuote, MintQuote, P2PKSigningKey, ProofInfo, Transaction, TransactionDirection,
    TransactionId, WalletSaga,
};
use cdk_common::{
    Amount, CheckStateRequest, CheckStateResponse, MeltQuoteCreateResponse, MeltQuoteRequest,
    MeltQuoteResponse, MeltRequest, MintQuoteRequest, MintQuoteResponse, MintRequest, MintResponse,
    RestoreRequest, RestoreResponse, SwapRequest, SwapResponse,
};

use crate::nuts::{
    nut17, nut19, BatchCheckMintQuoteRequest, BatchMintRequest, MeltQuoteBolt11Response,
    MeltQuoteState, NUT04Settings, NUT05Settings, PaymentMethod, SecretKey, State,
};
use crate::secret::Secret;
use crate::wallet::{MintConnector, Wallet};
use crate::Error;

/// Create test database
pub async fn create_test_db() -> Arc<dyn WalletDatabase<cdk_common::database::Error> + Send + Sync>
{
    let db = cdk_sqlite::wallet::memory::empty().await.unwrap();
    Arc::new(db)
}

/// Create a test mint URL
pub fn test_mint_url() -> MintUrl {
    MintUrl::from_str("https://test-mint.example.com").unwrap()
}

/// Create a test keyset ID
pub fn test_keyset_id() -> Id {
    Id::from_str("0094d5a774c40a32").unwrap()
}

/// Create a deterministic test keyset
pub fn test_keyset() -> KeySet {
    let keys = [
        (
            1_u64,
            "0331ad6dbac09400338c17d43fc23d70330547ff7416801c7900735b23fdab189b",
        ),
        (
            2_u64,
            "022d000b4bab1f64ba07c940fdc43ecfe45a94cc574023c3578f17ea635addb437",
        ),
        (
            4_u64,
            "028c127bc4981a995e4159e3df696dcde7c2a0c57f78d3b794080b018af43e2da8",
        ),
        (
            8_u64,
            "023b5e4a60d893d78c8bc7c6d415a13f7538be4898c0567823ed725d5893ae7469",
        ),
        (
            16_u64,
            "02c9b87544bac8bb8169f76671b8d8e107feeece3f6fda71a327311fabbd0a6b08",
        ),
        (
            32_u64,
            "02355039afac742268c9af57b0187957dbf6041f05008a9b2b2f4b3eb863ba81ce",
        ),
        (
            64_u64,
            "0280242434791a53cada80e46009df29c0c7b205d7f460a7917fa7a006e472288b",
        ),
        (
            128_u64,
            "03e231001406fbdbc548ca7edb3d4e5bc7c4dea72280c9caec3a75149f3fea5e49",
        ),
        (
            256_u64,
            "0326ebacc443234661fdc93a1854ab9cb52444a176894f8a54fd6051fa9ff0fbc4",
        ),
        (
            512_u64,
            "02a0eaacbe7b5d08ac5d65bc0f88c4622cfebb9295567e41ccef219454f7b3b880",
        ),
        (
            1024_u64,
            "03a2d67143804905764d8779629ab9d89acb7f1b9b03a14b3721d48892df621edd",
        ),
        (
            2048_u64,
            "031bbc93e01d2c3344ac8d424e5145d5c3b8ec1b309c17faf73beeabeca3c40117",
        ),
        (
            4096_u64,
            "0360e34d975f7fd8b1b89473902cc08752fa611d0f995ad18e7328c0b5e8cdd8f0",
        ),
        (
            8192_u64,
            "02d4b46f83d12b4223754498dc5016bdf761292683229c258f8ce687994879d1b6",
        ),
        (
            16384_u64,
            "03afd259181c3fc0ec5f115decb1cb6e2c5322625fd0c1b24c46bab5cf2f93c3d6",
        ),
        (
            32768_u64,
            "0342baac1eb121da5259a9b8c36a7e68b8454008f9b2f674a666af6f27a9686c98",
        ),
        (
            65536_u64,
            "022a3f9627590870309183cd2399fb6fac32a73302d02c39137c5393e4a5b35b84",
        ),
        (
            131072_u64,
            "033e972225097257f30e1251c84997fabaf0e8e91460e43ad821fe9d4b3d995993",
        ),
        (
            262144_u64,
            "02bcb4a4d34251455d2a76dc2489d34b214cc77c6c1c3c18058b9f1f6ab36016f0",
        ),
        (
            524288_u64,
            "03b301ab3e3104023cde110aa255d9588ad90c0f1e74f952bd0f7639344b99aa4a",
        ),
        (
            1048576_u64,
            "03ad341aebe044eae65ef40aaff4e72c2df21995b00ca494542e437be2acf94e73",
        ),
        (
            2097152_u64,
            "027bd960bc40c3e7767ce76ca3e8391bf982d77c1f1a2cadb7a4c2cc2c03d9dba1",
        ),
        (
            4194304_u64,
            "02bb13a47fc78a80654f19b346bbb21bbe145f127b8db7112b8e5192f6b1cdde21",
        ),
        (
            8388608_u64,
            "037461e11411d64aa7801c4b85d6fb6d2f9ca551f87541ba8723352e01c085d79e",
        ),
        (
            16777216_u64,
            "025a953e3a2314277ff4d19f08d0c21f4db9e03c8c8a6ee0ff65d067205861b909",
        ),
        (
            33554432_u64,
            "02bb9f5004ff60b811d3e4b924b299170a51fa3a02531ea2542c6da3911f35b240",
        ),
        (
            67108864_u64,
            "039de60467c8afda2f24589dd2da3b5e5f979b37e2633e38563161556e9526e583",
        ),
        (
            134217728_u64,
            "03eae2673c65f7f64fb4a2257df98d1ec34677a6f8d12cca46baf5d4dfb900c76d",
        ),
        (
            268435456_u64,
            "03bae4a407997ab35177bd824103941c8b008fae6674493f7ec8072f2e1ab86b33",
        ),
        (
            536870912_u64,
            "032264c60dec079b2e7b2354879973f6b299fc013d4731600581a149108a15ad60",
        ),
        (
            1073741824_u64,
            "02c4b9799e907c1c4e096140587a66ace67d50981a71762a60dc9ef82ac55c74cc",
        ),
        (
            2147483648_u64,
            "03b630c364d6e1f40fa9b3bdb455a4863d50a65cb38b7c0c69735fb683eb77d091",
        ),
    ]
    .into_iter()
    .map(|(amount, public_key)| {
        (
            Amount::from(amount),
            crate::nuts::PublicKey::from_hex(public_key).unwrap(),
        )
    })
    .collect::<BTreeMap<_, _>>();

    KeySet {
        id: test_keyset_id(),
        unit: CurrencyUnit::Sat,
        active: Some(true),
        keys: crate::nuts::Keys::new(keys),
        input_fee_ppk: 101,
        final_expiry: None,
    }
}

/// Create test mint info
pub fn test_mint_info() -> MintInfo {
    MintInfo::new()
        .name("cdk-mintd fake mint")
        .pubkey(
            crate::nuts::PublicKey::from_hex(
                "02836c831cfff541ba17fbca32dd0f6a54d06305893cbcb2af0d7143e66a609147",
            )
            .unwrap(),
        )
        .version(MintVersion::new(
            "cdk-mintd".to_string(),
            "0.15.0-rc.2".to_string(),
        ))
        .description("These are not real sats for testing only")
        .long_description("A longer mint for testing")
        .nuts(
            crate::nuts::Nuts::new()
                .nut04(NUT04Settings::new(
                    vec![
                        MintMethodSettings {
                            method: PaymentMethod::Known(KnownMethod::Bolt11),
                            unit: CurrencyUnit::Sat,
                            method_name: None,
                            min_amount: Some(Amount::from(1_u64)),
                            max_amount: Some(Amount::from(500_000_u64)),
                            options: Some(crate::nuts::nut04::MintMethodOptions::Bolt11 {
                                description: true,
                            }),
                        },
                        MintMethodSettings {
                            method: PaymentMethod::Known(KnownMethod::Bolt12),
                            unit: CurrencyUnit::Sat,
                            method_name: None,
                            min_amount: Some(Amount::from(1_u64)),
                            max_amount: Some(Amount::from(500_000_u64)),
                            options: None,
                        },
                    ],
                    false,
                ))
                .nut05(NUT05Settings {
                    methods: vec![
                        MeltMethodSettings {
                            method: PaymentMethod::Known(KnownMethod::Bolt11),
                            unit: CurrencyUnit::Sat,
                            method_name: None,
                            min_amount: Some(Amount::from(1_u64)),
                            max_amount: Some(Amount::from(500_000_u64)),
                            options: None,
                        },
                        MeltMethodSettings {
                            method: PaymentMethod::Known(KnownMethod::Bolt12),
                            unit: CurrencyUnit::Sat,
                            method_name: None,
                            min_amount: Some(Amount::from(1_u64)),
                            max_amount: Some(Amount::from(500_000_u64)),
                            options: None,
                        },
                    ],
                    disabled: false,
                })
                .nut07(true)
                .nut08(true)
                .nut09(true)
                .nut10(true)
                .nut11(true)
                .nut12(true)
                .nut14(true)
                .nut15(vec![MppMethodSettings {
                    method: PaymentMethod::Known(KnownMethod::Bolt11),
                    unit: CurrencyUnit::Sat,
                }])
                .nut17(vec![
                    nut17::SupportedMethods::new(
                        PaymentMethod::Known(KnownMethod::Bolt11),
                        CurrencyUnit::Sat,
                        vec![
                            nut17::WsCommand::Bolt11MintQuote,
                            nut17::WsCommand::Bolt11MeltQuote,
                            nut17::WsCommand::ProofState,
                        ],
                    ),
                    nut17::SupportedMethods::new(
                        PaymentMethod::Known(KnownMethod::Bolt12),
                        CurrencyUnit::Sat,
                        vec![
                            nut17::WsCommand::Bolt12MintQuote,
                            nut17::WsCommand::Bolt12MeltQuote,
                            nut17::WsCommand::ProofState,
                        ],
                    ),
                ])
                .nut19(
                    Some(60),
                    vec![
                        nut19::CachedEndpoint::new(nut19::Method::Post, nut19::Path::Swap),
                        nut19::CachedEndpoint::new(
                            nut19::Method::Post,
                            nut19::Path::Custom("/v1/mint/bolt11".to_string()),
                        ),
                        nut19::CachedEndpoint::new(
                            nut19::Method::Post,
                            nut19::Path::Custom("/v1/melt/bolt11".to_string()),
                        ),
                        nut19::CachedEndpoint::new(
                            nut19::Method::Post,
                            nut19::Path::Custom("/v1/mint/bolt12".to_string()),
                        ),
                        nut19::CachedEndpoint::new(
                            nut19::Method::Post,
                            nut19::Path::Custom("/v1/melt/bolt12".to_string()),
                        ),
                    ],
                )
                .nut20(true),
        )
        .time(1_773_219_614_u64)
}

/// Create a test proof
pub fn test_proof(keyset_id: Id, amount: u64) -> Proof {
    Proof {
        amount: Amount::from(amount),
        keyset_id,
        secret: Secret::generate(),
        c: SecretKey::generate().public_key(),
        witness: None,
        dleq: None,
        p2pk_e: None,
    }
}

/// Create a test proof info in Unspent state
pub fn test_proof_info(
    keyset_id: Id,
    amount: u64,
    mint_url: MintUrl,
) -> cdk_common::wallet::ProofInfo {
    let proof = test_proof(keyset_id, amount);
    cdk_common::wallet::ProofInfo::new(proof, mint_url, State::Unspent, CurrencyUnit::Sat).unwrap()
}

/// Create an inactive keyset with different keys and a properly computed ID.
pub fn make_inactive_keyset() -> KeySet {
    let mut ks = test_keyset();
    ks.active = Some(false);
    // Shift each key amount to produce different keys → different ID
    let shifted_keys: std::collections::BTreeMap<_, _> = ks
        .keys
        .iter()
        .map(|(amount, pk)| (Amount::from(amount.to_u64() * 2), *pk))
        .collect();
    ks.keys = Keys::new(shifted_keys);
    ks.id = Id::v2_from_data(&ks.keys, &ks.unit, ks.input_fee_ppk, ks.final_expiry);
    ks
}

/// Create a test melt quote
pub fn test_melt_quote() -> MeltQuote {
    MeltQuote {
        id: format!("test_melt_quote_{}", uuid::Uuid::new_v4()),
        mint_url: Some(test_mint_url()),
        unit: CurrencyUnit::Sat,
        amount: Amount::from(1000),
        request: "lnbc1000...".to_string(),
        fee_reserve: Amount::from(10),
        state: MeltQuoteState::Unpaid,
        expiry: 9999999999,
        payment_proof: None,
        estimated_blocks: None,
        fee_index: None,
        payment_method: PaymentMethod::Known(KnownMethod::Bolt11),
        used_by_operation: None,
        version: 0,
    }
}

/// Create a test mint quote
pub fn test_mint_quote(mint_url: MintUrl) -> MintQuote {
    MintQuote::new(
        format!("test_mint_quote_{}", uuid::Uuid::new_v4()),
        mint_url,
        PaymentMethod::Known(KnownMethod::Bolt11),
        Some(Amount::from(1000)),
        CurrencyUnit::Sat,
        "lnbc1000...".to_string(),
        9999999999,
        None,
    )
}

/// Create a test wallet
pub async fn create_test_wallet(
    db: Arc<dyn WalletDatabase<cdk_common::database::Error> + Send + Sync>,
) -> Wallet {
    let mint_url = "https://test-mint.example.com";
    let seed = Mnemonic::generate(12).unwrap().to_seed_normalized("");

    Wallet::new(mint_url, CurrencyUnit::Sat, db, seed, None).unwrap()
}

/// Create a test wallet with a mock client
pub async fn create_test_wallet_with_mock(
    db: Arc<dyn WalletDatabase<cdk_common::database::Error> + Send + Sync>,
    mock_client: Arc<MockMintConnector>,
) -> Wallet {
    let seed = Mnemonic::generate(12).unwrap().to_seed_normalized("");

    crate::wallet::WalletBuilder::new()
        .mint_url(test_mint_url())
        .unit(CurrencyUnit::Sat)
        .localstore(db)
        .seed(seed)
        .shared_client(mock_client)
        .build()
        .unwrap()
}

/// Create a test wallet with a mock client that uses HTTP polling for
/// subscriptions instead of WebSocket.
///
/// Useful for exercising subscription-driven flows (e.g. `PendingMelt::wait`)
/// deterministically against the mock connector, which has no WebSocket
/// endpoint.
pub async fn create_test_wallet_with_mock_http_subscription(
    db: Arc<dyn WalletDatabase<cdk_common::database::Error> + Send + Sync>,
    mock_client: Arc<MockMintConnector>,
) -> Wallet {
    let seed = Mnemonic::generate(12).unwrap().to_seed_normalized("");

    crate::wallet::WalletBuilder::new()
        .mint_url(test_mint_url())
        .unit(CurrencyUnit::Sat)
        .localstore(db)
        .seed(seed)
        .shared_client(mock_client)
        .use_http_subscription()
        .build()
        .unwrap()
}

/// Mock MintConnector for testing recovery scenarios
#[derive(Debug)]
pub struct MockMintConnector {
    /// Mock mint keyset state
    pub keysets: Mutex<Vec<KeySet>>,
    /// Mock mint info state
    pub mint_info: Mutex<MintInfo>,
    /// Response for post_check_state calls
    pub check_state_response: Mutex<Option<Result<CheckStateResponse, Error>>>,
    /// Response for post_restore calls
    pub restore_response: Mutex<Option<Result<RestoreResponse, Error>>>,
    /// Response for get_melt_quote_status calls
    pub melt_quote_status_response: Mutex<Option<Result<MeltQuoteBolt11Response<String>, Error>>>,
    /// Queue of responses for successive get_melt_quote_status calls.
    ///
    /// When non-empty, each `get_melt_quote_status` call pops the front entry.
    /// Takes precedence over `melt_quote_status_response`, letting a single
    /// test stage a sequence of status responses (e.g. a subscription event
    /// followed by an authoritative HTTP recheck).
    pub melt_quote_status_responses:
        Mutex<std::collections::VecDeque<Result<MeltQuoteBolt11Response<String>, Error>>>,
    /// Response for post_mint calls
    pub post_mint_response: Mutex<Option<Result<MintResponse, Error>>>,
    /// Queue of responses for successive post_mint calls.
    pub post_mint_responses: Mutex<std::collections::VecDeque<Result<MintResponse, Error>>>,
    /// Captured post_mint requests.
    pub post_mint_requests: Mutex<Vec<(PaymentMethod, MintRequest<String>)>>,
    /// Queue of responses for successive post_batch_mint calls.
    pub post_batch_mint_responses: Mutex<std::collections::VecDeque<Result<MintResponse, Error>>>,
    /// Captured post_batch_mint requests.
    pub post_batch_mint_requests: Mutex<Vec<(PaymentMethod, BatchMintRequest<String>)>>,
    /// Response for post_mint_quote_by_pubkey calls
    pub post_mint_quote_by_pubkey_response:
        Mutex<Option<Result<Vec<MintQuoteResponse<String>>, Error>>>,
    /// Captured post_mint_quote_by_pubkey requests for test verification.
    pub captured_mint_quote_by_pubkey_requests: Mutex<Vec<MintQuoteByPubkeyRequest>>,
    /// Response for post_swap calls
    pub post_swap_response: Mutex<Option<Result<SwapResponse, Error>>>,
    /// Queue of responses for successive post_swap calls.
    ///
    /// When non-empty, each `post_swap` call pops the front entry.
    /// Takes precedence over `post_swap_response`.
    pub post_swap_responses: Mutex<std::collections::VecDeque<Result<SwapResponse, Error>>>,
    /// Captured post_swap requests for test verification.
    pub captured_swap_requests: Mutex<Vec<SwapRequest>>,
    /// Response for post_melt calls
    pub post_melt_response: Mutex<Option<Result<MeltQuoteResponse<String>, Error>>>,
    /// Last post_melt method/request captured by the mock
    pub last_post_melt_request: Mutex<Option<(PaymentMethod, MeltRequest<String>)>>,
    /// Response for LNURL pay request calls
    pub lnurl_pay_request_response:
        Mutex<Option<Result<crate::lightning_address::LnurlPayResponse, Error>>>,
    /// Response for LNURL invoice calls
    pub lnurl_invoice_response:
        Mutex<Option<Result<crate::lightning_address::LnurlPayInvoiceResponse, Error>>>,
    /// Response for DNS TXT resolution calls
    #[cfg(all(feature = "bip353", not(target_arch = "wasm32")))]
    pub dns_txt_response: Mutex<Option<Result<Vec<String>, Error>>>,
    /// Number of times `get_mint_info` has been called.
    pub get_mint_info_calls: Mutex<usize>,
}

impl Default for MockMintConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl MockMintConnector {
    pub fn new() -> Self {
        let keyset = test_keyset();
        let mint_info = test_mint_info();

        Self {
            keysets: Mutex::new(vec![keyset]),
            mint_info: Mutex::new(mint_info),
            check_state_response: Mutex::new(None),
            restore_response: Mutex::new(None),
            melt_quote_status_response: Mutex::new(None),
            melt_quote_status_responses: Mutex::new(std::collections::VecDeque::new()),
            post_mint_response: Mutex::new(None),
            post_mint_responses: Mutex::new(std::collections::VecDeque::new()),
            post_mint_requests: Mutex::new(Vec::new()),
            post_batch_mint_responses: Mutex::new(std::collections::VecDeque::new()),
            post_batch_mint_requests: Mutex::new(Vec::new()),
            post_mint_quote_by_pubkey_response: Mutex::new(None),
            captured_mint_quote_by_pubkey_requests: Mutex::new(Vec::new()),
            post_swap_response: Mutex::new(None),
            post_swap_responses: Mutex::new(std::collections::VecDeque::new()),
            captured_swap_requests: Mutex::new(Vec::new()),
            post_melt_response: Mutex::new(None),
            last_post_melt_request: Mutex::new(None),
            lnurl_pay_request_response: Mutex::new(None),
            lnurl_invoice_response: Mutex::new(None),
            #[cfg(all(feature = "bip353", not(target_arch = "wasm32")))]
            dns_txt_response: Mutex::new(None),
            get_mint_info_calls: Mutex::new(0),
        }
    }

    pub fn set_check_state_response(&self, response: Result<CheckStateResponse, Error>) {
        *self.check_state_response.lock().unwrap() = Some(response);
    }

    pub fn set_mint_keys_response(&self, response: Result<Vec<KeySet>, Error>) {
        match response {
            Ok(keysets) => {
                *self.keysets.lock().unwrap() = keysets;
            }
            Err(_) => unimplemented!("error responses for key state are not supported"),
        }
    }

    pub fn set_mint_keyset_response(&self, response: Result<KeySet, Error>) {
        match response {
            Ok(keyset) => {
                let mut keysets = self.keysets.lock().unwrap();
                if let Some(existing) = keysets.iter_mut().find(|k| k.id == keyset.id) {
                    *existing = keyset;
                } else {
                    keysets.push(keyset);
                }
            }
            Err(_) => unimplemented!("error responses for key state are not supported"),
        }
    }

    pub fn set_mint_keysets_response(&self, response: Result<KeysetResponse, Error>) {
        match response {
            Ok(resp) => {
                let mut keysets = self.keysets.lock().unwrap();
                for info in resp.keysets {
                    if let Some(ks) = keysets.iter_mut().find(|k| k.id == info.id) {
                        ks.unit = info.unit;
                        ks.active = Some(info.active);
                        ks.input_fee_ppk = info.input_fee_ppk;
                        ks.final_expiry = info.final_expiry;
                    }
                }
            }
            Err(_) => unimplemented!("error responses for key state are not supported"),
        }
    }

    pub fn set_mint_info_response(&self, response: Result<MintInfo, Error>) {
        match response {
            Ok(mint_info) => *self.mint_info.lock().unwrap() = mint_info,
            Err(_) => unimplemented!("error responses for mint info state are not supported"),
        }
    }

    pub fn set_mint_quote_by_pubkey_response(
        &self,
        response: Result<Vec<MintQuoteResponse<String>>, Error>,
    ) {
        *self.post_mint_quote_by_pubkey_response.lock().unwrap() = Some(response);
    }

    pub fn set_active_keyset(&self, keyset: KeySet) {
        *self.keysets.lock().unwrap() = vec![keyset];
    }

    pub fn reset_default_mint_state(&self) {
        self.set_active_keyset(test_keyset());
        self.set_mint_info_response(Ok(test_mint_info()));
    }

    pub fn _set_restore_response(&self, response: Result<RestoreResponse, Error>) {
        *self.restore_response.lock().unwrap() = Some(response);
    }

    pub fn set_melt_quote_status_response(
        &self,
        response: Result<MeltQuoteBolt11Response<String>, Error>,
    ) {
        *self.melt_quote_status_response.lock().unwrap() = Some(response);
    }

    /// Enqueue a response for the next `get_melt_quote_status` call.
    ///
    /// Queued responses are consumed in FIFO order and take precedence over
    /// any value set via [`set_melt_quote_status_response`](Self::set_melt_quote_status_response).
    pub fn push_melt_quote_status_response(
        &self,
        response: Result<MeltQuoteBolt11Response<String>, Error>,
    ) {
        self.melt_quote_status_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    pub fn set_post_mint_response(&self, response: Result<MintResponse, Error>) {
        *self.post_mint_response.lock().unwrap() = Some(response);
    }

    /// Enqueue a response for the next `post_mint` call.
    pub fn push_post_mint_response(&self, response: Result<MintResponse, Error>) {
        self.post_mint_responses.lock().unwrap().push_back(response);
    }

    /// Return all captured `post_mint` requests.
    pub fn post_mint_requests(&self) -> Vec<(PaymentMethod, MintRequest<String>)> {
        self.post_mint_requests.lock().unwrap().clone()
    }

    /// Enqueue a response for the next `post_batch_mint` call.
    pub fn push_post_batch_mint_response(&self, response: Result<MintResponse, Error>) {
        self.post_batch_mint_responses
            .lock()
            .unwrap()
            .push_back(response);
    }

    /// Return all captured `post_batch_mint` requests.
    pub fn post_batch_mint_requests(&self) -> Vec<(PaymentMethod, BatchMintRequest<String>)> {
        self.post_batch_mint_requests.lock().unwrap().clone()
    }

    pub fn set_post_swap_response(&self, response: Result<SwapResponse, Error>) {
        *self.post_swap_response.lock().unwrap() = Some(response);
    }

    /// Enqueue a response for the next `post_swap` call.
    pub fn push_post_swap_response(&self, response: Result<SwapResponse, Error>) {
        self.post_swap_responses.lock().unwrap().push_back(response);
    }

    /// Get all captured swap requests.
    pub fn captured_swap_requests(&self) -> Vec<SwapRequest> {
        self.captured_swap_requests.lock().unwrap().clone()
    }

    pub fn set_post_melt_response(&self, response: Result<MeltQuoteResponse<String>, Error>) {
        *self.post_melt_response.lock().unwrap() = Some(response);
    }

    pub fn last_post_melt_request(&self) -> Option<(PaymentMethod, MeltRequest<String>)> {
        self.last_post_melt_request.lock().unwrap().clone()
    }

    pub fn set_lnurl_pay_request_response(
        &self,
        response: Result<crate::lightning_address::LnurlPayResponse, Error>,
    ) {
        *self.lnurl_pay_request_response.lock().unwrap() = Some(response);
    }

    pub fn set_lnurl_invoice_response(
        &self,
        response: Result<crate::lightning_address::LnurlPayInvoiceResponse, Error>,
    ) {
        *self.lnurl_invoice_response.lock().unwrap() = Some(response);
    }

    #[cfg(all(feature = "bip353", not(target_arch = "wasm32")))]
    pub fn set_dns_txt_response(&self, response: Result<Vec<String>, Error>) {
        *self.dns_txt_response.lock().unwrap() = Some(response);
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl MintConnector for MockMintConnector {
    #[cfg(all(feature = "bip353", not(target_arch = "wasm32")))]
    async fn resolve_dns_txt(&self, _domain: &str) -> Result<Vec<String>, Error> {
        self.dns_txt_response
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    async fn fetch_lnurl_pay_request(
        &self,
        _url: &str,
    ) -> Result<crate::lightning_address::LnurlPayResponse, Error> {
        self.lnurl_pay_request_response
            .lock()
            .unwrap()
            .take()
            .expect("MockMintConnector: fetch_lnurl_pay_request called without configured response")
    }

    async fn fetch_lnurl_invoice(
        &self,
        _url: &str,
    ) -> Result<crate::lightning_address::LnurlPayInvoiceResponse, Error> {
        self.lnurl_invoice_response
            .lock()
            .unwrap()
            .take()
            .expect("MockMintConnector: fetch_lnurl_invoice called without configured response")
    }

    async fn get_mint_keys(&self) -> Result<Vec<crate::nuts::KeySet>, Error> {
        Ok(self.keysets.lock().unwrap().clone())
    }

    async fn get_mint_keyset(&self, keyset_id: Id) -> Result<crate::nuts::KeySet, Error> {
        self.keysets
            .lock()
            .unwrap()
            .iter()
            .find(|ks| ks.id == keyset_id)
            .cloned()
            .ok_or(Error::UnknownKeySet)
    }

    async fn get_mint_keysets(&self) -> Result<KeysetResponse, Error> {
        let keysets = self.keysets.lock().unwrap();

        Ok(KeysetResponse {
            keysets: keysets
                .iter()
                .map(|ks| KeySetInfo {
                    id: ks.id,
                    unit: ks.unit.clone(),
                    active: ks.active.unwrap_or(true),
                    input_fee_ppk: ks.input_fee_ppk,
                    final_expiry: ks.final_expiry,
                })
                .collect(),
        })
    }

    async fn post_mint_quote(
        &self,
        _request: MintQuoteRequest,
    ) -> Result<MintQuoteResponse<String>, Error> {
        unimplemented!()
    }

    async fn get_mint_quote_status(
        &self,
        _method: PaymentMethod,
        _quote_id: &str,
    ) -> Result<MintQuoteResponse<String>, Error> {
        unimplemented!()
    }

    async fn post_mint_quote_by_pubkey(
        &self,
        request: MintQuoteByPubkeyRequest,
    ) -> Result<Vec<MintQuoteResponse<String>>, Error> {
        self.captured_mint_quote_by_pubkey_requests
            .lock()
            .unwrap()
            .push(request);

        self.post_mint_quote_by_pubkey_response
            .lock()
            .unwrap()
            .take()
            .expect(
                "MockMintConnector: post_mint_quote_by_pubkey called without configured response",
            )
    }

    async fn post_mint(
        &self,
        method: &PaymentMethod,
        request: MintRequest<String>,
    ) -> Result<MintResponse, Error> {
        self.post_mint_requests
            .lock()
            .unwrap()
            .push((method.clone(), request));

        let queued = self.post_mint_responses.lock().unwrap().pop_front();
        match queued {
            Some(response) => response,
            None => self
                .post_mint_response
                .lock()
                .unwrap()
                .take()
                .expect("MockMintConnector: post_mint called without configured response"),
        }
    }

    async fn post_melt_quote(
        &self,
        _request: MeltQuoteRequest,
    ) -> Result<MeltQuoteCreateResponse<String>, Error> {
        unimplemented!()
    }

    async fn get_melt_quote_status(
        &self,
        _method: PaymentMethod,
        _quote_id: &str,
    ) -> Result<MeltQuoteResponse<String>, Error> {
        let queued = self.melt_quote_status_responses.lock().unwrap().pop_front();
        let response = match queued {
            Some(response) => response,
            None => self
                .melt_quote_status_response
                .lock()
                .unwrap()
                .take()
                .expect(
                    "MockMintConnector: get_melt_quote_status called without configured response",
                ),
        }?;
        Ok(MeltQuoteResponse::Bolt11(response))
    }

    async fn post_swap(&self, request: SwapRequest) -> Result<SwapResponse, Error> {
        self.captured_swap_requests.lock().unwrap().push(request);
        let queued = self.post_swap_responses.lock().unwrap().pop_front();
        match queued {
            Some(response) => response,
            None => self
                .post_swap_response
                .lock()
                .unwrap()
                .take()
                .expect("MockMintConnector: post_swap called without configured response"),
        }
    }

    async fn get_mint_info(&self) -> Result<crate::nuts::MintInfo, Error> {
        *self.get_mint_info_calls.lock().unwrap() += 1;
        Ok(self.mint_info.lock().unwrap().clone())
    }

    async fn post_check_state(
        &self,
        _request: CheckStateRequest,
    ) -> Result<CheckStateResponse, Error> {
        self.check_state_response
            .lock()
            .unwrap()
            .take()
            .expect("MockMintConnector: post_check_state called without configured response")
    }

    async fn post_restore(&self, _request: RestoreRequest) -> Result<RestoreResponse, Error> {
        self.restore_response
            .lock()
            .unwrap()
            .take()
            .expect("MockMintConnector: post_restore called without configured response")
    }

    async fn get_auth_wallet(&self) -> Option<crate::wallet::AuthWallet> {
        None
    }

    async fn set_auth_wallet(&self, _wallet: Option<crate::wallet::AuthWallet>) {}

    async fn post_melt(
        &self,
        method: &PaymentMethod,
        request: MeltRequest<String>,
    ) -> Result<MeltQuoteResponse<String>, Error> {
        *self.last_post_melt_request.lock().unwrap() = Some((method.clone(), request));
        self.post_melt_response
            .lock()
            .unwrap()
            .take()
            .expect("MockMintConnector: post_melt called without configured response")
    }

    async fn post_batch_check_mint_quote_status(
        &self,
        _method: &PaymentMethod,
        _request: BatchCheckMintQuoteRequest<String>,
    ) -> Result<Vec<MintQuoteResponse<String>>, Error> {
        unimplemented!()
    }

    async fn post_batch_mint(
        &self,
        method: &PaymentMethod,
        request: BatchMintRequest<String>,
    ) -> Result<MintResponse, Error> {
        self.post_batch_mint_requests
            .lock()
            .unwrap()
            .push((method.clone(), request));

        self.post_batch_mint_responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("MockMintConnector: post_batch_mint called without configured response")
    }
}

/// Test-only [`WalletDatabase`] wrapper that forwards every call to an inner database unchanged,
/// while counting how many times `add_mint_quote` is called.
///
/// This exists to give change-guarded write paths (e.g.
/// `Wallet::fetch_mint_quotes_by_pubkey`'s guard against rewriting an unchanged mint response) a
/// real regression witness: a test built only on the *value* stored is satisfied whether or not
/// the guard exists, since an unconditional overwrite with the same data is invisible from the
/// outside. Counting the underlying write call closes that gap.
///
/// `WalletDatabase` is a large (~50-method) trait, so this is a mechanical, one-time delegation
/// cost - the same trade `get_mint_info_calls` already makes on [`MockMintConnector`] above, just
/// for the storage side instead of the connector side.
#[derive(Debug)]
pub struct CountingWalletDb {
    inner: Arc<dyn WalletDatabase<cdk_common::database::Error> + Send + Sync>,
    /// Number of times `add_mint_quote` has been called.
    add_mint_quote_calls: AtomicUsize,
}

impl CountingWalletDb {
    /// Wrap `inner`, starting the `add_mint_quote` counter at zero.
    pub fn new(inner: Arc<dyn WalletDatabase<cdk_common::database::Error> + Send + Sync>) -> Self {
        Self {
            inner,
            add_mint_quote_calls: AtomicUsize::new(0),
        }
    }

    /// Number of `add_mint_quote` calls observed so far.
    pub fn add_mint_quote_calls(&self) -> usize {
        self.add_mint_quote_calls.load(Ordering::SeqCst)
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl WalletDatabase<cdk_common::database::Error> for CountingWalletDb {
    async fn get_mint(
        &self,
        mint_url: MintUrl,
    ) -> Result<Option<MintInfo>, cdk_common::database::Error> {
        self.inner.get_mint(mint_url).await
    }

    async fn get_mints(
        &self,
    ) -> Result<std::collections::HashMap<MintUrl, Option<MintInfo>>, cdk_common::database::Error>
    {
        self.inner.get_mints().await
    }

    async fn get_mint_keysets(
        &self,
        mint_url: MintUrl,
    ) -> Result<Option<Vec<KeySetInfo>>, cdk_common::database::Error> {
        self.inner.get_mint_keysets(mint_url).await
    }

    async fn get_keyset_by_id(
        &self,
        keyset_id: &Id,
    ) -> Result<Option<KeySetInfo>, cdk_common::database::Error> {
        self.inner.get_keyset_by_id(keyset_id).await
    }

    async fn get_mint_quote(
        &self,
        quote_id: &str,
    ) -> Result<Option<MintQuote>, cdk_common::database::Error> {
        self.inner.get_mint_quote(quote_id).await
    }

    async fn get_mint_quotes(&self) -> Result<Vec<MintQuote>, cdk_common::database::Error> {
        self.inner.get_mint_quotes().await
    }

    async fn get_unissued_mint_quotes(
        &self,
    ) -> Result<Vec<MintQuote>, cdk_common::database::Error> {
        self.inner.get_unissued_mint_quotes().await
    }

    async fn get_melt_quote(
        &self,
        quote_id: &str,
    ) -> Result<Option<MeltQuote>, cdk_common::database::Error> {
        self.inner.get_melt_quote(quote_id).await
    }

    async fn get_melt_quotes(&self) -> Result<Vec<MeltQuote>, cdk_common::database::Error> {
        self.inner.get_melt_quotes().await
    }

    async fn get_keys(&self, id: &Id) -> Result<Option<Keys>, cdk_common::database::Error> {
        self.inner.get_keys(id).await
    }

    async fn get_proofs(
        &self,
        mint_url: Option<MintUrl>,
        unit: Option<CurrencyUnit>,
        state: Option<Vec<State>>,
        spending_conditions: Option<Vec<SpendingConditions>>,
    ) -> Result<Vec<ProofInfo>, cdk_common::database::Error> {
        self.inner
            .get_proofs(mint_url, unit, state, spending_conditions)
            .await
    }

    async fn get_proofs_by_ys(
        &self,
        ys: Vec<PublicKey>,
    ) -> Result<Vec<ProofInfo>, cdk_common::database::Error> {
        self.inner.get_proofs_by_ys(ys).await
    }

    async fn get_balance(
        &self,
        mint_url: Option<MintUrl>,
        unit: Option<CurrencyUnit>,
        state: Option<Vec<State>>,
    ) -> Result<u64, cdk_common::database::Error> {
        self.inner.get_balance(mint_url, unit, state).await
    }

    async fn get_transaction(
        &self,
        transaction_id: TransactionId,
    ) -> Result<Option<Transaction>, cdk_common::database::Error> {
        self.inner.get_transaction(transaction_id).await
    }

    async fn list_transactions(
        &self,
        mint_url: Option<MintUrl>,
        direction: Option<TransactionDirection>,
        unit: Option<CurrencyUnit>,
    ) -> Result<Vec<Transaction>, cdk_common::database::Error> {
        self.inner
            .list_transactions(mint_url, direction, unit)
            .await
    }

    async fn update_proofs(
        &self,
        added: Vec<ProofInfo>,
        removed_ys: Vec<PublicKey>,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.update_proofs(added, removed_ys).await
    }

    async fn update_proofs_state(
        &self,
        ys: Vec<PublicKey>,
        state: State,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.update_proofs_state(ys, state).await
    }

    async fn add_transaction(
        &self,
        transaction: Transaction,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.add_transaction(transaction).await
    }

    async fn update_mint_url(
        &self,
        old_mint_url: MintUrl,
        new_mint_url: MintUrl,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.update_mint_url(old_mint_url, new_mint_url).await
    }

    async fn increment_keyset_counter(
        &self,
        keyset_id: &Id,
        count: u32,
    ) -> Result<u32, cdk_common::database::Error> {
        self.inner.increment_keyset_counter(keyset_id, count).await
    }

    async fn add_mint(
        &self,
        mint_url: MintUrl,
        mint_info: Option<MintInfo>,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.add_mint(mint_url, mint_info).await
    }

    async fn remove_mint(&self, mint_url: MintUrl) -> Result<(), cdk_common::database::Error> {
        self.inner.remove_mint(mint_url).await
    }

    async fn add_mint_keysets(
        &self,
        mint_url: MintUrl,
        keysets: Vec<KeySetInfo>,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.add_mint_keysets(mint_url, keysets).await
    }

    /// The counted call: increments before delegating, so the count reflects calls attempted
    /// even if the inner database goes on to fail.
    async fn add_mint_quote(&self, quote: MintQuote) -> Result<(), cdk_common::database::Error> {
        self.add_mint_quote_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.add_mint_quote(quote).await
    }

    async fn remove_mint_quote(&self, quote_id: &str) -> Result<(), cdk_common::database::Error> {
        self.inner.remove_mint_quote(quote_id).await
    }

    async fn add_melt_quote(&self, quote: MeltQuote) -> Result<(), cdk_common::database::Error> {
        self.inner.add_melt_quote(quote).await
    }

    async fn remove_melt_quote(&self, quote_id: &str) -> Result<(), cdk_common::database::Error> {
        self.inner.remove_melt_quote(quote_id).await
    }

    async fn add_keys(&self, keyset: KeySet) -> Result<(), cdk_common::database::Error> {
        self.inner.add_keys(keyset).await
    }

    async fn remove_keys(&self, id: &Id) -> Result<(), cdk_common::database::Error> {
        self.inner.remove_keys(id).await
    }

    async fn remove_transaction(
        &self,
        transaction_id: TransactionId,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.remove_transaction(transaction_id).await
    }

    async fn add_saga(&self, saga: WalletSaga) -> Result<(), cdk_common::database::Error> {
        self.inner.add_saga(saga).await
    }

    async fn get_saga(
        &self,
        id: &uuid::Uuid,
    ) -> Result<Option<WalletSaga>, cdk_common::database::Error> {
        self.inner.get_saga(id).await
    }

    async fn update_saga(&self, saga: WalletSaga) -> Result<bool, cdk_common::database::Error> {
        self.inner.update_saga(saga).await
    }

    async fn delete_saga(&self, id: &uuid::Uuid) -> Result<(), cdk_common::database::Error> {
        self.inner.delete_saga(id).await
    }

    async fn get_incomplete_sagas(&self) -> Result<Vec<WalletSaga>, cdk_common::database::Error> {
        self.inner.get_incomplete_sagas().await
    }

    async fn reserve_proofs(
        &self,
        ys: Vec<PublicKey>,
        operation_id: &uuid::Uuid,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.reserve_proofs(ys, operation_id).await
    }

    async fn release_proofs(
        &self,
        operation_id: &uuid::Uuid,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.release_proofs(operation_id).await
    }

    async fn get_reserved_proofs(
        &self,
        operation_id: &uuid::Uuid,
    ) -> Result<Vec<ProofInfo>, cdk_common::database::Error> {
        self.inner.get_reserved_proofs(operation_id).await
    }

    async fn reserve_melt_quote(
        &self,
        quote_id: &str,
        operation_id: &uuid::Uuid,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.reserve_melt_quote(quote_id, operation_id).await
    }

    async fn release_melt_quote(
        &self,
        operation_id: &uuid::Uuid,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.release_melt_quote(operation_id).await
    }

    async fn reserve_mint_quote(
        &self,
        quote_id: &str,
        operation_id: &uuid::Uuid,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.reserve_mint_quote(quote_id, operation_id).await
    }

    async fn release_mint_quote(
        &self,
        operation_id: &uuid::Uuid,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner.release_mint_quote(operation_id).await
    }

    async fn kv_read(
        &self,
        primary_namespace: &str,
        secondary_namespace: &str,
        key: &str,
    ) -> Result<Option<Vec<u8>>, cdk_common::database::Error> {
        self.inner
            .kv_read(primary_namespace, secondary_namespace, key)
            .await
    }

    async fn kv_list(
        &self,
        primary_namespace: &str,
        secondary_namespace: &str,
    ) -> Result<Vec<String>, cdk_common::database::Error> {
        self.inner
            .kv_list(primary_namespace, secondary_namespace)
            .await
    }

    async fn kv_write(
        &self,
        primary_namespace: &str,
        secondary_namespace: &str,
        key: &str,
        value: &[u8],
    ) -> Result<(), cdk_common::database::Error> {
        self.inner
            .kv_write(primary_namespace, secondary_namespace, key, value)
            .await
    }

    async fn kv_remove(
        &self,
        primary_namespace: &str,
        secondary_namespace: &str,
        key: &str,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner
            .kv_remove(primary_namespace, secondary_namespace, key)
            .await
    }

    async fn add_p2pk_key(
        &self,
        pubkey: &PublicKey,
        derivation_path: DerivationPath,
        derivation_index: u32,
    ) -> Result<(), cdk_common::database::Error> {
        self.inner
            .add_p2pk_key(pubkey, derivation_path, derivation_index)
            .await
    }

    async fn get_p2pk_key(
        &self,
        pubkey: &PublicKey,
    ) -> Result<Option<P2PKSigningKey>, cdk_common::database::Error> {
        self.inner.get_p2pk_key(pubkey).await
    }

    async fn list_p2pk_keys(&self) -> Result<Vec<P2PKSigningKey>, cdk_common::database::Error> {
        self.inner.list_p2pk_keys().await
    }

    async fn latest_p2pk(&self) -> Result<Option<P2PKSigningKey>, cdk_common::database::Error> {
        self.inner.latest_p2pk().await
    }
}
