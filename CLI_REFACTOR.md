# 🔧 CDK Balance Command Refactor Plan

## 📋 Current Problems Identified

### 1. 🔍 Unit Filtering Issue
The balance command only queries for `CurrencyUnit::Sat` (line 10 in `balance.rs`), completely ignoring other units like `hash`.

### 2. 🌐 Network Dependency
The `total_balance()` method in `balance.rs:9-11` has no network dependency - it's purely a local database query. However, the wallet creation process during startup does have network calls that can fail.

### 3. 🔇 Silent Failure
When there are no wallets created (due to empty supported units), the command shows no useful information about what's actually in the database.

---

## 🔍 Root Cause Analysis

The architecture has these layers:
```
CLI balance command 
  → calls mint_balances() with hardcoded CurrencyUnit::Sat
    → calls MultiMintWallet.get_balances(unit) 
      → filters wallets by unit match (line 124 in multi_mint_wallet.rs)
        → calls individual Wallet.total_balance()
          → calls get_unspent_proofs() 
            → queries database with mint_url + unit filters (proofs.rs:49-61)
```

**🎯 The core issue**: Line 124 in `multi_mint_wallet.rs` only includes wallets where `unit == u`, so if no wallet exists for a unit, that unit's balance is never shown.

---

## 🚀 Implementation Plan

### Phase 1: Fix Unit Discovery (Offline Balance Query)

**📁 File**: `/home/evan/work/cdk/crates/cdk-cli/src/sub_commands/balance.rs`

Replace the current hardcoded approach with a comprehensive balance query:

```rust
pub async fn balance(multi_mint_wallet: &MultiMintWallet) -> Result<()> {
    // Get all units that have proofs in the database
    let all_balances = get_all_unit_balances(multi_mint_wallet).await?;
    
    if all_balances.is_empty() {
        println!("No proofs found in wallet database.");
        return Ok(());
    }
    
    println!("Wallet Balances:");
    for ((mint_url, unit), amount) in all_balances {
        if amount > Amount::ZERO {
            println!("  {}: {} {}", mint_url, amount, unit);
        }
    }
    
    // Also show zero balances for configured wallets
    show_configured_wallets_summary(multi_mint_wallet).await?;
    
    Ok(())
}

async fn get_all_unit_balances(
    multi_mint_wallet: &MultiMintWallet
) -> Result<BTreeMap<(MintUrl, CurrencyUnit), Amount>> {
    // Direct database query to get all proofs grouped by mint_url and unit
    // This bypasses the wallet filtering entirely
    let proof_summary = multi_mint_wallet.localstore
        .get_proof_summary_by_mint_and_unit().await?;
    
    proof_summary.into_iter().collect()
}
```

### Phase 2: Add Database Method for Direct Proof Queries

**📁 File**: `/home/evan/work/cdk/crates/cdk-common/src/database/wallet.rs`

Add new method to the Database trait:

```rust
/// Get summary of unspent proofs grouped by mint_url and unit
async fn get_proof_summary_by_mint_and_unit(
    &self
) -> Result<Vec<((MintUrl, CurrencyUnit), Amount)>, Self::Err>;
```

**📁 File**: `/home/evan/work/cdk/crates/cdk-sql-common/src/wallet/mod.rs`

Implement the method:

```rust
async fn get_proof_summary_by_mint_and_unit(
    &self
) -> Result<Vec<((MintUrl, CurrencyUnit), Amount)>, Self::Err> {
    let conn = self.pool.get().map_err(|e| Error::Database(Box::new(e)))?;
    
    let results = query(
        r#"
        SELECT 
            mint_url,
            unit,
            SUM(amount) as total_amount,
            COUNT(*) as proof_count
        FROM proof 
        WHERE state = 'unspent'
        GROUP BY mint_url, unit
        ORDER BY mint_url, unit
        "#,
    )?
    .fetch_all(&*conn)
    .await?;
    
    let mut summary = Vec::new();
    for row in results {
        let mint_url = column_as_string!(row[0], MintUrl::from_str);
        let unit = column_as_string!(row[1], CurrencyUnit::from_str);
        let amount = Amount::from(column_as_number!(row[2]) as u64);
        
        summary.push(((mint_url, unit), amount));
    }
    
    Ok(summary)
}
```

### Phase 3: Enhanced MultiMintWallet Methods

**📁 File**: `/home/evan/work/cdk/crates/cdk/src/wallet/multi_mint_wallet.rs`

Add comprehensive balance methods:

```rust
/// Get balances for all units across all mints
pub async fn get_all_balances(&self) -> Result<BTreeMap<(MintUrl, CurrencyUnit), Amount>, Error> {
    // Direct database query - no wallet filtering
    let summary = self.localstore.get_proof_summary_by_mint_and_unit().await?;
    Ok(summary.into_iter().collect())
}

/// Get all currency units that have proofs
pub async fn get_active_units(&self) -> Result<HashSet<CurrencyUnit>, Error> {
    let summary = self.localstore.get_proof_summary_by_mint_and_unit().await?;
    Ok(summary.into_iter().map(|((_, unit), _)| unit).collect())
}

/// Get balance for specific mint and unit (works even without active wallet)
pub async fn get_balance_for_mint_unit(
    &self, 
    mint_url: &MintUrl, 
    unit: &CurrencyUnit
) -> Result<Amount, Error> {
    // Direct database query
    let proofs = self.localstore.get_proofs(
        Some(mint_url.clone()),
        Some(unit.clone()),
        Some(vec![State::Unspent]),
        None
    ).await?;
    
    Ok(proofs.into_iter().map(|p| p.proof.amount).sum::<Result<Amount, _>>()?)
}
```

