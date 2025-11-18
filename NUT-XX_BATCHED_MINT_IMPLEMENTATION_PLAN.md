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

## Phase 2: Wallet-Side Implementation

### Task 2.1: Wallet Batch Minting Client

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

### Task 2.2: Testing (Wallet)

**Objective**: Test wallet batch minting client.

**Tests**:
- Unit tests: secret/message generation, validation
- Integration tests: full workflow with mock mint
- Error handling: network failures, invalid quotes, expired quotes
- Idempotency: batch minting same quotes twice
- Proof storage: proofs correctly stored in database

**Files to Create/Modify**:
- `crates/cdk/tests/wallet_batch_mint.rs` (new test file)

---

## Phase 2.5: NUT-20 Support (Deferred but Required)

**Note**: NUT-20 integration can be deferred to after Phase 2 is complete and tested, but is a hard requirement before merging to main.

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

### Task 2.5.2: Mint-Side NUT-20

**Changes to Batch Mint Endpoint**:
1. Accept `signature: Vec<Option<String>>` in request
2. Validation: if any signature non-null, length must match quotes length
3. For each (quote, signature) pair:
   - If signature is null: continue (unlocked quote)
   - If signature is non-null: verify signature matches locked quote's pubkey
   - Verify signature is valid (existing NUT-20 crypto)
4. Generate blind signatures only after all validations pass

**Files to Modify**:
- `crates/cdk/src/mint/issue/mod.rs` (NUT-20 validation)
- `crates/cdk-axum/src/router_handlers.rs` (signature parsing)

---

### Task 2.5.3: Wallet-Side NUT-20

**Changes to `mint_batch()`**:
1. Detect which quotes are NUT-20 locked (requires quote details)
2. For each locked quote: generate NUT-20 signature using wallet's key
3. Build signature array with nulls for unlocked, signatures for locked
4. Include signature array in batch mint request

**Files to Modify**:
- `crates/cdk/src/wallet/issue/mod.rs` (signature generation)

---

### Task 2.5.4: NUT-20 Testing

**Tests**:
- Mint: batch with all locked quotes, all unlocked, mixed
- Wallet: signature generation, request building
- Signature validation: invalid signatures rejected
- Array length validation: mismatched array lengths rejected

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

- ✅ All Phase 1 tests pass (13/13 tests passing)
- ⏳ All Phase 2 tests pass
- ⏳ Backward compatibility: single mint still works identically
- ⏳ Performance: batch mint 100 quotes < 2x time of single quote
- ⏳ No new security vulnerabilities introduced
- ⏳ Code review approval
- ⏳ NUT-20 support implemented (even if deferred, must be complete)

---

## Timeline Estimate

- Phase 1: 3-4 days (design, endpoints, testing)
- Phase 2: 2-3 days (wallet client, testing)
- Phase 2.5: 2-3 days (NUT-20 integration, testing)
- Phase 3: 1-2 days (documentation, integration tests)
- **Total**: 8-12 days

---

## References

- NUT-XX Spec: `/home/evan/work/nuts/xx.md`
- NUT-04 (Mint Quote): https://github.com/cashubtc/nuts/blob/main/04.md
- NUT-20 (Deterministic Metadata): https://github.com/cashubtc/nuts/blob/main/20.md
- CDK Mint Implementation: `crates/cdk/src/mint/`
- CDK Wallet Implementation: `crates/cdk/src/wallet/`
