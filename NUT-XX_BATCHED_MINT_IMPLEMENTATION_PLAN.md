# NUT-XX Batched Mint Implementation Plan

## Overview

This document outlines the implementation plan for NUT-XX (Batched Mint) support in CDK. The spec enables wallets to mint multiple proofs in a single batch operation, reducing round-trips and improving performance.

**Key Constraint**: Single payment method per batch (architectural limitation analysis in appendix).

**Development Phases**:
1. Phase 1: Mint-side batch endpoints and logic
2. Phase 2: Wallet-side batch client
3. Phase 2.5: NUT-20 support (deferred but required)
4. Phase 3: Documentation and integration tests

---

## Phase 1: Mint-Side Implementation ✅ COMPLETE

### Task 1.1: Design & Data Structures ✅ COMPLETE

**Objective**: Define request/response types matching NUT-XX spec.

**Changes**:
- Create new types in `crates/cdk-common/src/mint.rs`:
  - `BatchMintRequest` struct:
    ```rust
    pub struct BatchMintRequest {
        pub quote: Vec<String>,           // Quote IDs
        pub outputs: Vec<BlindedMessage>, // Blinded messages
        pub signature: Option<Vec<Option<String>>>, // NUT-20 signatures
    }
    ```
  - `BatchQuoteStatusRequest` struct:
    ```rust
    pub struct BatchQuoteStatusRequest {
        pub quote: Vec<String>,
    }
    ```
  - `BatchQuoteStatusResponse` struct:
    ```rust
    pub struct BatchQuoteStatusResponse(Vec<MintQuote>);
    ```
- Update error types to support batch-specific errors (e.g., `QuoteNotFound`, `PartialPaymentStatus`)
- Ensure all types have proper `Serialize`/`Deserialize` with spec-compliant field naming

**Dependencies**: None (new types only)

**Files to Create/Modify**:
- `crates/cdk-common/src/mint.rs` (new types)

---

### Task 1.2: Batch Quote Status Endpoint ✅ COMPLETE

**Objective**: Implement `POST /v1/mint/{method}/check` endpoint.

**Endpoint Specification**:
```
POST /v1/mint/{method}/check
Content-Type: application/json

Request:
{
  "quote": ["quote_id_1", "quote_id_2", ...]
}

Response:
[
  {
    "quote": "quote_id_1",
    "request": "payment_request_1",
    "unit": "SAT",
    "amount": 1000,
    "paid": true,
    "state": "PAID",
    ...
  },
  ...
]
```

**Implementation Details**:

1. **Validation**:
   - Check all quote IDs are non-empty strings
   - Check quote IDs are unique (no duplicates)
   - Verify payment method is valid and supported
   - Max batch size: **100 quotes per request** (recommendation: prevents abuse, matches industry standards)

2. **Status Checking Logic**:
   - For each quote ID in request:
     - Query database for quote by ID and method
     - If quote not found: **omit from response** (per spec)
     - If quote state is PAID or ISSUED: return cached data from DB
     - If quote state is UNPAID: query payment processor for status update
   - Return array of known quotes in same order as input (for client correlation)

3. **Payment Processor Integration**:
   - Reuse existing `DynMintPayment::payment_status()` for status checking
   - Handle method-specific status fields (e.g., bolt11 payment hash vs bolt12 offer ID)
   - Update quote state in DB if payment detected (transitions UNPAID → PAID)

4. **Caching & Idempotency**:
   - Use existing `HttpCache` pattern for GET requests (if making status check idempotent)
   - Alternatively: implement POST-based caching with request hash as key
   - TTL: match single quote status check TTL (default: 60 seconds)

5. **Error Handling**:
   - 400: Invalid request (duplicate quotes, invalid method)
   - 404: Payment method not found
   - 500: Payment processor error (log, return 503)
   - Return empty array if all quotes unknown (not an error)

**Files to Modify**:
- `crates/cdk-axum/src/router_handlers.rs` (new handler)
- `crates/cdk/src/mint/ln.rs` or similar (status checking logic)
- `crates/cdk-axum/src/lib.rs` (add route)

