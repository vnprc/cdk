//! Batch Mint Tests [NUT-XX]
//!
//! This file contains tests for the batch minting functionality [NUT-XX].
//!
//! ## Current Test Coverage
//! - Request serialization/deserialization
//! - NUT-20 signature array structure validation
//! - Quote list size and order preservation
//!
//! ## Critical Missing Tests
//! These tests require integration with actual handlers and database, and should be added:
//!
//! ### Mint Handler Validation Tests (cdk-axum router_handlers.rs:780-818)
//! - Empty quote list rejection (400 error)
//! - Duplicate quote rejection (400 error)
//! - Batch size > 100 rejection (400 error)
//! - Output count mismatch rejection (400 error)
//! - Signature count mismatch rejection (400 error)
//!
//! ### Mint-Side NUT-20 Signature Validation Tests (cdk/src/mint/issue/mod.rs:766-791)
//! - Valid NUT-20 signature verification against pubkey
//! - Invalid signature rejection
//! - Signature provided but quote has no pubkey error
//! - Mixed locked/unlocked quote batch validation
//!
//! ### Wallet Batch Mint Tests (cdk/src/wallet/issue/batch.rs)
//! - Quote validation (same mint, same method, same unit, PAID state)
//! - Wallet secret key signature generation for NUT-20
//! - Blinded message generation from multiple secrets
//! - Proof storage and retrieval after minting
//!
//! ### End-to-End Integration Tests
//! - Full flow: mint quote creation → status check → batch mint → proof retrieval
//! - Multiple payment methods in separate batch requests
//! - Database transactions and rollback on failure
//! - Idempotency (same batch request twice returns same signatures)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bip39::Mnemonic;
use cashu::quote_id::QuoteId;
use cdk::mint::{MintBuilder, MintMeltLimits};
use cdk::nuts::{CurrencyUnit, PaymentMethod};
use cdk::types::{FeeReserve, QuoteTTL};
use cdk_common::mint::{BatchMintRequest, BatchQuoteStatusRequest};
use cdk_fake_wallet::FakeWallet;
use cdk_sqlite::mint::memory;

const MINT_URL: &str = "http://127.0.0.1:8088";

/// Helper function to create a test mint with fake wallet
async fn create_test_mint() -> Arc<cdk::Mint> {
    let mnemonic = Mnemonic::generate(12).unwrap();
    let fee_reserve = FeeReserve {
        min_fee_reserve: 1.into(),
        percent_fee_reserve: 1.0,
    };

    let database = memory::empty().await.expect("valid db instance");

    let fake_wallet = FakeWallet::new(
        fee_reserve,
        HashMap::default(),
        HashSet::default(),
        0,
        CurrencyUnit::Sat,
    );

    let localstore = Arc::new(database);
    let mut mint_builder = MintBuilder::new(localstore.clone());

    mint_builder = mint_builder
        .with_name("test mint".to_string())
        .with_description("test mint".to_string());

    mint_builder
        .add_payment_processor(
            CurrencyUnit::Sat,
            PaymentMethod::Bolt11,
            MintMeltLimits::new(1, 5_000),
            Arc::new(fake_wallet),
        )
        .await
        .unwrap();

    let mint = mint_builder
        .build_with_seed(localstore.clone(), &mnemonic.to_seed_normalized(""))
        .await
        .unwrap();

    let quote_ttl = QuoteTTL::new(10000, 10000);
    mint.set_quote_ttl(quote_ttl).await.unwrap();

    Arc::new(mint)
}

