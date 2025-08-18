use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bip39::Mnemonic;
use bitcoin::hashes::Hash;
use cdk::mint::{MintBuilder, MintMeltLimits};
use cdk::nuts::SecretKey;
use cdk::nuts::{CurrencyUnit, MintQuoteBolt11Request, PaymentMethod, PublicKey};
use cdk::types::{FeeReserve, QuoteTTL};
use cdk::{util::unix_time, Amount, Mint};
use cdk_fake_wallet::FakeWallet;
use cdk_integration_tests::init_pure_tests::{
    create_and_start_test_mint, create_test_wallet_for_mint, setup_tracing,
};
use tokio::sync::Notify;

async fn create_mining_share_test_mint() -> anyhow::Result<Mint> {
    let mut mint_builder = MintBuilder::new()
        .with_localstore(Arc::new(cdk_sqlite::mint::memory::empty().await?))
        .with_keystore(Arc::new(cdk_sqlite::mint::memory::empty().await?));

    let fee_reserve = FeeReserve {
        min_fee_reserve: 1.into(),
        percent_fee_reserve: 1.0,
    };

    // Add Bolt11 backend
    let ln_fake_backend = FakeWallet::new(
        fee_reserve.clone(),
        HashMap::default(),
        HashSet::default(),
        0,
    );

    mint_builder = mint_builder
        .add_ln_backend(
            CurrencyUnit::Sat,
            PaymentMethod::Bolt11,
            MintMeltLimits::new(1, 10_000),
            Arc::new(ln_fake_backend),
        )
        .await?;

    // Add MiningShare backend (using same fake wallet - mining shares don't need real backend)
    let mining_fake_backend = FakeWallet::new(
        fee_reserve.clone(),
        HashMap::default(),
        HashSet::default(),
        0,
    );

    mint_builder = mint_builder
        .add_ln_backend(
            CurrencyUnit::Sat,
            PaymentMethod::MiningShare,
            MintMeltLimits::new(1, 10_000),
            Arc::new(mining_fake_backend),
        )
        .await?;

    let mnemonic = Mnemonic::generate(12)?;

    mint_builder = mint_builder
        .with_name("mining share test mint".to_string())
        .with_description("mining share test mint".to_string())
        .with_urls(vec!["https://aaa".to_string()])
        .with_seed(mnemonic.to_seed_normalized("").to_vec());

    let localstore = mint_builder
        .localstore
        .as_ref()
        .map(|x| x.clone())
        .expect("localstore");

    let mut tx = localstore.begin_transaction().await?;
    tx.set_mint_info(mint_builder.mint_info.clone()).await?;

    let quote_ttl = QuoteTTL::new(10000, 10000);
    tx.set_quote_ttl(quote_ttl).await?;
    tx.commit().await?;

    let mint = mint_builder.build().await?;

    let mint_clone = mint.clone();
    let shutdown = Arc::new(Notify::new());
    tokio::spawn({
        let shutdown = Arc::clone(&shutdown);
        async move { mint_clone.wait_for_paid_invoices(shutdown).await }
    });

    Ok(mint)
}

#[tokio::test]
async fn test_hashpool_quote_lookup() -> anyhow::Result<()> {
    let mint = create_and_start_test_mint().await?;

    // Generate a test key pair for locking the quote
    let secret_key = SecretKey::generate();
    let pubkey = secret_key.public_key();

    // Create a mint quote with the locking pubkey
    let quote_request = MintQuoteBolt11Request {
        amount: Amount::from(100),
        unit: CurrencyUnit::Sat,
        description: Some("Test quote for quote lookup".to_string()),
        pubkey: Some(pubkey),
    };

    // Request the mint quote directly from the mint
    let quote_response = mint.get_mint_quote(quote_request.into()).await?;
    let quote_id = match quote_response {
        cdk::mint::MintQuoteResponse::Bolt11(response) => response.quote.to_string(),
        cdk::mint::MintQuoteResponse::Bolt12(response) => response.quote.to_string(),
        cdk::mint::MintQuoteResponse::MiningShare(response) => response.quote.to_string(),
    };

    // Now test the lookup functionality directly
    let lookup_response = mint
        .lookup_mint_quotes_by_pubkeys(&[pubkey], cdk::hashpool::MintQuoteStateFilter::All)
        .await?;

    // Verify the response
    assert_eq!(lookup_response.len(), 1);

    let found_quote = &lookup_response[0];
    assert_eq!(found_quote.pubkey, pubkey);
    assert_eq!(found_quote.quote, quote_id);
    assert_eq!(found_quote.method, PaymentMethod::Bolt11);

    println!("✅ Hashpool quote lookup test passed!");
    Ok(())
}