### Phase 4: Improved CLI Output and Diagnostics

**📁 File**: `/home/evan/work/cdk/crates/cdk-cli/src/sub_commands/balance.rs`

Enhanced output with detailed information:

```rust
async fn show_detailed_balance(multi_mint_wallet: &MultiMintWallet) -> Result<()> {
    println!("=== Wallet Balance Report ===\n");
    
    // Show actual proof balances (works offline)
    let all_balances = multi_mint_wallet.get_all_balances().await?;
    
    if all_balances.is_empty() {
        println!("No proofs found in local database.");
    } else {
        println!("Actual Balances (from local proofs):");
        let mut total_by_unit: BTreeMap<CurrencyUnit, Amount> = BTreeMap::new();
        
        for ((mint_url, unit), amount) in &all_balances {
            if *amount > Amount::ZERO {
                println!("  {}: {} {}", mint_url, amount, unit);
                *total_by_unit.entry(unit.clone()).or_default() += *amount;
            }
        }
        
        if total_by_unit.len() > 1 {
            println!("\nTotals by Unit:");
            for (unit, total) in total_by_unit {
                println!("  Total {}: {}", unit, total);
            }
        }
    }
    
    // Show configured wallet status
    println!("\n=== Configured Wallets ===");
    let wallets = multi_mint_wallet.get_wallets().await;
    if wallets.is_empty() {
        println!("No wallets currently configured.");
        
        // Show what's in the database even without wallets
        let mints = multi_mint_wallet.localstore.get_mints().await?;
        if !mints.is_empty() {
            println!("Available mints in database:");
            for (mint_url, _) in mints {
                println!("  {}", mint_url);
            }
        }
    } else {
        for wallet in wallets {
            let balance = wallet.total_balance().await.unwrap_or(Amount::ZERO);
            println!("  {} ({}): {}", wallet.mint_url, wallet.unit, balance);
        }
    }
    
    Ok(())
}
```

### Phase 5: Network Independence and Error Handling

**📁 File**: `/home/evan/work/cdk/crates/cdk-cli/src/main.rs`

Improve wallet creation to be more resilient:

```rust
// In the wallet creation loop, add error handling
for unit in units {
    tracing::info!("Creating wallet for mint {} with unit {}", mint_url, unit);
    match builder.build() {
        Ok(wallet) => {
            // Only try to refresh keysets if we can connect
            let wallet_clone = wallet.clone();
            tokio::spawn(async move {
                if let Err(err) = wallet_clone.refresh_keysets().await {
                    tracing::warn!(
                        "Could not refresh keysets for {} (offline?): {}",
                        wallet_clone.mint_url,
                        err
                    );
                    // Don't fail - wallet can still work for local operations
                }
            });
            
            wallets.push(wallet);
        }
        Err(err) => {
            tracing::warn!("Failed to create wallet for {} with unit {}: {}", mint_url, unit, err);
            // Continue with other wallets
        }
    }
}
```

---

## 📈 Implementation Priority

1. **🔥 High Priority**: Phase 2 & 3 - Add database methods and MultiMintWallet improvements
2. **🔥 High Priority**: Phase 1 - Fix the CLI balance command to show all units  
3. **⚡ Medium Priority**: Phase 4 - Enhanced output and diagnostics
4. **💡 Low Priority**: Phase 5 - Network resilience improvements

---

## 🎯 Expected Results

After implementation:
- ✅ `cdk-cli balance` will show all units (sat, hash, etc.) without network connection
- ✅ Shows actual proof counts and amounts from local database
- ✅ Works even when wallet creation fails due to network issues
- ✅ Provides clear diagnostic information about what's in the wallet
- ✅ Maintains backward compatibility

**💡 The key insight**: Balance should be a **pure database operation** that doesn't depend on having configured wallets or network connectivity.

---

## 📊 Summary

The CDK balance command has fundamental architectural issues that need to be addressed:

### 🐛 Issues
1. **Unit filtering bug**: Hardcoded to only show sat units
2. **Network dependency**: Shouldn't need mint connection for balance queries
3. **Silent failures**: Doesn't show actual database contents when wallet creation fails

### 🔧 Core Fix Requirements
- Adding direct database queries that bypass wallet filtering
- Implementing `get_proof_summary_by_mint_and_unit()` method
- Updating the CLI to query all units instead of just sat
- Making the balance command work purely offline

This would transform the balance command from a **network-dependent wallet operation** into a **fast, reliable local database query** that shows all proof balances regardless of wallet configuration status.