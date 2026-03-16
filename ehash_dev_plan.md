# Ehash Extraction Plan (No NUT-XX, Minimal Core Changes)

This plan assumes:
- No NUT-XX terminology or spec updates.
- Batched minting is already in `main` (NUT-29, merged upstream March 9 2026).
- We minimize fork diffs by using **custom payment methods**, **custom units**, and **`extra_json`** supported in `main`.

**Goal:** Extract all ehash logic into a standalone `cdk-ehash` crate that can be released and
maintained independently of this fork. Hashpool then imports `cdk-ehash` + stock upstream cdk,
removes the cdk fork and all vendored cdk code.

---

## Upstream Status (as of 2026-03-16)

Several features that were originally planned as Phase 2 work have already landed in `origin/main`:

| Feature | Upstream commit | Status |
|---------|----------------|--------|
| `PaymentMethod::Custom(String)` | `255db0c3` | ✅ In main |
| `CustomRouter` / `CustomHandlers` in `cdk-axum` | `255db0c3` | ✅ In main |
| NUT-29 batch minting | `c78329af` (2026-03-09) | ✅ In main |
| `extra_json: Option<Value>` field on `MintQuote` | `c78329af` | ✅ In main (struct only) |
| `extra_json` DB persistence | — | ❌ Not yet — field exists, no column/migration |
| `keyset_id` on `MintQuote` | fork-only | ❌ Not in upstream main |

**Key implication:** Phase 2 custom-method infrastructure is largely already in upstream. The thin
patch is smaller than originally anticipated. The main remaining work is:
1. Wiring `extra_json` to the DB (small upstream PR).
2. Cutting `cdk-ehash` free of mining-share types using the upstream custom-method API.

---

## 1) Baseline + Branch Strategy (Phased)

**Chosen strategy: thin patch + upstream PR**

- Run `cdk-ehash` on top of a minimal patch applied to the latest `origin/main`.
- Simultaneously open a PR upstream for the generic parts (extra_json DB persistence, any
  remaining custom-method gaps) so the patch can eventually be deleted.
- The old fork is kept only as a reference for mining-share logic; no new work goes there.

**Why thin patch over waiting for upstream:**
- Gets hashpool off the fork sooner.
- Patch is small enough to rebase on each upstream release.
- Once upstream accepts the generic changes, the patch disappears.

**Branch naming:**
- `ehash-phase1` — Phase 1 complete, delegates to mining-share (current: `ehash-plan`)
- `ehash-phase2` — fresh branch from `origin/main` + thin patch, no mining-share

---

## 2) Runtime Identifiers + Data Model

**Custom payment method:** `PaymentMethod::Custom("ehash")` (already in upstream nut00)

**Custom currency unit:** `CurrencyUnit::custom("EHASH")` (already in upstream)

**Request identifier:** `PaymentIdentifier::CustomId(<header_hash_hex>)`

**keyset_id storage:** Pack into `MintQuote.extra_json` as `{"keyset_id": "<hex>"}`.
- Do NOT use the fork's dedicated `keyset_id: Option<Id>` column.
- Upstream already has the struct field; the DB column just needs to be added (see Section 4A).

**Extra JSON fields** (in ehash quote request/response):
- Required: `header_hash` (hex string)
- Mint-assigned: `keyset_id` (returned in response, read from `extra_json`)
- Optional: `amount_issued`, `share_height`, `pool_id`

---

## 3) New Crate: `crates/cdk-ehash` — Current State

The crate was scaffolded in commit `152c54bb` with the following module layout (complete):

```
crates/cdk-ehash/
  src/
    lib.rs       ✅ done
    types.rs     ✅ done — EhashQuoteRequest, EhashQuoteResponse, EhashBatchEntry
    mint.rs      ✅ done — create_ehash_quote, get_ehash_quote
    wallet.rs    ✅ done — EhashWalletClient<T>
    axum.rs      ✅ done — 4 routes wired
```

**Current coupling (must be removed in Phase 2):**