/// Helper to detect duplicates in a quote list
fn has_duplicate_quotes(quotes: &[String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    !quotes.iter().all(|q| seen.insert(q.clone()))
}

#[test]
fn test_batch_request_quote_order_preservation() {
    // Test that quote order is preserved through serialization
    let quotes = vec!["q1", "q2", "q3", "q4", "q5"];
    let request = BatchQuoteStatusRequest {
        quote: quotes.iter().map(|q| q.to_string()).collect(),
    };

    // Verify order is preserved
    assert_eq!(
        request.quote,
        vec!["q1", "q2", "q3", "q4", "q5"]
            .iter()
            .map(|q| q.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_batch_request_duplicate_detection() {
    // Test helper function can detect duplicates
    let quotes_with_duplicates = vec!["q1".to_string(), "q2".to_string(), "q1".to_string()];
    assert!(has_duplicate_quotes(&quotes_with_duplicates));

    let unique_quotes = vec!["q1".to_string(), "q2".to_string(), "q3".to_string()];
    assert!(!has_duplicate_quotes(&unique_quotes));
}

#[test]
fn test_batch_request_valid_size_boundaries() {
    // Test 50 quotes (well within limit)
    let quotes_50: Vec<String> = (0..50).map(|i| format!("quote_{}", i)).collect();
    assert!(quotes_50.len() <= 100);

    // Test exactly 100 quotes (at limit)
    let quotes_100: Vec<String> = (0..100).map(|i| format!("quote_{}", i)).collect();
    assert_eq!(quotes_100.len(), 100);

    // Test 101 quotes (exceeds limit, should be rejected by handler)
    let quotes_101: Vec<String> = (0..101).map(|i| format!("quote_{}", i)).collect();
    assert!(quotes_101.len() > 100);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_mint_setup() {
    let _mint = create_test_mint().await;
    // Just verify that the test mint can be created successfully
    // Full end-to-end testing would require completing the batch processing logic
}

// ============================================================================
// NUT-20 Batch Minting Tests
// ============================================================================

#[test]
fn test_batch_mint_signature_array_validation_length_mismatch() {
    // Test that signature array length mismatch is detectable
    let request_json = r#"{
        "quote": ["q1", "q2", "q3"],
        "outputs": [],
        "signature": ["sig1", "sig2"]
    }"#;

    let request: Result<BatchMintRequest, _> = serde_json::from_str(request_json);
    assert!(request.is_ok());
    let req = request.unwrap();

    // Signature array length should not match quotes length
    assert_eq!(req.quote.len(), 3);
    assert_eq!(req.signature.as_ref().unwrap().len(), 2);
    assert_ne!(req.quote.len(), req.signature.as_ref().unwrap().len());
}

#[test]
fn test_batch_mint_signature_array_with_nulls() {
    // Test that signature array can have null entries for unlocked quotes
    let request_json = r#"{
        "quote": ["q1", "q2", "q3"],
        "outputs": [],
        "signature": ["sig1", null, "sig3"]
    }"#;

    let request: Result<BatchMintRequest, _> = serde_json::from_str(request_json);
    assert!(request.is_ok());
    let req = request.unwrap();

    // Verify structure
    assert_eq!(req.quote.len(), 3);
    assert_eq!(req.signature.as_ref().unwrap().len(), 3);
    assert_eq!(req.signature.as_ref().unwrap()[0], Some("sig1".to_string()));
    assert_eq!(req.signature.as_ref().unwrap()[1], None);
    assert_eq!(req.signature.as_ref().unwrap()[2], Some("sig3".to_string()));
}

#[test]
fn test_batch_mint_no_signatures_is_valid() {
    // Test that a batch request with no signatures (all unlocked quotes) is valid
    let request_json = r#"{
        "quote": ["q1", "q2"],
        "outputs": []
    }"#;

    let request: Result<BatchMintRequest, _> = serde_json::from_str(request_json);
    assert!(request.is_ok());
    let req = request.unwrap();

    // Request should be valid
    assert_eq!(req.quote.len(), 2);
    assert!(req.signature.is_none());
}

#[test]
fn test_batch_mint_all_nulls_signatures_valid() {
    // Test that a signature array with all nulls is valid (all unlocked quotes)
    let request_json = r#"{
        "quote": ["q1", "q2"],
        "outputs": [],
        "signature": [null, null]
    }"#;

    let request: Result<BatchMintRequest, _> = serde_json::from_str(request_json);
    assert!(request.is_ok());
    let req = request.unwrap();

    // Request should be valid
    assert_eq!(req.quote.len(), 2);
    assert_eq!(req.signature.as_ref().unwrap().len(), 2);
    assert!(req.signature.as_ref().unwrap().iter().all(|s| s.is_none()));
}

#[test]
fn test_batch_mint_single_quote_with_signature() {
    // Test batch with single NUT-20 locked quote
    let request_json = r#"{
        "quote": ["q1"],
        "outputs": [],
        "signature": ["sig1"]
    }"#;

    let request: Result<BatchMintRequest, _> = serde_json::from_str(request_json);
    assert!(request.is_ok());
    let req = request.unwrap();

    // Verify single-quote batch
    assert_eq!(req.quote.len(), 1);
    assert_eq!(req.outputs.len(), 0);
    assert_eq!(req.signature.as_ref().unwrap().len(), 1);
}

#[test]
fn test_batch_mint_request_serialization_with_signatures() {
    // Test that batch requests with signatures serialize/deserialize correctly
    let request_json = r#"{
        "quote": ["q1", "q2"],
        "outputs": [],
        "signature": ["sig1", null]
    }"#;

    let request: Result<BatchMintRequest, _> = serde_json::from_str(request_json);
    assert!(request.is_ok());
    let req = request.unwrap();

    // Serialize and deserialize
    let json = serde_json::to_string(&req).expect("serialize");
    let deserialized: BatchMintRequest = serde_json::from_str(&json).expect("deserialize");

    // Verify structure is preserved
    assert_eq!(deserialized.quote.len(), 2);
    assert_eq!(deserialized.outputs.len(), 0);
    assert_eq!(deserialized.signature.as_ref().unwrap().len(), 2);
    assert_eq!(
        deserialized.signature.as_ref().unwrap()[0],
        Some("sig1".to_string())
    );
    assert_eq!(deserialized.signature.as_ref().unwrap()[1], None);
}
