# Ehash Extraction Plan (No NUT-XX, Minimal Core Changes)

This plan assumes:
- No NUT-XX terminology or spec updates.
- Batched minting is already in `main`.
- We minimize fork diffs by using **custom payment methods**, **custom units**, and **custom request/response extras** already supported in `main`.

---

## 1) Baseline + Branch Strategy

- Create a fresh branch from `origin/main` (e.g., `ehash-extract`).
- Keep `hashpool-batched-minting` only as a reference for mining-share logic to transplant.
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
    mint.rs            // payment processor + mint‑side helpers
    wallet.rs          // wallet helpers + batch mint convenience
    axum.rs            // optional: only if you later want custom extra handlers
    ffi.rs (optional)  // separate FFI surface if needed
```

### `types.rs`
- Define `EhashQuoteRequest`, `EhashQuoteResponse`, `EhashBatchEntry`.
- Provide conversions to/from:
  - `cdk::nuts::MintQuoteCustomRequest`
  - `cdk::nuts::MintQuoteCustomResponse`
- Handle `extra` JSON packing/unpacking.

### `mint.rs`
- Implement `EhashPaymentProcessor: MintPayment`.
- Provide `EhashMintConfig` + `build_ehash_processor(...)`.

### `wallet.rs`
- Helpers wrapping `MintConnector`:
  - `create_ehash_quote(...) -> MintQuoteCustomResponse`
  - `check_ehash_quote(...)`
  - `mint_ehash(...)`
  - `batch_mint_ehash(...)` (uses batch mint endpoint on `main`)
- Add batch validation for matching `keyset_id` in `extra` if required.

### `axum.rs` (optional)
- Do **not** keep legacy `/mint/quote/mining_share` routes.
- Rely solely on existing custom routes:
  - `POST /v1/mint/quote/ehash`
  - `GET /v1/mint/quote/ehash/{id}`
  - `POST /v1/mint/ehash`
  - `POST /v1/mint/ehash/batch`

---

## 4) Mint Payment Processor Design (No Core Changes)

Implement a custom payment processor inside `cdk-ehash`:

- `get_settings()`:
  - Return `SettingsResponse { custom: {"ehash": "<json or empty>"}, unit: "EHASH", ... }`.

- `create_incoming_payment_request(...)`:
  - Parse `CustomIncomingPaymentOptions.extra_json` for `header_hash`.
  - Validate header hash format and not‑all‑zero.
  - Return `CreateIncomingPaymentResponse` with:
    - `request_lookup_id = CustomId(header_hash_hex)`
    - `request = header_hash_hex` (or a short formatted request string)
    - `expiry = Some(unix_expiry)`
    - `extra_json = Some({ header_hash, keyset_id?, amount_issued? })`

- `check_incoming_payment_status(payment_identifier)`:
  - Minimal approach: resolve status from mint quotes using
    `DynMintDatabase::get_mint_quote_by_request_lookup_id`.
  - Return “paid” when quote exists for this identifier.

- `wait_payment_event()`:
  - Can be empty stream or immediate “paid” events if async updates needed.

---

## 5) Wire Into Mint Startup (Minimal)

- In `cdk-mintd` (or mint binary), register ehash processor:
  - `MintBuilder::add_payment_processor(...)` with the ehash `MintPayment`.
- Ensure `create_mint_router(..., custom_methods)` includes `"ehash"` to enable custom routes.
- No core API changes required.

---

## 6) Wallet Integration (No Core Changes)

Use existing `MintConnector` methods:

- `post_mint_quote(MintQuoteRequest::Custom { method: "ehash", request })`
- `get_mint_quote_status(PaymentMethod::Custom("ehash"), quote_id)`
- `post_mint(PaymentMethod::Custom("ehash"), MintRequest { ... })`
- `post_batch_mint(PaymentMethod::Custom("ehash"), BatchMintRequest { ... })`

`cdk-ehash::wallet` provides typed wrappers around these.

---

## 7) Remove Mining-Share Code From Fork

On the new branch, **do not carry** any of the old mining‑share artifacts into `main`:

- `cashu` NUT‑XX / mining_share types
- `PaymentMethod::MiningShare` / `PaymentIdentifier::MiningShareHash`
- mining_share endpoints in `cdk-axum`
- mining_share wallet helpers in `cdk`
- mining_share FFI types / subscription kinds
- mining_share migrations (keyset_id on mint quotes)

This shrinks the fork diff to just the new crate + wiring.

---

## 8) Reapply onto Latest `main`

- Rebase or reapply only `cdk-ehash` and minimal wiring on top of `origin/main`.
- Conflicts should be limited to workspace `Cargo.toml` and mintd wiring.

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

## Implementation Checklist (Optional)

1. Add `crates/cdk-ehash` to workspace.
2. Scaffold modules + types.
3. Implement `EhashPaymentProcessor`.
4. Wire into mintd + custom methods list.
5. Implement wallet helper API.
6. Add basic tests.
7. Rebase onto latest `main`.
