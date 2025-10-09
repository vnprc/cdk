# Hashpool Rebase & Mining-Share Alignment Plan

## Goals
- Cut the `hashpool5` branch down to the mining-share feature set (mint + wallet + protocol glue)
  while eliminating legacy drift (MultiMint wallet, CLI refactors, pub/sub copies, docs).
- Layer the mining-share functionality onto current upstream abstractions so it can merge cleanly.
- Rebase onto the latest upstream `main` to produce `hashpool6`, keeping the mining-share flow intact.

## Assumptions
- Hashpool does not consume the CDK pub/sub manager or CLI; those edits can be dropped.
- The `nutXX` module naming stays because the mining-share spec has no official NUT number yet.
- Smoke testing happens through the hashpool stack; automated coverage relies on upstream CI.

## Phase 1 – Hashpool5 Cleanup (current branch)
1. **Remove drifted surfaces**
   - Restore upstream `crates/cdk/src/wallet/multi_mint_wallet.rs` and dependent CLI modules.
   - Delete the legacy pub/sub manager and associated rewrites (we do not use them downstream).
   - Drop branch-only docs (`CLI_REFACTOR.md`, etc.) once replacements are captured elsewhere.
2. **Isolate mining-share logic**
   - Keep `wallet/issue/issue_mining_share.rs`, HTTP client endpoints, mint issue flow, `nutXX`
     structs, and translator/mint protocol glue.
   - Refactor these pieces to extend the upstream code (traits/feature flags) instead of replacing
     whole files.
3. **Schema alignment**
   - Ensure only the `mint_quote.keyset_id` addition remains, delivered via the existing
     `20250815`/`20250820` migrations for mint and wallet SQLite DBs.
4. **Validation**
   - `cargo test --workspace` (or the subset touched) and a quick translator/mint smoke run to
     confirm mining-share quote creation + minting still works after the cleanup.

## Phase 2 – Rebase to `hashpool6`
1. Fetch upstream and branch: `git fetch upstream main && git checkout -b hashpool6 upstream/main`.
2. Replay the reduced mining-share commits (types → HTTP client → mint → wallet → tests), resolving
   conflicts against the latest upstream structure.
3. Run unit tests plus the hashpool smoke workflow (translator ↔ mint) to verify mining-share
   issuance.
4. Force-push the rebased branch and keep the old branch for reference until the new one is stable.

## Post-Rebase Follow-Ups
- Upstream any generally useful fixes (e.g., bitcoin 0.32 import tweaks, SQL select corrections)
  as separate PRs.
- Monitor upstream for official NUT assignment; rename `nutXX` when the spec lands.
- Periodically re-run `just update-cdk` + smoke tests to ensure the hashpool stack stays green.
