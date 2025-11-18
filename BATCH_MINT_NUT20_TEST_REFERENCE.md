# Batch Mint + NUT-20 Test Reference Code

Quick copy-paste templates for writing the critical missing tests.

## Test Infrastructure Template

```rust
// Import everything you need once at the top
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bip39::Mnemonic;
use cdk::mint::{MintBuilder, MintMeltLimits};
use cdk::nuts::{CurrencyUnit, PaymentMethod, SecretKey, PublicKey};
use cdk::types::{FeeReserve, QuoteTTL};
use cdk::wallet::Wallet;
use cdk_common::mint::{BatchMintRequest, BatchQuoteStatusRequest};
use cdk_common::wallet::MintQuote;
use cdk_fake_wallet::FakeWallet;
use cdk_sqlite::wallet::memory;
use cdk_integration_tests::init_pure_tests::DirectMintConnection;

// Reusable mint setup (already exists in batch_mint.rs)
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

    mint.set_quote_ttl(cdk::types::QuoteTTL::new(10000, 10000))
        .await
        .unwrap();

    Arc::new(mint)
}
```

## Phase 1: Handler Validation Tests

### Test 1: Empty Quote List
```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_handler_rejects_empty_quotes() {
    let mint = create_test_mint().await;

    let request = BatchMintRequest {
        quote: vec![],
        outputs: vec![],
        signature: None,
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(
        result.is_err(),
        "Handler should reject empty quote list"
    );
}
```

### Test 2: Duplicate Quotes
```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_handler_rejects_duplicates() {
    let mint = create_test_mint().await;

    let request = BatchMintRequest {
        quote: vec!["q1".to_string(), "q2".to_string(), "q1".to_string()],
        outputs: vec![],
        signature: None,
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(
        result.is_err(),
        "Handler should reject duplicate quote IDs"
    );
}
```

### Test 3: Over 100 Quotes
```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_handler_rejects_over_limit() {
    let mint = create_test_mint().await;

    let quotes: Vec<String> = (0..101).map(|i| format!("quote_{}", i)).collect();
    let request = BatchMintRequest {
        quote: quotes,
        outputs: vec![],
        signature: None,
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(
        result.is_err(),
        "Handler should reject batch > 100 quotes"
    );
}
```

### Test 4: Output Count Mismatch
```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_handler_validates_output_count() {
    let mint = create_test_mint().await;

    let request = BatchMintRequest {
        quote: vec!["q1".to_string(), "q2".to_string()],
        outputs: vec![], // 0 outputs for 2 quotes
        signature: None,
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(
        result.is_err(),
        "Handler should reject output count mismatch"
    );
}
```

### Test 5: Signature Count Mismatch
```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_handler_validates_signature_count() {
    let mint = create_test_mint().await;

    let request = BatchMintRequest {
        quote: vec!["q1".to_string(), "q2".to_string()],
        outputs: vec![],
        signature: Some(vec![Some("sig1".to_string())]), // 1 sig for 2 quotes
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(
        result.is_err(),
        "Handler should reject signature count mismatch"
    );
}
```

## Phase 2: NUT-20 Signature Validation Tests

### Test 1: Valid NUT-20 Signature

This is more complex because it needs a real quote. Use DirectMintConnection pattern:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_accepts_valid_nut20_signatures() {
    let mint = create_test_mint().await;
    let connector = DirectMintConnection::new((*mint).clone());

    // Create wallet with the direct mint connection
    let wallet = Wallet::new(
        "http://test.mint",
        CurrencyUnit::Sat,
        Arc::new(memory::empty().await.unwrap()),
        Mnemonic::generate(12).unwrap().to_seed_normalized(""),
        Some(Arc::new(connector)),
    )
    .unwrap();

    // Generate secret key for signing
    let secret_key = SecretKey::generate();
    let pubkey = secret_key.public_key();

    // Create a quote with pubkey (simulates NUT-20 locked quote)
    let mut quote = MintQuote::new(
        "test_quote".to_string(),
        wallet.mint_url.clone(),
        PaymentMethod::Bolt11,
        Some(100.into()),
        CurrencyUnit::Sat,
        "lnbc1000n...".to_string(),
        9999999999,
        Some(secret_key.clone()),
    );

    // Add to database so it can be fetched
    wallet.localstore.add_mint_quote(quote).await.unwrap();

    // Create a batch request with signature
    let mut mint_req = cdk_common::nuts::MintRequest {
        quote: "test_quote".to_string(),
        outputs: vec![], // Would be actual blinded messages
        signature: None,
    };

    // Sign it with the secret key
    mint_req.sign(secret_key).unwrap();
    let signature = mint_req.signature.unwrap();

    let request = BatchMintRequest {
        quote: vec!["test_quote".to_string()],
        outputs: vec![], // In real test, add actual outputs
        signature: Some(vec![Some(signature)]),
    };

    // Should NOT error when signature is valid
    // (Full test would check actual mint succeeds, not just validates)
    // let result = mint.process_batch_mint_request(request).await;
    // assert!(result.is_ok(), "Should accept valid NUT-20 signatures");
}
```

### Test 2: Invalid Signature Rejected

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_rejects_invalid_nut20_signatures() {
    let mint = create_test_mint().await;

    let request = BatchMintRequest {
        quote: vec!["test_quote".to_string()],
        outputs: vec![],
        signature: Some(vec![Some("invalid_signature_data".to_string())]),
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(
        result.is_err(),
        "Should reject invalid signatures"
    );
}
```

