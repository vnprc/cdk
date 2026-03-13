# Ehash Extraction Plan (No NUT-XX, Minimal Core Changes)

This plan assumes:
- No NUT-XX terminology or spec updates.
- Batched minting is already in `main`.
- We minimize fork diffs by using **custom payment methods**, **custom units**, and **custom request/response extras** supported in `main`.

---

## 1) Baseline + Branch Strategy (Phased)

**Decision (Phase 1 first):**
1. Implement a minimal `cdk-ehash` crate that exposes ehash endpoints **backed by the existing mining-share flow** (no core changes).
2. Commit and re-apply/cherry-pick onto a fresh branch from `origin/main`.
3. Verify full stack on latest `main` before Phase 2.

**Why:** small diff, low conflict surface, and lets us validate behavior on up-to-date `main` quickly.

**Branch instructions (Phase 1):**
- Create a fresh branch from `origin/main` (e.g., `ehash-phase1`).
- Keep the old fork only as a reference for mining-share logic to transplant.
- Target near‑zero diffs in core crates (`cashu`, `cdk`, `cdk-common`, `cdk-axum`, `cdk-ffi`, `cdk-sql-common`).

---

## 2) Runtime Identifiers + Data Model

**Custom payment method:** `ehash` (lowercase, URL‑safe)

**Custom currency unit:** `CurrencyUnit::custom("EHASH")` (uppercase normalization already in main)

**Request identifier:** `PaymentIdentifier::CustomId(<header_hash_hex>)`

**Extra JSON fields** (in `MintQuoteCustomRequest` / `MintQuoteCustomResponse`):
- Required: `header_hash` (hex string)
- Optional (only if needed): `keyset_id`, `amount_issued`, `share_height`, `pool_id`

Rationale: `main` already supports custom methods, custom units, and `extra` JSON fields.

---

## 3) New Crate: `crates/cdk-ehash`

Create a new workspace crate with a clear module layout:

```
crates/cdk-ehash/
  src/
    lib.rs
    types.rs           // request/response structs, extra JSON parsing
    mint.rs            // mint‑side helpers
    wallet.rs          // wallet helpers + batch mint convenience
    axum.rs            // ehash routes
    ffi.rs (optional)  // separate FFI surface if needed
```

### `types.rs`
- Define `EhashQuoteRequest`, `EhashQuoteResponse`, `EhashBatchEntry`.
- Provide conversions to/from:
  - `cdk::nuts::MintQuoteCustomRequest`
  - `cdk::nuts::MintQuoteCustomResponse`
- Handle `extra` JSON packing/unpacking.

### `mint.rs`
- Provide mint‑side helpers that map to the underlying quote logic.

### `wallet.rs`
- Helpers wrapping mint HTTP:
  - `create_ehash_quote(...)`
  - `check_ehash_quote(...)`
  - `mint_ehash(...)`
  - `batch_mint_ehash(...)`
- Add batch validation for matching `keyset_id` in `extra` if required.

### `axum.rs`
- Provide ehash endpoints:
  - `POST /v1/mint/quote/ehash`
  - `GET /v1/mint/quote/ehash/{id}`
  - `POST /v1/mint/ehash`
  - `POST /v1/mint/ehash/batch`
- **Phase 1:** these endpoints delegate to mining‑share logic to keep core untouched.
- **Phase 2:** these endpoints use custom-method core handling (no mining‑share).

---

## 4) Phase 2: Custom-Method Implementation (Core Changes)

**Goal:** Stop depending on mining-share/NUT-XX entirely by using **custom payment method + extras**.

**Approach (recommended):**
1. **Core types**:
   - Add `MintQuoteCustomRequest` / `MintQuoteCustomResponse` in `cashu` (likely `nut04` or a new NUT-XX-free module).
   - Include `extra: serde_json::Value` for fields like `header_hash`, `keyset_id`, `amount_issued`, `share_height`, `pool_id`.
2. **Mint routing**:
   - Add generic custom-method mint quote/mint endpoints in `cdk-axum`:
     - `POST /v1/mint/quote/{method}`
     - `GET /v1/mint/quote/{method}/{id}`
     - `POST /v1/mint/{method}`
     - `POST /v1/mint/{method}/batch`
   - Ensure `PaymentMethod::Custom(method)` is used.
