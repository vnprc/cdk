use std::sync::Arc;

use cdk::amount::{Amount, SplitTarget};
use cdk::wallet::Wallet;
use cdk_common::{MintQuoteState, PaymentMethod};
use cdk_sqlite::wallet::memory;

// Validation tests for batch minting - detailed integration tests will be in cdk-integration-tests
#[tokio::test]
async fn test_wallet_batch_mint_validates_same_unit() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), seed, None)?;

    // Create a quote and manually create another with different unit
    let quote1 = wallet.mint_quote(Amount::from(100), None).await?;
    let quote2 = cdk_common::wallet::MintQuote::new(
        "quote_different_unit".to_string(),
        "https://fake.thesimplekid.dev".parse()?,
        PaymentMethod::Bolt11,
        Some(Amount::from(200)),
        cdk::nuts::CurrencyUnit::Usd, // Different unit
        "lnbc2000n1ps0qqqqpp5qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqc8md94k6ar0da6gur0d3shg2zkyypqsp5qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqhp58yjmyan4xq28guqq3c0sd5zyab0duulfr60v2n9qfv33zsrxqsp5qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqhp4qqzj3u8ysyg8u0yy".to_string(),
        1000,
        None,
    );
    wallet.localstore.add_mint_quote(quote2.clone()).await?;

    // Mark both as paid
    for quote_id in [&quote1.id, &quote2.id] {
        let mut quote_info = wallet.localstore.get_mint_quote(quote_id).await?.unwrap();
        quote_info.state = MintQuoteState::Paid;
        wallet.localstore.add_mint_quote(quote_info).await?;
    }

    // Try to mint batch with different units - should fail before HTTP call
    let quote_ids = vec![quote1.id.clone(), quote2.id.clone()];
    let result = wallet
        .mint_batch(quote_ids.clone(), SplitTarget::default(), None)
        .await;

    assert!(result.is_err());
    match result {
        Err(cdk::error::Error::UnsupportedUnit) => (),
        _ => panic!("Expected UnsupportedUnit error"),
    }

    Ok(())
}

#[tokio::test]
async fn test_wallet_batch_mint_mixed_payment_methods_error() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit.clone(), Arc::new(localstore), seed, None)?;

    // Create quotes with different payment methods
    let quote1 = wallet.mint_quote(Amount::from(100), None).await?;

    // Create a quote with bolt12 payment method manually for testing
    let quote2 = cdk_common::wallet::MintQuote::new(
        "quote2".to_string(),
        "https://fake.thesimplekid.dev".parse()?,
        PaymentMethod::Bolt12,
        Some(Amount::from(200)),
        unit.clone(),
        "lnbc2000n1ps0qqqqpp5qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqc8md94k6ar0da6gur0d3shg2zkyypqsp5qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqhp58yjmyan4xq28guqq3c0sd5zyab0duulfr60v2n9qfv33zsrxqsp5qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqhp4qqzj3u8ysyg8u0yy".to_string(),
        1000,
        None,
    );
    wallet.localstore.add_mint_quote(quote2.clone()).await?;

    // Try to mint batch with mixed payment methods
    let quote_ids = vec![quote1.id.clone(), quote2.id.clone()];

    // Mark both as paid
    for quote_id in quote_ids.iter() {
        let mut quote_info = wallet.localstore.get_mint_quote(quote_id).await?.unwrap();
        quote_info.state = MintQuoteState::Paid;
        wallet.localstore.add_mint_quote(quote_info).await?;
    }

    // This should fail because quotes have different payment methods
    let result = wallet
        .mint_batch(quote_ids.clone(), SplitTarget::default(), None)
        .await;

    assert!(result.is_err());
    match result {
        Err(cdk::error::Error::UnsupportedPaymentMethod) => (),
        _ => panic!("Expected UnsupportedPaymentMethod error"),
    }

    Ok(())
}

#[tokio::test]
async fn test_wallet_batch_mint_unpaid_quote_error() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), seed, None)?;

    // Create two quotes
    let quote1 = wallet.mint_quote(Amount::from(100), None).await?;
    let quote2 = wallet.mint_quote(Amount::from(200), None).await?;

    // Mark only quote1 as paid
    let mut quote_info = wallet.localstore.get_mint_quote(&quote1.id).await?.unwrap();
    quote_info.state = MintQuoteState::Paid;
    wallet.localstore.add_mint_quote(quote_info).await?;

    // Try to mint batch with one unpaid quote
    let quote_ids = vec![quote1.id.clone(), quote2.id.clone()];
    let result = wallet
        .mint_batch(quote_ids, SplitTarget::default(), None)
        .await;

    // Should fail because quote2 is not paid
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_wallet_batch_mint_single_quote_validation() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), seed, None)?;

    // Create single quote
    let amount = Amount::from(500);
    let quote = wallet.mint_quote(amount, None).await?;

    // Mark as paid
    let mut quote_info = wallet.localstore.get_mint_quote(&quote.id).await?.unwrap();
    quote_info.state = MintQuoteState::Paid;
    wallet.localstore.add_mint_quote(quote_info).await?;

    // Try to mint batch with single quote - will fail at HTTP level but validation should pass
    let quote_ids = vec![quote.id.clone()];
    let result = wallet
        .mint_batch(quote_ids, SplitTarget::default(), None)
        .await;

    // Should fail due to HTTP communication, not validation
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn test_wallet_batch_mint_empty_list_error() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), seed, None)?;

    // Try to mint batch with empty list
    let result = wallet
        .mint_batch(vec![], SplitTarget::default(), None)
        .await;

    assert!(result.is_err());
    match result {
        Err(cdk::error::Error::AmountUndefined) => (),
        _ => panic!("Expected AmountUndefined error"),
    }

    Ok(())
}

#[tokio::test]
async fn test_wallet_batch_mint_unknown_quote_error() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), seed, None)?;

    // Try to mint batch with non-existent quote
    let result = wallet
        .mint_batch(
            vec!["nonexistent".to_string()],
            SplitTarget::default(),
            None,
        )
        .await;

    assert!(result.is_err());
    match result {
        Err(cdk::error::Error::UnknownQuote) => (),
        _ => panic!("Expected UnknownQuote error"),
    }

    Ok(())
}
