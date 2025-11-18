# Batch Mint + NUT-20 Integration Test Strategy

## Executive Summary

The CDK codebase has **comprehensive testing infrastructure** already in place. Writing the critical missing batch mint + NUT-20 tests is straightforward and follows established patterns.

## Current Test Infrastructure

### 1. Test Setup Utilities (cdk-integration-tests crate)

**Location:** `crates/cdk-integration-tests/src/`

**Available Components:**

- **DirectMintConnection** (`init_pure_tests.rs`) - Direct in-memory mint connection
  - Implements `MintConnector` trait
  - No HTTP overhead, no network required
  - Perfect for unit + integration testing
  - Already converts between String and Uuid quote IDs

- **Test Helpers:**
  - `fund_wallet()` - Quick wallet funding
  - `get_mint_url_from_env()` - Environment-based mint selection
  - `pay_if_regtest()` - Conditional payment for regtest mode
  - Wallet and Mint builder patterns

### 2. CI/CD Pipeline

**Location:** `.github/workflows/ci.yml`

**Database Coverage:**
- SQLite (all PRs)
- PostgreSQL (main branch + release)

**Feature Combinations:**
- Tests against multiple feature flags
- Tests with/without database features
- Tests integration-tests crate

**Test Execution:**
- Linux runners with ample resources
- Rust cache enabled
- Nix flake-based builds (reproducible)

### 3. Existing Test Patterns

**Integration Test Examples:**
- `tests/happy_path_mint_wallet.rs` - Full mint-melt flow with WebSocket notifications
- `tests/mint.rs` - Mint-specific operations (keyset rotation, DB transactions)
- `tests/batch_mint.rs` - Current batch mint structure tests
- `tests/p2pk_*.rs` - NUT-11 spending condition tests (excellent NUT-20 reference)

## What to Write for Batch Mint + NUT-20

### Phase 1: Handler Validation Tests (5 tests)
**File:** `tests/batch_mint.rs` - Add to existing file

These test the HTTP handler validation logic in `cdk-axum/src/router_handlers.rs:780-818`

```rust
#[tokio::test]
async fn test_batch_mint_empty_quote_list_rejected() {
    let mint = create_test_mint().await;

    let request = BatchMintRequest {
        quote: vec![],  // Empty!
        outputs: vec![],
        signature: None,
    };

    // Should return error about empty quotes
    let result = mint.process_batch_mint_request(request).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_batch_mint_duplicate_quotes_rejected() {
    let mint = create_test_mint().await;

    // Create a batch with duplicate quote IDs
    let request = BatchMintRequest {
        quote: vec!["q1".to_string(), "q2".to_string(), "q1".to_string()],
        outputs: vec![],
        signature: None,
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_batch_mint_exceeds_100_quotes() {
    let mint = create_test_mint().await;

    let quotes: Vec<String> = (0..101).map(|i| format!("q{}", i)).collect();
    let request = BatchMintRequest {
        quote: quotes,
        outputs: vec![],
        signature: None,
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_batch_mint_output_count_mismatch() {
    let mint = create_test_mint().await;

    let request = BatchMintRequest {
        quote: vec!["q1".to_string(), "q2".to_string()],
        outputs: vec![], // 0 outputs for 2 quotes
        signature: None,
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_batch_mint_signature_count_mismatch() {
    let mint = create_test_mint().await;

    let request = BatchMintRequest {
        quote: vec!["q1".to_string(), "q2".to_string()],
        outputs: vec![],
        signature: Some(vec![Some("sig1".to_string())]), // 1 sig for 2 quotes
    };

    let result = mint.process_batch_mint_request(request).await;
    assert!(result.is_err());
}
```

**Why this matters:**
- These tests the actual handler validation we implemented
- Currently untested error paths
- ~30 minutes to write

### Phase 2: NUT-20 Signature Validation Tests (4 tests)
**File:** `tests/batch_mint.rs` - New section

Tests the signature verification logic in `cdk/src/mint/issue/mod.rs:766-791`

```rust
#[tokio::test]
async fn test_batch_mint_valid_nut20_signature_accepted() {
    // Create a real quote with a pubkey
    // Generate a valid signature using the secret key
    // Submit batch mint with valid signature
    // Assert it's accepted
}

#[tokio::test]
async fn test_batch_mint_invalid_nut20_signature_rejected() {
    // Create quote with pubkey
    // Submit with tampered/invalid signature
    // Assert rejection with appropriate error
}

#[tokio::test]
async fn test_batch_mint_signature_without_pubkey_rejected() {
    // Create quote without pubkey (unlocked)
    // Submit with signature anyway
    // Assert error: "signature provided but quote has no pubkey"
}

#[tokio::test]
async fn test_batch_mint_mixed_locked_unlocked() {
    // Create 3 quotes: locked, unlocked, locked
    // Submit signatures for quotes 0 and 2 (null for 1)
    // Assert batch processes correctly with mixed signatures
}
```

**Why this matters:**
- Tests the NUT-20 verification we added
- Mixed locked/unlocked is a key feature
- ~45 minutes to write

### Phase 3: Wallet Batch Mint Tests (4 tests)
**File:** `tests/batch_mint.rs` or new file `tests/wallet_batch_mint.rs`

Tests the wallet-side batch minting in `cdk/src/wallet/issue/batch.rs`

