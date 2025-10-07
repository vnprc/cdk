# Pending Quotes Query Optimization Development Plan

## Problem Statement
The current `get_mint_quotes()` method in CDK performs a full table scan (SELECT * with no WHERE clause), causing slow queries (22ms+) that worsen as the database grows. The proof sweeper in hashpool only needs quotes with pending mintable amounts but must fetch ALL quotes and filter in-memory.

## Root Cause
- The `state` column was removed from the database in the bolt12 migration
- State is now computed dynamically: `amount_paid > amount_issued` = "Paid" state
- No optimized query exists for fetching only pending/paid quotes

## Development Plan

### 1. Add New Database Method
**File:** `crates/cdk-common/src/database/wallet.rs`
- Add trait method: `async fn get_pending_mint_quotes(&self) -> Result<Vec<WalletMintQuote>, Self::Err>;`
- Method should return quotes where `amount_paid > amount_issued`

### 2. Implement for SQLite Backend
**File:** `crates/cdk-sql-common/src/wallet/mod.rs`
```rust
async fn get_pending_mint_quotes(&self) -> Result<Vec<MintQuote>, Self::Err> {
    let conn = self.pool.get().map_err(|e| Error::Database(Box::new(e)))?;
    Ok(query(
        r#"
        SELECT
            id, mint_url, amount, unit, request, state, expiry,
            secret_key, payment_method, amount_issued, amount_paid, keyset_id
        FROM mint_quote
        WHERE amount_paid > amount_issued
        AND amount_paid > 0
        ORDER BY created_time ASC
        LIMIT 100
        "#,
    )?
    .fetch_all(&*conn)
    .await?
    .into_iter()
    .map(sql_row_to_mint_quote)
    .collect::<Result<Vec<_>, _>>()?)
}
```

### 3. Implement for Other Backends
- **ReDB backend:** `crates/cdk-redb/src/wallet/mod.rs`
- **FFI backend:** `crates/cdk-ffi/src/database.rs`
- Each implementation should filter for `amount_paid > amount_issued` efficiently

### 4. Add Database Index
**File:** `crates/cdk-sql-common/src/wallet/migrations/sqlite/[new_migration].sql`
```sql
CREATE INDEX IF NOT EXISTS idx_mint_quote_pending 
ON mint_quote(amount_paid, amount_issued)
WHERE amount_paid > amount_issued;
```

### 5. Update Wallet Implementation
**File:** `crates/cdk/src/wallet/issue/issue_bolt11.rs`
- Add public method to expose the optimized query:
```rust
pub async fn get_pending_mint_quotes(&self) -> Result<Vec<MintQuote>, Error> {
    let mut pending_quotes = self.localstore.get_pending_mint_quotes().await?;
    let unix_time = unix_time();
    pending_quotes.retain(|quote| {
        quote.mint_url == self.mint_url
            && quote.expiry > unix_time
    });
    Ok(pending_quotes)
}
```

### 6. Update Hashpool Proof Sweeper
**File:** `roles/translator/src/lib/mod.rs`
- Replace `wallet.localstore.get_mint_quotes()` with `wallet.get_pending_mint_quotes()`
- Remove in-memory filtering since database now handles it

## Expected Performance Improvement
- **Current:** 22ms+ for full table scan of all quotes
- **Expected:** 1-2ms for indexed query of only pending quotes
- **Benefit:** Query time remains constant as database grows

## Testing Plan
1. Benchmark query performance before/after with 10,000+ quotes
2. Verify correct quotes are returned (amount_paid > amount_issued)
3. Test with edge cases:
   - No pending quotes
   - All quotes pending
   - Mixed states
4. Ensure backwards compatibility with existing code

## Migration Strategy
1. Add new method alongside existing `get_mint_quotes()`
2. Update hashpool to use new method
3. Existing code continues to work with old method
4. Gradually migrate other use cases if applicable

## Alternative Solutions (Not Recommended)
- **Re-add state column:** Would require significant migration and duplicate state tracking
- **Cache in application:** Adds complexity and potential staleness issues
- **Periodic cleanup:** Doesn't solve the root performance issue

## Success Criteria
- [ ] Query time reduced from 22ms to <2ms for typical workloads
- [ ] No functional regression in quote processing
- [ ] Database migration applies cleanly
- [ ] All backend implementations updated consistently