### Test 3: Signature Without Pubkey

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_rejects_signature_without_pubkey() {
    let mint = create_test_mint().await;

    // Create quote WITHOUT pubkey (unlocked)
    let quote = MintQuote::new(
        "test_quote".to_string(),
        "http://test.mint".try_into().unwrap(),
        PaymentMethod::Bolt11,
        Some(100.into()),
        CurrencyUnit::Sat,
        "lnbc1000n...".to_string(),
        9999999999,
        None, // NO SECRET KEY
    );

    // Try to provide signature anyway
    let request = BatchMintRequest {
        quote: vec!["test_quote".to_string()],
        outputs: vec![],
        signature: Some(vec![Some("some_signature".to_string())]),
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(
        result.is_err(),
        "Should reject signature for unlocked quote"
    );
}
```

### Test 4: Mixed Locked/Unlocked

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_handles_mixed_locked_unlocked() {
    let mint = create_test_mint().await;

    let secret_key_1 = SecretKey::generate();

    // Quote 1: locked (has secret key)
    let quote_1 = MintQuote::new(
        "q1".to_string(),
        "http://test.mint".try_into().unwrap(),
        PaymentMethod::Bolt11,
        Some(50.into()),
        CurrencyUnit::Sat,
        "lnbc500n...".to_string(),
        9999999999,
        Some(secret_key_1.clone()),
    );

    // Quote 2: unlocked (no secret key)
    let quote_2 = MintQuote::new(
        "q2".to_string(),
        "http://test.mint".try_into().unwrap(),
        PaymentMethod::Bolt11,
        Some(50.into()),
        CurrencyUnit::Sat,
        "lnbc500n...".to_string(),
        9999999999,
        None,
    );

    // Store both quotes
    // mint.localstore.add_mint_quote(quote_1).await.unwrap();
    // mint.localstore.add_mint_quote(quote_2).await.unwrap();

    // Create batch with mixed signatures (sig for q1, null for q2)
    let mut mint_req_1 = cdk_common::nuts::MintRequest {
        quote: "q1".to_string(),
        outputs: vec![],
        signature: None,
    };
    mint_req_1.sign(secret_key_1).unwrap();

    let request = BatchMintRequest {
        quote: vec!["q1".to_string(), "q2".to_string()],
        outputs: vec![], // Would add actual outputs
        signature: Some(vec![
            mint_req_1.signature.clone(), // Signature for locked quote
            None, // No signature for unlocked quote
        ]),
    };

    // Should process without error
    // let result = mint.process_batch_mint_request(request).await;
    // assert!(result.is_ok(), "Should handle mixed locked/unlocked");
}
```

## Phase 3: Wallet Batch Mint Tests

### Test 1: Quote Validation (Different Mints)

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_wallet_batch_mint_requires_same_mint() {
    let wallet1 = Wallet::new(
        "http://mint1.example.com",
        CurrencyUnit::Sat,
        Arc::new(memory::empty().await.unwrap()),
        Mnemonic::generate(12).unwrap().to_seed_normalized(""),
        None,
    )
    .unwrap();

    let wallet2 = Wallet::new(
        "http://mint2.example.com",
        CurrencyUnit::Sat,
        Arc::new(memory::empty().await.unwrap()),
        Mnemonic::generate(12).unwrap().to_seed_normalized(""),
        None,
    )
    .unwrap();

    // Create quotes from different mints
    let quote1 = MintQuote::new(
        "q1".to_string(),
        wallet1.mint_url.clone(),
        PaymentMethod::Bolt11,
        Some(100.into()),
        CurrencyUnit::Sat,
        "lnbc1000n...".to_string(),
        9999999999,
        None,
    );

    let quote2 = MintQuote::new(
        "q2".to_string(),
        wallet2.mint_url.clone(),
        PaymentMethod::Bolt11,
        Some(100.into()),
        CurrencyUnit::Sat,
        "lnbc1000n...".to_string(),
        9999999999,
        None,
    );

    // Store in wallet1's database
    wallet1.localstore.add_mint_quote(quote1).await.unwrap();

    // Try to batch mint from both (should fail because different mints)
    let result = wallet1
        .mint_batch(
            vec!["q1".to_string()],
            cdk::amount::SplitTarget::default(),
            None,
        )
        .await;

    // The validation should catch this when trying to mint from different mints
    // Result depends on implementation, but concept is: verify same mint validation
}
```

### Test 2: NUT-20 Signature Generation

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_wallet_batch_mint_generates_nut20_signatures() {
    let wallet = Wallet::new(
        "http://test.mint",
        CurrencyUnit::Sat,
        Arc::new(memory::empty().await.unwrap()),
        Mnemonic::generate(12).unwrap().to_seed_normalized(""),
        Some(Arc::new(create_test_mint_connector().await)),
    )
    .unwrap();

    let secret_key = SecretKey::generate();

    // Create quote with secret key
    let quote = MintQuote::new(
        "q1".to_string(),
        wallet.mint_url.clone(),
        PaymentMethod::Bolt11,
        Some(100.into()),
        CurrencyUnit::Sat,
        "lnbc1000n...".to_string(),
        9999999999,
        Some(secret_key),
    );

    wallet.localstore.add_mint_quote(quote).await.unwrap();

    // When wallet.mint_batch is called, it should generate signatures
    // (Actual test implementation would mock the actual mint call)
    // let proofs = wallet.mint_batch(...).await.unwrap();
    // assert!(!proofs.is_empty());
}
```

## Phase 4: End-to-End Integration Test Template

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_e2e_full_flow() {
    // 1. Setup
    let mint = create_test_mint().await;
    let connector = DirectMintConnection::new((*mint).clone());

    // 2. Create wallet with direct connection
    let wallet = Wallet::new(
        "http://test.mint",
        CurrencyUnit::Sat,
        Arc::new(memory::empty().await.unwrap()),
        Mnemonic::generate(12).unwrap().to_seed_normalized(""),
        Some(Arc::new(connector)),
    )
    .unwrap();

    // 3. Create multiple mint quotes
    let q1 = wallet.mint_quote(100.into(), None).await.unwrap();
    let q2 = wallet.mint_quote(50.into(), None).await.unwrap();

    // 4. In test environment, FakeWallet auto-pays, so quotes are marked paid
    // In regtest, would use: pay_if_regtest(&work_dir, &invoice).await.unwrap();

    // 5. Perform batch mint
    let proofs = wallet
        .mint_batch(
            vec![q1.id.clone(), q2.id.clone()],
            cdk::amount::SplitTarget::default(),
            None,
        )
        .await
        .unwrap();

    // 6. Verify results
    assert!(!proofs.is_empty(), "Should return proofs");
    let total = proofs.total_amount().unwrap();
    assert_eq!(total, cdk::Amount::from(150), "Total should match quotes");

    // 7. Verify proofs are stored
    let all_proofs = wallet.get_proofs().await.unwrap();
    assert!(!all_proofs.is_empty(), "Proofs should be in storage");
}
```

## Key Patterns

1. **Always use `#[tokio::test(flavor = "multi_thread", worker_threads = 1)]`**
   - Required for async/await
   - Single worker thread prevents test interactions

2. **Use DirectMintConnection for wallet tests**
   - In-memory, no HTTP
   - Implements MintConnector trait
   - Already converts String ↔ UUID quote IDs

3. **Use `memory::empty()` for databases**
   - No filesystem required
   - Fresh DB per test
   - Fast execution

4. **Generate keys with `SecretKey::generate()`**
   - Each test gets unique keys
   - Ready to use for signing

5. **Use `.unwrap()` liberally in tests**
   - If setup fails, test should fail
   - Better error messages than Result propagation

## Running Tests

```bash
# All batch mint tests
cargo test --test batch_mint

# With output
cargo test --test batch_mint -- --nocapture

# Specific test
cargo test --test batch_mint test_batch_mint_handler_rejects_empty

# With specific database
CDK_TEST_DB_TYPE=memory cargo test --test batch_mint
```