| File | Coupling point | What to replace with |
|------|---------------|----------------------|
| `types.rs:4-5` | imports `nutXX::MintQuoteMiningShareRequest/Response` | upstream `MintQuoteRequest`/custom fields |
| `mint.rs:24,35` | calls `mint.create_mint_mining_share_quote()` / `get_mint_mining_share_quote()` | upstream custom-method quote API |
| `axum.rs:109` | `PaymentMethod::MiningShare` in batch handler | `PaymentMethod::Custom("ehash")` |

Until these three coupling points are eliminated, `cdk-ehash` cannot work with stock cdk.

### Module responsibilities (unchanged)

**`types.rs`:** `EhashQuoteRequest`, `EhashQuoteResponse<Q>`, `EhashBatchEntry`.
Pack/unpack `keyset_id` into `extra_json` (replacing direct `nutXX` conversions).

**`mint.rs`:** Mint-side helpers using upstream custom-method quote API.

**`wallet.rs`:** `EhashWalletClient<T>` — `create_ehash_quote`, `check_ehash_quote`,
`mint_ehash`, `batch_mint_ehash`. Validate `keyset_id` in response.

**`axum.rs`:** Four ehash endpoints delegating to `cdk-ehash::mint` helpers.
Phase 2: swap `PaymentMethod::MiningShare` → `PaymentMethod::Custom("ehash")`.

---

## 4) Thin Patch Contents (Phase 2)

The thin patch is applied on top of `origin/main` and contains only generic, upstreamable changes.
It must contain **zero** ehash-specific types.

### 4A) `extra_json` DB persistence (required, small)

Upstream added `extra_json: Option<serde_json::Value>` to `MintQuote` but never wired it to the
DB. The thin patch adds:

- Migration: `ALTER TABLE mint_quote ADD COLUMN extra TEXT;` (sqlite + postgres)
- `quotes.rs` read: deserialize `extra` column → `extra_json`
- `quotes.rs` write: serialize `extra_json` → `extra` column
- Open as a standalone upstream PR — no ehash mention needed.

**Drop from fork:** The dedicated `keyset_id: Option<Id>` field on `MintQuote` and its migration
(`20250815069420_add_keyset_id_to_mint_quote.sql`).

### 4B) Custom-method mint quote API on `Mint` (required if gap exists)

Verify whether upstream's `Mint` struct exposes a generic
`create_mint_quote(PaymentMethod::Custom(...), ...)` path that `cdk-ehash::mint` can call.
If not, add a thin generic method — no mining-share types.

### 4C) Auth routing for custom endpoints (if needed)

Upstream's `CustomRouter` may already handle auth. Verify `cdk-ehash`'s axum routes are
properly covered. If a gap exists, extend `RoutePath` with a pattern-based hook.

---

## 5) Wire Into Mint Startup

**Phase 1 (done):** `create_ehash_router` merged into `cdk-mintd/src/lib.rs`.

**Phase 2:**
- Remove old mining-share auth endpoint wiring from `cdk-mintd/src/lib.rs` (lines ~734–771).
- Verify the ehash axum router integrates cleanly with upstream's `CustomRouter` infrastructure.

---

## 6) Wallet Integration

**Phase 1 (done):** `EhashWalletClient<T>` in `cdk-ehash::wallet` with direct HTTP calls.

**Phase 2:** Replace mining-share connector methods with upstream custom-method equivalents:
- `post_mint_quote(PaymentMethod::Custom("ehash"), request)`
- `get_mint_quote_status(PaymentMethod::Custom("ehash"), quote_id)`
- `post_mint(PaymentMethod::Custom("ehash"), MintRequest { ... })`
- `post_batch_mint(PaymentMethod::Custom("ehash"), BatchMintRequest { ... })` (NUT-29)

---

## 7) Remove Mining-Share Code From Fork (Phase 2)

On the Phase 2 branch, **do not carry** any of these fork artifacts:

**`cashu` crate:**
- `nutXX.rs` (entire file, 456 lines) — all NUT-XX types
- `nuts/mod.rs` — nutXX re-exports
- `nut17/mod.rs` — `Kind::MiningShareMintQuote` variant
- `nut19.rs` — `Path::MintMiningShare`, `MintMiningShareBatch`, `MintQuoteMiningShare` variants