```rust
#[tokio::test]
async fn test_wallet_batch_mint_quote_validation() {
    // Create 3 quotes with different mints/methods
    // Attempt batch mint
    // Assert error for different mints/methods
}

#[tokio::test]
async fn test_wallet_batch_mint_generates_nut20_signatures() {
    // Create wallet with secret keys in quotes
    // Call mint_batch
    // Verify signatures are generated in the request
    // Verify they're valid against the secret keys
}

#[tokio::test]
async fn test_wallet_batch_mint_proof_storage() {
    // Complete a successful batch mint
    // Verify proofs are stored in localstore
    // Verify proof count matches output count
}

#[tokio::test]
async fn test_wallet_batch_mint_idempotency() {
    // Submit same batch request twice
    // Verify same signatures/proofs returned both times
    // Verify no double-spending or state issues
}
```

**Why this matters:**
- Tests the wallet-side signature generation we implemented
- Ensures proof storage works
- ~60 minutes to write

### Phase 4: End-to-End Integration Tests (3+ tests)
**File:** `tests/happy_path_batch_mint.rs` (new file)

Full flow testing using DirectMintConnection pattern:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_full_flow() {
    // 1. Create test mint
    let mint = create_test_mint().await;
    let connector = DirectMintConnection::new(mint.clone());

    // 2. Create wallet
    let wallet = Wallet::new(
        "http://test.mint",
        CurrencyUnit::Sat,
        Arc::new(memory::empty().await.unwrap()),
        Mnemonic::generate(12).unwrap().to_seed_normalized(""),
        Some(Arc::new(connector)),
    ).unwrap();

    // 3. Create multiple quotes
    let quote1 = wallet.mint_quote(100.into(), None).await.unwrap();
    let quote2 = wallet.mint_quote(50.into(), None).await.unwrap();

    // 4. Simulate payment (FakeWallet auto-pays in tests)
    // (In regtest, would use pay_if_regtest helper)

    // 5. Check batch status
    wallet.check_batch_quote_status(vec![quote1.id.clone(), quote2.id.clone()]).await.unwrap();

    // 6. Perform batch mint
    let proofs = wallet.mint_batch(
        vec![quote1.id.clone(), quote2.id.clone()],
        SplitTarget::default(),
        None,
    ).await.unwrap();

    // 7. Verify results
    assert_eq!(proofs.len(), 2);
    let total = proofs.total_amount().unwrap();
    assert_eq!(total, Amount::from(150));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_with_nut20_locking() {
    // Like above but with spending conditions
    // Creates quotes with pubkeys
    // Mints with signatures
    // Verifies locked proofs have spending conditions
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_batch_mint_multiple_payment_methods() {
    // Create quotes with different payment methods
    // Should error or handle gracefully
}
```

**Why this matters:**
- Full integration testing
- Uses existing DirectMintConnection pattern
- Tests real wallet-mint interaction
- ~90 minutes to write

## Implementation Timeline

| Phase | Tests | Effort | Priority |
|-------|-------|--------|----------|
| 1 | 5 handler validation | 30 min | HIGH |
| 2 | 4 NUT-20 signature | 45 min | HIGH |
| 3 | 4 wallet batch mint | 60 min | MEDIUM |
| 4 | 3+ end-to-end | 90 min | MEDIUM |

**Total effort: ~225 minutes (~3.5 hours)**

## Key Infrastructure Already Available

✅ **DirectMintConnection** - in-memory mint testing
✅ **Wallet builder** - creates test wallets
✅ **FakeWallet** - auto-pays invoices in tests
✅ **Database backends** - SQLite + PostgreSQL
✅ **CI runners** - Postgres/SQLite, multiple feature combos
✅ **Async test framework** - tokio integration
✅ **Helper utilities** - fund_wallet, environment helpers

## Implementation Steps

1. **Phase 1 (Handler Validation)** - Add 5 tests to `batch_mint.rs`
   - Start with `create_test_mint()` helper already in place
   - Call `mint.process_batch_mint_request()` directly
   - Assert error cases

2. **Phase 2 (NUT-20 Signatures)** - Add 4 tests to same file
   - Use `SecretKey::generate()` to create pubkeys
   - Create quotes with `quote.pubkey = Some(pubkey)`
   - Use existing `MintRequest::sign()` method

3. **Phase 3 (Wallet Tests)** - Add 4 tests
   - Use DirectMintConnection pattern
   - Create real Wallet instances
   - Call `wallet.mint_batch()` directly
   - Verify localstore contents

4. **Phase 4 (E2E Tests)** - New `happy_path_batch_mint.rs`
   - Copy pattern from `happy_path_mint_wallet.rs`
   - Create multiple quotes in loop
   - Verify full flow works

## CI Integration

Tests will automatically run in CI because they're in `cdk-integration-tests`:

- ✅ Database: SQLite + PostgreSQL
- ✅ Feature matrix: All current combinations
- ✅ No additional configuration needed
- ✅ Will run on every PR to main branch

## Summary

**The infrastructure is ready.** The codebase has:
- In-memory mint testing (DirectMintConnection)
- Full wallet-mint integration patterns
- Comprehensive CI that tests with multiple databases
- Well-established test file organization
- Helper utilities for common operations

**Writing comprehensive batch mint + NUT-20 tests is straightforward and follows established patterns in the codebase.**