#[tokio::test]
async fn test_hashpool_quote_lookup_multiple_keys() -> anyhow::Result<()> {
    let mint = create_and_start_test_mint().await?;

    // Generate multiple test key pairs
    let secret_key1 = SecretKey::generate();
    let pubkey1 = secret_key1.public_key();

    let secret_key2 = SecretKey::generate();
    let pubkey2 = secret_key2.public_key();

    // Create quotes with different pubkeys
    let _quote1 = mint
        .get_mint_quote(
            MintQuoteBolt11Request {
                amount: Amount::from(50),
                unit: CurrencyUnit::Sat,
                description: Some("Quote 1".to_string()),
                pubkey: Some(pubkey1),
            }
            .into(),
        )
        .await?;

    let _quote2 = mint
        .get_mint_quote(
            MintQuoteBolt11Request {
                amount: Amount::from(75),
                unit: CurrencyUnit::Sat,
                description: Some("Quote 2".to_string()),
                pubkey: Some(pubkey2),
            }
            .into(),
        )
        .await?;

    // Lookup both quotes
    let lookup_response = mint
        .lookup_mint_quotes_by_pubkeys(
            &[pubkey1, pubkey2],
            cdk::hashpool::MintQuoteStateFilter::All,
        )
        .await?;

    // Should find both quotes
    assert_eq!(lookup_response.len(), 2);

    let mut found_pubkeys = HashSet::new();
    for quote in &lookup_response {
        found_pubkeys.insert(quote.pubkey);
    }

    assert!(found_pubkeys.contains(&pubkey1));
    assert!(found_pubkeys.contains(&pubkey2));

    println!("✅ Hashpool multiple key lookup test passed!");
    Ok(())
}

#[tokio::test]
async fn test_hashpool_quote_lookup_empty_result() -> anyhow::Result<()> {
    let mint = create_and_start_test_mint().await?;

    // Generate a key that has no associated quotes
    let secret_key = SecretKey::generate();
    let pubkey = secret_key.public_key();

    let lookup_response = mint
        .lookup_mint_quotes_by_pubkeys(&[pubkey], cdk::hashpool::MintQuoteStateFilter::All)
        .await?;

    // Should return empty array
    assert_eq!(lookup_response.len(), 0);

    println!("✅ Hashpool empty lookup test passed!");
    Ok(())
}

#[tokio::test]
async fn test_wallet_hashpool_quote_lookup() -> anyhow::Result<()> {
    let mint = create_and_start_test_mint().await?;
    let wallet = create_test_wallet_for_mint(mint.clone()).await?;

    // Initialize the wallet by fetching mint info
    wallet.get_mint_info().await?;

    // Generate a test key pair for locking the quote
    let secret_key = SecretKey::generate();
    let pubkey = secret_key.public_key();

    // Create a mint quote with the locking pubkey via the wallet
    let quote_request = MintQuoteBolt11Request {
        amount: Amount::from(100),
        unit: CurrencyUnit::Sat,
        description: Some("Test quote for wallet lookup".to_string()),
        pubkey: Some(pubkey),
    };

    // Request the mint quote via the wallet with locking pubkey
    let _quote_response = wallet
        .mint_quote_with_pubkey(
            Amount::from(100),
            Some("Test quote for wallet lookup".to_string()),
            Some(pubkey),
        )
        .await?;

    // Now test the wallet lookup functionality
    let lookup_response = wallet
        .lookup_mint_quotes_by_pubkeys(&[pubkey], cdk::hashpool::MintQuoteStateFilter::All)
        .await?;

    // Verify the response
    assert_eq!(lookup_response.len(), 1);

    let found_quote = &lookup_response[0];
    assert_eq!(found_quote.pubkey, pubkey);
    assert_eq!(found_quote.method, PaymentMethod::Bolt11);

    println!("✅ Wallet hashpool quote lookup test passed!");
    Ok(())
}