**`cdk-common` crate:**
- `payment.rs` — `PaymentIdentifier::MiningShareHash` variant
- `mint.rs` — `MintQuote.keyset_id` field + all `TryFrom<MintQuote>` for `MintQuoteMiningShareResponse`
- `subscription.rs` — `Kind::MiningShareMintQuote` handling
- `ws.rs` — `NotificationPayload::MintQuoteMiningShareResponse` conversion

**`cdk` crate:**
- `wallet/mod.rs` — `MiningShareBatchEntry` struct, `WalletSubscription::MiningShareMintQuoteState`
- `wallet/issue/issue_mining_share.rs` — entire file (333 lines, 4 dedicated functions)
- `wallet/mint_connector/mod.rs` — 3 mining-share methods from `MintConnector` trait
- `wallet/mint_connector/http_client.rs` — 5 mining-share HTTP implementations
- `mint/issue/mod.rs` — `MintQuoteRequest::MiningShare` variant and handling
- `mint/subscription.rs` — `NotificationId::MintQuoteMiningShare` handling
- `event.rs` — `MintQuoteMiningShareResponse` notification wrapping

**`cdk-axum` crate:**
- `router_handlers.rs` — mining-share route handlers and imports

**`cdk-ffi` crate:**
- `cdk-ffi` generates UniFFI bindings for Python, Swift, and Kotlin so that native mobile app
  developers (Swift/Kotlin) can use CDK as a compiled native library instead of reimplementing
  Cashu cryptography.
- `cdk-ehash` does NOT need FFI bindings. Miner self-custody wallets talk to the mint over
  HTTP/JSON — they do not link against Rust code. Any Cashu wallet that can make HTTP requests
  can receive ehash rewards without knowing anything about `cdk-ehash` internals.
- The FFI removals below are about **shrinking the fork diff**, not about making `cdk-ehash`
  independent. They can be deferred without blocking the Phase 3 independence goal.
- If a mobile wallet developer later wants first-class ehash support (e.g. show `amount_issued`,
  `share_height` natively), that is a separate future crate (`cdk-ehash-ffi`) wrapping
  `cdk-ehash`. Out of scope for now.
- Items to remove from fork when cleaning up (non-blocking):
  - `types/subscription.rs` — `SubscriptionKind::MiningShareMintQuote` variant
  - `types/quote.rs` — `MintQuote.keyset_id` FFI field
  - `types/mint.rs` — mining-share FFI types

**`cdk-mintd`:**
- `lib.rs` lines ~734–771 — mining-share auth endpoint wiring

**`cdk-sql-common` migrations:**
- `20250815069420_add_keyset_id_to_mint_quote.sql` (mint, sqlite)
- `20251010120000_add_keyset_id_to_mint_quote.sql` (mint, postgres)
- `20250820042069_add_keyset_id_to_mint_quote.sql` (wallet, sqlite)
- `20251010121000_add_keyset_id_to_mint_quote.sql` (wallet, postgres)

This shrinks the fork diff to the thin patch (Section 4) plus the `cdk-ehash` crate itself.

---

## 8) Reapply onto Latest `main`

**Phase 1.5 (before rebase — blockers to fix first):**

Pre-existing compile errors on the current fork branch must be resolved before any rebase:

1. `cdk-ffi` — `get_unpaid_mint_quotes` added to `WalletDatabase` trait but not implemented in
   `database.rs` (mock), `sqlite.rs`, and `postgres.rs` FFI wrappers.
2. `batch.rs` test — 9-argument `MintQuote::new()` call is missing the `keyset_id` `Option<Id>`
   argument.
3. `cashu/nuts/mod.rs` — `#[warn(non_snake_case)]` on `nutXX` module name (rename to `nut_xx`
   or suppress).

**Phase 2 rebase instructions:**
1. Fix Phase 1.5 blockers on current branch.
2. Create `ehash-phase2` from `origin/main`.
3. Apply thin patch (Section 4).
4. Cherry-pick / rewrite `cdk-ehash` with mining-share coupling removed.
5. Drop all mining-share artifacts listed in Section 7.
6. Validate full-stack: create quote → check status → mint → batch mint.

---

## 9) Tests

**Unit tests** in `cdk-ehash` (partial — exists in `types.rs`):
- ✅ `parse_header_hash` rejects invalid input
- ✅ `mining_share_conversion_sets_state` (to be removed/replaced in Phase 2)
- ✅ `ehash_request_converts_to_mining_share` (to be removed/replaced in Phase 2)
- ❌ `extra_json` packing/unpacking for `keyset_id`
- ❌ Custom unit normalization round-trip (`EHASH`)

