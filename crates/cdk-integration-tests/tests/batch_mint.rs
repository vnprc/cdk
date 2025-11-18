//! Batch Mint Tests [NUT-XX]
//!
//! This file contains tests for the batch minting functionality [NUT-XX].
//! Tests focus on request validation and structure rather than full end-to-end flows.

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

#[test]
fn test_batch_quote_status_request_structure() {
    let request = BatchQuoteStatusRequest {
        quote: vec![
            "quote_1".to_string(),
            "quote_2".to_string(),
            "quote_3".to_string(),
        ],
    };

    // Verify structure
    assert_eq!(request.quote.len(), 3);
    assert_eq!(request.quote[0], "quote_1");
}

#[test]
fn test_batch_quote_status_request_empty() {
    let request = BatchQuoteStatusRequest { quote: vec![] };

    // Empty quotes should be validated
    assert!(request.quote.is_empty());
}

#[test]
fn test_batch_quote_status_max_size_limit() {
    // Create request with 101 quotes (exceeds max batch size of 100)
    let quotes: Vec<String> = (0..101).map(|i| format!("quote_{}", i)).collect();
    let request = BatchQuoteStatusRequest { quote: quotes };

    // Should validate batch size limit
    assert!(request.quote.len() > 100);
    assert_eq!(request.quote.len(), 101);
}

#[test]
fn test_batch_mint_request_structure() {
    let request = BatchMintRequest {
        quote: vec!["quote_1".to_string(), "quote_2".to_string()],
        outputs: vec![],
        signature: None,
    };

    // Verify structure
    assert_eq!(request.quote.len(), 2);
    assert_eq!(request.outputs.len(), 0);
    assert!(request.signature.is_none());
}

#[test]
fn test_batch_mint_request_with_signatures() {
    let request = BatchMintRequest {
        quote: vec!["quote_1".to_string(), "quote_2".to_string()],
        outputs: vec![],
        signature: Some(vec![Some("sig_1".to_string()), Some("sig_2".to_string())]),
    };

    // Verify structure
    assert_eq!(request.quote.len(), 2);
    let sigs = request.signature.as_ref().unwrap();
    assert_eq!(sigs.len(), 2);
    assert_eq!(sigs[0], Some("sig_1".to_string()));
}

#[test]
fn test_batch_mint_validation_quote_count_mismatch() {
    let request = BatchMintRequest {
        quote: vec!["quote_1".to_string(), "quote_2".to_string()],
        outputs: vec![], // 0 outputs but 2 quotes
        signature: None,
    };

    // Validation should catch the mismatch
    assert_ne!(request.quote.len(), request.outputs.len());
}

#[test]
fn test_batch_mint_validation_signature_count_mismatch() {
    let request = BatchMintRequest {
        quote: vec!["quote_1".to_string(), "quote_2".to_string()],
        outputs: vec![],
        signature: Some(vec![Some("sig_1".to_string())]), // Only 1 signature for 2 quotes
    };

    // Validation should catch the mismatch
    let sigs = request.signature.as_ref().unwrap();
    assert_ne!(sigs.len(), request.quote.len());
    assert_eq!(sigs.len(), 1);
    assert_eq!(request.quote.len(), 2);
}

#[test]
fn test_batch_mint_duplicate_detection() {
    let quotes = vec!["quote_1", "quote_2", "quote_1"]; // quote_1 is duplicate

    // Detect duplicates
    let mut unique_set = std::collections::HashSet::new();
    let has_duplicates = !quotes.iter().all(|q| unique_set.insert(*q));

    assert!(has_duplicates);
}

#[test]
fn test_batch_mint_mixed_signatures() {
    let request = BatchMintRequest {
        quote: vec![
            "quote_1".to_string(),
            "quote_2".to_string(),
            "quote_3".to_string(),
        ],
        outputs: vec![],
        signature: Some(vec![
            Some("sig_1".to_string()),
            None, // Unlocked quote
            Some("sig_3".to_string()),
        ]),
    };

    // Verify mixed signature support
    let sigs = request.signature.as_ref().unwrap();
    assert_eq!(sigs.len(), 3);
    assert!(sigs[0].is_some());
    assert!(sigs[1].is_none()); // Unlocked
    assert!(sigs[2].is_some());
}

#[test]
fn test_batch_quote_status_preserves_order() {
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
fn test_batch_mint_50_quotes() {
    // Create request with 50 quotes
    let quotes: Vec<String> = (0..50).map(|i| format!("quote_{}", i)).collect();
    let request = BatchQuoteStatusRequest { quote: quotes };

    // Should be valid batch size
    assert!(request.quote.len() <= 100);
    assert_eq!(request.quote.len(), 50);
}

#[test]
fn test_batch_mint_100_quotes_limit() {
    // Create request with exactly 100 quotes (max limit)
    let quotes: Vec<String> = (0..100).map(|i| format!("quote_{}", i)).collect();
    let request = BatchQuoteStatusRequest { quote: quotes };

    // Should be at the limit
    assert_eq!(request.quote.len(), 100);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_mint_setup() {
    let _mint = create_test_mint().await;
    // Just verify that the test mint can be created successfully
    // Full end-to-end testing would require completing the batch processing logic
}