#[tokio::test]
async fn test_wallet_hashpool_multiple_quote_lookup() -> anyhow::Result<()> {
    let mint = create_and_start_test_mint().await?;
    let wallet = create_test_wallet_for_mint(mint.clone()).await?;

    // Initialize the wallet by fetching mint info
    wallet.get_mint_info().await?;

    // Generate multiple test key pairs
    let secret_key1 = SecretKey::generate();
    let pubkey1 = secret_key1.public_key();

    let secret_key2 = SecretKey::generate();
    let pubkey2 = secret_key2.public_key();

    // Create quotes with different pubkeys via the wallet
    let _quote1 = wallet
        .mint_quote_with_pubkey(
            Amount::from(50),
            Some("Wallet Quote 1".to_string()),
            Some(pubkey1),
        )
        .await?;

    let _quote2 = wallet
        .mint_quote_with_pubkey(
            Amount::from(75),
            Some("Wallet Quote 2".to_string()),
            Some(pubkey2),
        )
        .await?;

    // Lookup both quotes via the wallet
    let lookup_response = wallet
        .lookup_mint_quotes_by_pubkeys(
            &[pubkey1, pubkey2],
            cdk::hashpool::MintQuoteStateFilter::All,
        )
        .await?;

    // Should find both quotes
    assert_eq!(lookup_response.len(), 2);

    let mut found_pubkeys = HashSet::new();
    for quote in &lookup_response {
        found_pubkeys.insert(quote.pubkey);
    }

    assert!(found_pubkeys.contains(&pubkey1));
    assert!(found_pubkeys.contains(&pubkey2));

    println!("✅ Wallet multiple hashpool quote lookup test passed!");
    Ok(())
}

#[tokio::test]
async fn test_mint_tokens_for_pubkey() -> anyhow::Result<()> {
    setup_tracing();

    // Create a custom mint with mining share support
    let mint = create_mining_share_test_mint().await?;
    let wallet = create_test_wallet_for_mint(mint.clone()).await?;

    // Initialize wallet and keysets
    wallet.get_mint_info().await?;

    // Load mint keysets to ensure wallet has keyset information
    let _keysets = wallet.get_mint_keysets().await?;

    // Generate a keypair for locking and signing
    use cdk::nuts::SecretKey;
    let secret_key = SecretKey::generate();
    let pubkey = secret_key.public_key();

    // Create mining share quote on the mint with locking pubkey
    let quote_request = cashu::MintQuoteMiningShareRequest {
        amount: Amount::from(1),
        unit: CurrencyUnit::Sat,
        header_hash: bitcoin::hashes::sha256::Hash::hash(&[1; 32]),
        description: None,
        pubkey: Some(pubkey),
        keyset_id: mint.keysets().keysets[0].id,
    };

    // Create the quote on the mint
    let _quote = mint.create_mint_mining_share_quote(quote_request).await?;

    // Now try to mint tokens for this pubkey using the wallet
    let proofs = wallet
        .mint_tokens_for_pubkey(pubkey, Some(secret_key))
        .await?;

    // Should have minted some proofs
    assert!(!proofs.is_empty(), "Should have minted at least one proof");
    assert_eq!(
        proofs.len(),
        1,
        "Should have minted exactly one proof for amount 1"
    );
    assert_eq!(
        proofs[0].amount,
        Amount::from(1),
        "Proof should be for amount 1"
    );

    println!(
        "✅ Mint tokens for pubkey test passed! Minted {} proofs",
        proofs.len()
    );
    Ok(())
}