**Integration tests** (none yet — required before Phase 2):
- Create ehash quote → check status → mint tokens
- Batch mint with multiple quotes (NUT-29 path)
- Verify custom routes respond correctly for `ehash` method
- Verify `keyset_id` survives the round-trip through `extra_json`

---

## 10) Extract and Publish `cdk-ehash` (Phase 3)

Once Phase 2 is stable and `cdk-ehash` has zero nutXX/mining-share imports:

1. Move `crates/cdk-ehash` into its own git repository.
2. Declare `cdk = "X.Y"` (upstream release, no patch) as a dependency in its `Cargo.toml`.
3. Publish to crates.io as `cdk-ehash`.
4. Update hashpool to depend on `cdk-ehash = "X.Y"` + `cdk = "X.Y"` (stock, no fork).
5. Remove vendored cdk fork and all hashpool workarounds for fork dependencies.

**Independence gate (definition of done for Phase 2):**
`cdk-ehash` compiles with `cdk` and `cdk-common` pointing at the upstream release crate, with
zero references to `nutXX`, `MiningShare`, or `mining_share` anywhere in its source tree.

---

## 11) Documentation / Migration Notes

- Endpoints use custom-method style (no legacy paths supported):
  - `POST /v1/mint/quote/ehash`
  - `GET  /v1/mint/quote/ehash/{id}`
  - `POST /v1/mint/ehash`
  - `POST /v1/mint/ehash/batch`
- Existing mining-share quotes in old DB are not forward-compatible. No migration needed —
  drop the old DB and start a fresh instance on Phase 2 deployment.

---

## Implementation Checklist

### Phase 1 (complete)
1. ✅ Add `crates/cdk-ehash` to workspace.
2. ✅ Scaffold modules + types (`lib.rs`, `types.rs`, `mint.rs`, `wallet.rs`, `axum.rs`).
3. ✅ Implement ehash endpoints delegating to mining-share logic.
4. ✅ Wire `create_ehash_router` into `cdk-mintd`.
5. ✅ Implement `EhashWalletClient<T>` wallet helper API.
6. ⚠️  Add basic tests (unit tests exist; integration tests missing).
7. ❌ Rebase onto latest `main` (blocked by Phase 1.5).

### Phase 1.5 (pre-rebase blockers)
1. ❌ Fix `get_unpaid_mint_quotes` missing from FFI `WalletDatabase` implementations.
2. ❌ Fix `batch.rs` test — missing `keyset_id` argument in `MintQuote::new()` call.
3. ❌ Fix `nutXX` non-snake-case warning in `cashu/nuts/mod.rs`.

### Phase 2 (custom method — thin patch on origin/main)
1. ❌ Verify upstream `Mint::create_mint_quote` handles `PaymentMethod::Custom` (gap check).
2. ❌ Add `extra_json` DB column + migration to thin patch (open as standalone upstream PR).
3. ❌ Remove three mining-share coupling points from `cdk-ehash` (types.rs, mint.rs, axum.rs).
4. ❌ Replace `keyset_id` field usage with `extra_json["keyset_id"]` packing in `cdk-ehash`.
5. ❌ Verify NUT-29 batch mint works with `PaymentMethod::Custom("ehash")`.
6. ❌ Remove mining-share auth wiring from `cdk-mintd` (lines ~734–771).
7. ❌ Remove all mining-share artifacts listed in Section 7.
8. ❌ Write missing unit tests (extra_json round-trip, custom unit normalization).
9. ❌ Write integration tests (full create → check → mint → batch flow).
10. ❌ Verify `cdk-ehash` independence gate: zero nutXX/mining-share imports.

### Phase 3 (extract and publish)
1. ❌ Extract `cdk-ehash` into standalone git repository.
2. ❌ Point `Cargo.toml` at upstream cdk release (not fork).
3. ❌ Publish `cdk-ehash` to crates.io.
4. ❌ Update hashpool to use published `cdk-ehash` + stock `cdk`.
5. ❌ Remove cdk fork and vendored code from hashpool.