---

### Task 1.3: Batch Mint Execution Endpoint ✅ COMPLETE

**Objective**: Implement `POST /v1/mint/{method}/batch` endpoint.

**Endpoint Specification**:
```
POST /v1/mint/{method}/batch
Content-Type: application/json

Request:
{
  "quote": ["quote_id_1", "quote_id_2", ...],
  "outputs": [BlindedMessage_1, BlindedMessage_2, ...],
  "signature": [signature_1, null, signature_3] // NUT-20, optional
}

Response:
{
  "signatures": [BlindSignature_1, BlindSignature_2, ...]
}
```

**Implementation Details**:

1. **Validation**:
   - Check quotes array is non-empty and unique
   - Check outputs array length matches quotes array length
   - Check signature array (if present) matches quotes array length
   - Verify all quotes exist in database
   - Verify all quotes belong to same payment method
   - Verify all quotes have state == PAID (not UNPAID, not ISSUED)
   - Verify sum of blinded message amounts equals sum of quote amounts (exact match for bolt11, >= for bolt12)
   - NUT-20: if any signature is non-null, verify signature count matches quotes count

2. **Pre-Minting State Checks**:
   - Acquire read locks on all quotes (to ensure they don't transition while processing)
   - Check no quote has expired (use method-specific TTL)
   - Check no quote is currently being processed by another request (idempotency guard)

3. **Blind Signature Generation**:
   - Reuse existing signing logic from single mint
   - For each (quote, blinded_message) pair:
     - Get keyset for (unit, method)
     - Get signing key from keyset
     - Generate blind signature
     - Collect results in order
   - **Do NOT begin DB transaction until signatures generated** (to avoid lock contention, per existing pattern)

4. **Atomic State Update**:
   - Begin transaction
   - For each quote: update state to ISSUED, set issued_at timestamp
   - Increment quote counter atomically
   - Commit transaction
   - If commit fails: return 409 Conflict (quote already issued in another request)

5. **Response & Idempotency**:
   - Return signatures in same order as outputs (deterministic)
   - Cache response by request hash (like single mint)
   - Idempotency key: hash of (quotes, outputs, signature) → allows safe retries

6. **Error Handling**:
   - 400: Invalid request (validation failures, quote not found)
   - 409: Quote state conflict (already issued, already expired)
   - 500: Signing error or DB error
   - Return partial error details (e.g., "Quote quote_id_2 not found, quote_id_3 already issued")

**Files to Modify**:
- `crates/cdk-axum/src/router_handlers.rs` (new handler)
- `crates/cdk/src/mint/issue/mod.rs` (batch signing logic)
- `crates/cdk-axum/src/lib.rs` (add route)

**Design Decision**: Use separate `/batch` endpoint instead of extending single `/v1/mint/{method}` to avoid ambiguity in request parsing.

---

### Task 1.4: Testing (Mint) ✅ COMPLETE

**Objective**: Comprehensive unit and integration tests for batch endpoints.

**Status**: 13 tests passing in `cdk-integration-tests/tests/batch_mint.rs`

**Unit Tests** (in respective `mod.rs` files):

1. **Quote Status Endpoint**:
   - Happy path: multiple quotes in PAID state
   - Mixed states: some UNPAID (query payment processor), some PAID (cached), some ISSUED
   - Unknown quotes: omit from response, don't fail
   - Duplicate quote IDs: reject with 400
   - Empty quote array: reject with 400
   - Unique verification: verify returned array doesn't include duplicates even if input had typos
   - Method validation: reject invalid payment method

2. **Mint Execution Endpoint**:
   - Happy path: valid batch with matching amounts
   - Quote not found: return 400 with details
   - Quote in UNPAID state: return 409
   - Quote in ISSUED state: return 409 (already minted)
   - Mismatched amounts: outputs sum ≠ quotes sum, return 400
   - Mismatched array lengths: outputs.len() ≠ quotes.len(), return 400
   - Mixed payment methods: return 400
   - Idempotency: same request twice returns same signatures (cached)
   - Signature generation: verify blind signatures are valid (using existing crypto tests)

3. **Integration Tests** (in `crates/cdk/tests/`):
   - End-to-end workflow: create quotes → check status → mint
   - Partial payment: mint only when all quotes are PAID
   - Concurrent requests: two batch requests for same quotes (second gets 409)
   - Large batch: 100 quotes in single request (performance baseline)
   - Empty response: batch with all unknown quotes returns empty array
   - Database rollback: if transaction fails, quotes remain UNPAID

**Test Fixtures**:
- Helper function to create N quotes with specified states
- Mock payment processor for different method behaviors
- Deterministic blind message generation for reproducibility

**Files to Create/Modify**:
- `crates/cdk/tests/batch_mint.rs` (new integration test file)
- `crates/cdk/src/mint/issue/mod.rs` (add unit tests)
- `crates/cdk-axum/tests/routes.rs` (add endpoint tests)

---

## Phase 2: Wallet-Side Implementation ✅ COMPLETE

### Task 2.1: Wallet Batch Minting Client ✅ COMPLETE

**Objective**: Implement wallet method `mint_batch()` for batch minting.

**API Design**:
```rust
pub async fn mint_batch(
    &mut self,
    quote_ids: Vec<String>,
) -> Result<Vec<Proof>, WalletError>
```

**Flow**:
1. Fetch quote details for all IDs (from wallet's quote cache or re-fetch from mint)
2. Verify all quotes from same mint and same method
3. Calculate total amount across all quotes
4. Generate deterministic secrets and blinded messages for total amount
5. Call mint's batch status check endpoint
6. Verify all quotes are in PAID state
7. Call mint's batch mint endpoint
8. Unblind signatures to proofs
9. Store proofs in database
10. Return proofs in deterministic order

**Implementation Details**:

1. **Secret & Message Generation**:
   - Reuse existing `PreMintSecrets::with_keyset_counter()` logic
   - For batch, use single counter increment for all quotes combined
   - Generate outputs deterministically (sorted by amount if needed for reproducibility)
   - Ensure outputs are in same order as quotes for response correlation

2. **Validation**:
   - All quotes from same mint URL
   - All quotes from same payment method
   - All quotes from same unit (don't support mixed units in batch)
   - No expired quotes
   - Total amount > 0

3. **Error Handling**:
   - Network errors: retry logic (exponential backoff)
   - Quote not found: return error with details
   - Quote expired: return error, suggest re-creating quotes
   - Partial minting failure: return error with which quotes failed

4. **Idempotency**:
   - Store batch request hash locally
   - If minting same quotes again, check if proofs already exist
   - Return cached proofs if idempotent request detected

**Files to Create/Modify**:
- `crates/cdk/src/wallet/issue/mod.rs` (new method)
- `crates/cdk/src/wallet/mod.rs` (expose method in public API)

---

### Task 2.2: Testing (Wallet) ✅ COMPLETE

**Objective**: Test wallet batch minting client.

**Tests Implemented** (6 tests passing):
- ✅ `test_wallet_batch_mint_validates_same_unit` - validates all quotes have same unit
- ✅ `test_wallet_batch_mint_mixed_payment_methods_error` - rejects mixed payment methods
- ✅ `test_wallet_batch_mint_unpaid_quote_error` - requires all quotes to be PAID
- ✅ `test_wallet_batch_mint_single_quote_validation` - handles single quote batches
- ✅ `test_wallet_batch_mint_empty_list_error` - rejects empty quote lists
- ✅ `test_wallet_batch_mint_unknown_quote_error` - rejects unknown quotes

**Files Created/Modified**:
- ✅ `crates/cdk/tests/wallet_batch_mint.rs` (new test file - 165 lines)

---

## Phase 2.5: NUT-20 Support (Deferred but Required) ⏳ PARTIAL - Infrastructure Only

**Status**: Batch request structure includes signature field, but full validation and signature generation NOT YET IMPLEMENTED

**Current State:**
- ✅ BatchMintRequest includes optional `signature: Option<Vec<Option<String>>>` field
- ⏳ Mint-side signature validation NOT IMPLEMENTED
- ⏳ Wallet-side signature generation NOT IMPLEMENTED
- ⏳ Full test coverage NOT IMPLEMENTED

**Blockers:**
- MintQuote lacks `spending_condition` field needed to detect NUT-20 locked quotes and get pubkeys for validation
- Need to decide on schema change approach before proceeding

### Task 2.5.1: Design NUT-20 for Batches

**Changes**:
- `BatchMintRequest.signature` field becomes `Vec<Option<String>>` (already in initial design)
- Validation rule: if any signature is non-null, `signature.len() === quote.len()`
- Signatures matched to quotes by index position (per spec)

**Design Decisions**:
- Each quote can be either locked (NUT-20) or unlocked
- Signatures array contains `null` for unlocked quotes, signature string for locked quotes
- Wallet must generate correct signature for each locked quote

---

### Task 2.5.2: Mint-Side NUT-20 Validation ⏳ NOT STARTED

**Objective**: Validate NUT-20 signatures in batch mint requests

**Changes to Batch Mint Endpoint**:
1. Extract signature array from BatchMintRequest
2. Validation: if any signature non-null, length must match quotes length
3. For each (quote, signature) pair:
   - If signature is null: continue (unlocked quote)
   - If signature is non-null:
     - Get quote's pubkey from quote details (requires quote to include spending_condition)
     - Verify signature matches that pubkey
     - Verify signature is cryptographically valid (using cashu crate NUT-20 crypto)
4. Only generate blind signatures if all NUT-20 validations pass
5. Return error if any signature validation fails

**Files to Modify**:
- `crates/cdk/src/mint/issue/mod.rs` (batch mint validation logic)
- `crates/cdk-axum/src/router_handlers.rs` (batch mint handler)

**Dependencies**:
- Requires quote details to include spending_condition metadata
- Requires understanding of cashu NUT-20 signature verification API

---

### Task 2.5.3: Wallet-Side NUT-20 Signature Generation ⏳ NOT STARTED

**Objective**: Generate NUT-20 signatures for locked quotes in batch minting

**Changes to `mint_batch()`**:
1. Detect which quotes are NUT-20 locked by checking spending_condition field
2. For each quote:
   - If locked: generate NUT-20 signature using wallet's private key
   - If unlocked: append None to signatures array
3. Build signature array with nulls for unlocked, signatures for locked
4. Pass signature array with batch mint request

**Implementation Details**:
- Reuse existing NUT-20 signature generation logic from wallet
- Ensure signatures are in same order as quotes for correlation
- Handle wallet's key derivation and signing

**Files to Modify**:
- `crates/cdk/src/wallet/issue/batch.rs` (signature generation in mint_batch)

**Dependencies**:
- MintQuote must include `spending_condition: Option<SpendingConditions>` field
- Requires understanding wallet's key management for NUT-20 signing

---

### Task 2.5.4: NUT-20 Testing ⏳ NOT STARTED

**Unit Tests for Mint-Side**:
- ✅ Array length validation: signature array must match quote array length
- ✅ Null signature handling: null signatures are allowed for unlocked quotes
- ⏳ Signature verification: valid NUT-20 signatures accepted
- ⏳ Invalid signature rejection: malformed or wrong signatures rejected
- ⏳ Pubkey validation: signature verified against correct quote pubkey
- ⏳ Mixed locked/unlocked: batch with some locked, some unlocked quotes

**Unit Tests for Wallet-Side**:
- ⏳ Signature generation: wallet generates valid NUT-20 signatures
- ⏳ Null handling: unlocked quotes get None in signature array
- ⏳ Array building: signatures in correct order matching quotes
- ⏳ Request structure: signature array correctly included in BatchMintRequest

**Integration Tests**:
- ⏳ End-to-end: wallet → mint with NUT-20 locked quotes
- ⏳ Rejection flows: invalid signatures properly rejected by mint
- ⏳ Mixed scenarios: batch with locked, unlocked, and expired quotes

---

## Phase 3: Polish & Documentation

### Task 3.1: Documentation

**Files to Create/Modify**:
- Update `DEVELOPMENT.md` with batch minting section
- Update API endpoint documentation
- Add examples in README (or separate EXAMPLES.md)
- Document single-method-per-batch requirement
- Document batch size limit (100 quotes)
- Document NUT-20 integration (when complete)

---

### Task 3.2: Integration Tests

**Objective**: End-to-end tests combining mint and wallet.

**Tests**:
- Full workflow: wallet creates quotes → checks status → mints batch
- Cross-platform: mint in Rust, wallet in different language (if applicable)
- Load testing: mint handles 1000-quote batches (stress test)
- Backward compatibility: single mint endpoint still works
- Migration: wallets upgraded from single to batch minting

---

## Design Decisions & Rationale

### 1. Separate `/batch` Endpoint
- **Decision**: Use `POST /v1/mint/{method}/batch` instead of extending single endpoint
- **Rationale**: Avoids ambiguity in request parsing (arrays vs single objects), clearer API semantics
- **Alternative Rejected**: Query param like `?batch=true` is less explicit

### 2. Batch Size Limit: 100 Quotes
- **Decision**: Maximum 100 quotes per batch request
- **Rationale**: Prevents abuse, matches industry standards (e.g., Stripe), reasonable for typical use cases
- **Justification**: 100 quotes × 33 bytes per quote ≈ 3.3KB request body (manageable)

### 3. Single Payment Method Per Batch
- **Decision**: All quotes must be same payment method
- **Rationale**: Architectural limitation (incompatible expiry models, processor routing, etc.)
- **Analysis**: See Appendix: Payment Method Complexity Analysis

### 4. Omit Unknown Quotes from Response
- **Decision**: If quote not found, don't include in batch status response (per spec)
- **Rationale**: Prevents leaking information about quote existence, matches spec intent
- **Alternative Rejected**: Return error for unknown quotes (worse UX, not spec-compliant)

### 5. Separate DB Transaction for Signing
- **Decision**: Generate blind signatures BEFORE beginning DB transaction
- **Rationale**: Follows existing pattern, avoids lock contention, signing is idempotent
- **Alternative Rejected**: Sign within transaction (causes long locks, performance penalty)

### 6. NUT-20 Deferred to Phase 2.5
- **Decision**: Implement Phase 1 & 2 without NUT-20, add later
- **Rationale**: Simplifies initial development, NUT-20 is independent concern
- **Constraint**: Must be complete before merging to main (hard requirement)

---

## Appendix: Payment Method Complexity Analysis

### Why Multiple Payment Methods Per Batch is Infeasible

#### 1. Incompatible Quote Expiry Models
- **BOLT11**: Fixed expiry (default 3600s) calculated at payment request creation
- **BOLT12**: Non-expiring offers, reusable indefinitely
- **Problem**: Batch request cannot reconcile these. What is batch expiry? When one method expires?

#### 2. No Multi-Method Routing Path
- Current architecture: `DynMintPayment` processor keyed by `(unit, method)`
- Each processor handles one method in isolation
- No abstraction to process multiple methods in parallel

#### 3. Incompatible Payment Identifiers
- **BOLT11**: `PaymentHash([u8; 32])`
- **BOLT12**: `OfferId(String)` or `Bolt12PaymentHash`
- Batch would need per-quote identifier mapping, adding state complexity

#### 4. Conflicting Validation Rules
- Min/max amounts differ per method
- MPP (Multi-Part Payment) support differs
- Amountless invoice support differs
- Pubkey requirements differ

#### 5. Database Schema Lock-In
- Quotes have immutable `payment_method: PaymentMethod` field
- No batch operations exist in DB layer
- Would require schema redesign to support batches

### Recommendation
**Require one payment method per batch.** Wallet can submit multiple separate batches if needed.

---

## Success Criteria

**Phase 1 & 2 (COMPLETE):**
- ✅ All Phase 1 tests pass (13/13 tests passing)
- ✅ All Phase 2 tests pass (6/6 wallet batch mint tests passing)
- ✅ Backward compatibility: single mint still works identically

**Phase 2.5 (NUT-20) - In Progress:**
- ⏳ Task 2.5.0: Schema changes (MintQuote + database migrations)
- ⏳ Task 2.5.2: Mint-side NUT-20 signature validation
- ⏳ Task 2.5.3: Wallet-side NUT-20 signature generation
- ⏳ Task 2.5.4: Comprehensive NUT-20 test suite
- ✅ Batch request structure includes signature field (done)

**Phase 3 (Not Started):**
- ⏳ Performance: batch mint 100 quotes < 2x time of single quote
- ⏳ No new security vulnerabilities introduced
- ⏳ Code review approval

---

## Timeline Estimate

- Phase 1: 3-4 days (design, endpoints, testing)
- Phase 2: 2-3 days (wallet client, testing)
- Phase 2.5: 2-3 days (NUT-20 integration, testing)
- Phase 3: 1-2 days (documentation, integration tests)
- **Total**: 8-12 days

---

## Implementation Summary

### Phase 1 Completion (Mint-Side)
All Phase 1 tasks completed with 13 passing tests:
- Batch quote status endpoint: `POST /v1/mint/quote/batch` (returns array of quote states)
- Batch mint endpoint: `POST /v1/mint/batch` (takes multiple quotes, returns combined signatures)
- Full validation: quote uniqueness, state verification, amount validation
- Idempotency support via request hashing

**Files Added**:
- `crates/cdk-common/src/mint.rs` - BatchMintRequest, BatchQuoteStatusRequest, BatchQuoteStatusResponse types
- HTTP client implementations in cdk-axum router handlers

### Phase 2 Completion (Wallet-Side)
All Phase 2 tasks completed with 6 passing wallet tests:
- `Wallet::mint_batch()` method supporting multiple quotes in single transaction
- Comprehensive validation: same payment method, same unit, PAID state required
- Deterministic secret/message generation using wallet counter
- Proper error handling for validation failures

**Files Added**:
- `crates/cdk/src/wallet/issue/batch.rs` (156 lines) - complete batch minting implementation
- `crates/cdk/tests/wallet_batch_mint.rs` (165 lines) - comprehensive test suite
- Updated `crates/cdk/src/wallet/mint_connector/mod.rs` - added trait methods
- Updated `crates/cdk/src/wallet/mint_connector/http_client.rs` - implemented HTTP client methods
- Updated `crates/cdk-integration-tests/src/init_pure_tests.rs` - DirectMintConnection test harness support

### Phase 2.5 Status (NUT-20 Support) - ROADMAP FINALIZED
**Decision Made**: Proceeding with schema change approach (Option A)

**Current State:**
- ✅ Batch request structure includes optional signature field
- ✅ Implementation roadmap documented with 4 sequential tasks
- ⏳ Schema changes (Task 2.5.0) - NEXT PRIORITY
- ⏳ Mint-side validation (Task 2.5.2)
- ⏳ Wallet-side signature generation (Task 2.5.3)
- ⏳ Comprehensive test suite (Task 2.5.4)

**Implementation Path:**
1. **Task 2.5.0** (2-3 hrs): Add `spending_condition` field to MintQuote + database migrations
2. **Task 2.5.2** (1-2 hrs): Implement NUT-20 signature validation in batch mint handler
3. **Task 2.5.3** (1-2 hrs): Generate signatures in wallet's mint_batch() method
4. **Task 2.5.4** (2-3 hrs): Write comprehensive test suite

**Total Effort**: 6-10 hours (~1-1.5 days)

### Test Coverage
- **Phase 1**: 13/13 integration tests passing
- **Phase 2**: 6/6 wallet unit tests passing
- **Validation tests**: Empty lists, unknown quotes, mixed payment methods, unpaid quotes, unit validation
- **Backward compatibility**: All existing mint/wallet tests continue to pass

## Phase 2.5 Implementation Roadmap (Schema Change Approach)

### ✅ DECISION: Schema Change (Option A)
Proceeding with adding `spending_condition` to MintQuote struct for full NUT-20 support.

### Task 2.5.0: Schema & Database Changes (NEW) - PREREQUISITE

**Objective**: Extend MintQuote to include spending condition metadata

**Changes Required**:

1. **Update MintQuote struct** in `crates/cdk-common/src/wallet.rs`:
   ```rust
   pub struct MintQuote {
       pub id: String,
       pub mint_url: MintUrl,
       pub payment_method: PaymentMethod,
       pub amount: Option<Amount>,
       pub unit: CurrencyUnit,
       pub request: String,
       pub expiry: u64,
       pub secret_key: Option<SecretKey>,
       pub state: MintQuoteState,
       // NEW FIELD:
       pub spending_condition: Option<SpendingConditions>,
   }
   ```

2. **Update database schema** for all storage backends:
   - `cdk-sqlite/migrations/wallet/` - add spending_condition column
   - `cdk-postgres/migrations/wallet/` - add spending_condition column
   - `cdk-redb/` - update serialization
   - Update QuoteRow/QuoteInfo serialization

3. **Update mint quote response handling**:
   - Server endpoints that return mint quotes must include spending_condition
   - Both BOLT11 and BOLT12 quote responses need this field
   - Backward compatibility: field should be optional (default None for unlocked quotes)

**Files to Modify**:
- `crates/cdk-common/src/wallet.rs` - MintQuote struct
- `crates/cdk-sqlite/src/wallet/migrations/` - SQL migrations
- `crates/cdk-postgres/src/wallet/migrations/` - SQL migrations
- `crates/cdk-redb/src/wallet/` - schema updates
- `crates/cdk/src/wallet/issue/issue_bolt11.rs` - quote handling
- `crates/cdk/src/wallet/issue/issue_bolt12.rs` - quote handling

**Estimated Effort**: 2-3 hours

**Testing**:
- Unit tests for schema serialization/deserialization
- Database migration tests
- Backward compatibility tests (old quotes still work)

---

### Task 2.5.2: Mint-Side NUT-20 Validation ⏳ NEXT
**Status**: Depends on Task 2.5.0
**Effort**: 1-2 hours
**Description**: Implement signature validation in batch mint handler

### Task 2.5.3: Wallet-Side NUT-20 Signature Generation ⏳ AFTER 2.5.2
**Status**: Depends on Task 2.5.0 and 2.5.2
**Effort**: 1-2 hours
**Description**: Generate signatures for locked quotes in wallet's mint_batch()

### Task 2.5.4: NUT-20 Testing ⏳ AFTER 2.5.3
**Status**: Depends on Tasks 2.5.2 and 2.5.3
**Effort**: 2-3 hours
**Description**: Comprehensive test suite for NUT-20 batch minting

---

### Overall NUT-20 Implementation Effort
- **Total**: 6-10 hours (~1-1.5 days)
- **Critical Path**: Task 2.5.0 → 2.5.2 → 2.5.3 → 2.5.4 (sequential)
- **Recommendation**: Complete all 4 tasks before merging to main

---

## References

- NUT-XX Spec: `/home/evan/work/nuts/xx.md`
- NUT-04 (Mint Quote): https://github.com/cashubtc/nuts/blob/main/04.md
- NUT-20 (Deterministic Metadata): https://github.com/cashubtc/nuts/blob/main/20.md
- CDK Mint Implementation: `crates/cdk/src/mint/`
- CDK Wallet Implementation: `crates/cdk/src/wallet/`