3. **Mint core**:
   - Extend `MintQuoteRequest/Response` to include a `Custom` variant.
   - Add validation helpers for custom requests (e.g., required fields in `extra`).
4. **Wallet connector**:
   - Add generic “custom method” HTTP calls in `MintConnector`.
   - Avoid new dedicated methods per custom type; use a single custom flow.
5. **Auth routing**:
   - Extend `RoutePath` or add a pattern-based auth hook so custom endpoints can be protected without adding one enum variant per custom method.
6. **FFI**:
   - Add a single custom-mint-quote API surface if needed, using `extra` JSON.

**Why this design:**
- Enables upstreaming without baking in ehash-specific types.
- Removes all mining-share/NUT-XX code from your fork once ehash is fully custom.

---

## 5) Wire Into Mint Startup

**Phase 1:** Wire the `cdk-ehash` axum router into `cdk-mintd`.

**Phase 2:** Register custom-method processor and ensure `ehash` is included in custom route handling.

---

## 6) Wallet Integration

**Phase 1:** Use explicit ehash endpoints from `cdk-ehash::wallet`.

**Phase 2:** Use a generic custom-method connector:
- `post_mint_quote(MintQuoteRequest::Custom { method: "ehash", request })`
- `get_mint_quote_status(PaymentMethod::Custom("ehash"), quote_id)`
- `post_mint(PaymentMethod::Custom("ehash"), MintRequest { ... })`
- `post_batch_mint(PaymentMethod::Custom("ehash"), BatchMintRequest { ... })`

---

## 7) Remove Mining-Share Code From Fork (Phase 2+)

On the Phase 2 branch, **do not carry** any of the old mining‑share artifacts into `main`:
- `cashu` NUT‑XX / mining_share types
- `PaymentMethod::MiningShare` / `PaymentIdentifier::MiningShareHash`
- mining_share endpoints in `cdk-axum`
- mining_share wallet helpers in `cdk`
- mining_share FFI types / subscription kinds
- mining_share migrations (keyset_id on mint quotes)

This shrinks the fork diff to just the new crate + wiring + generic custom-method support (which can be upstreamed).

---

## 8) Reapply onto Latest `main`

**Phase 1 instructions:**
- Commit the Phase 1 changes in this fork.
- Create a fresh branch off `origin/main`.
- Cherry-pick the Phase 1 commit(s).
- Validate full-stack behavior on latest `main`.

**Phase 2 instructions:**
- Implement custom-method support on a new branch off `origin/main`.
- Once stable, drop mining-share/NUT-XX usages and remove legacy endpoints.

---

## 9) Tests

**Unit tests** in `cdk-ehash`:
- JSON packing/unpacking for `extra`.
- Validation of `header_hash`.
- Custom unit normalization (`EHASH`).

**Integration tests** (required):
- Create quote → check status → mint.
- Batch mint with multiple quotes.
- Ensure custom routes respond for `ehash`.

---

## 10) Documentation / Migration Notes

- New endpoints are custom‑method style:
  - `/v1/mint/quote/ehash`
  - `/v1/mint/ehash`
- Do **not** support legacy paths; update mining side to use the new custom URLs.
- Existing mining‑share quotes in old DB won’t be recognized unless explicitly migrated; plan for clean cutover or a one‑time migration tool.

---

## Implementation Checklist (Phased)

**Phase 1 (now):**
1. Add `crates/cdk-ehash` to workspace.
2. Scaffold modules + types.
3. Implement ehash endpoints (delegating to mining-share logic).
4. Wire into mintd.
5. Implement wallet helper API.
6. Add basic tests.
7. Rebase onto latest `main`.

**Phase 2 (custom method):**
1. Add core custom request/response types with `extra` JSON.
2. Wire custom-method routes in `cdk-axum`.
3. Extend `MintQuoteRequest/Response` in `cdk`.
4. Add generic custom-method support in `MintConnector`.
5. Add auth handling for custom routes.
6. Update `cdk-ehash` to use custom method (drop mining-share dependency).
7. Remove NUT-XX/mining-share artifacts from fork.
