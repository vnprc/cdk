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
        .mint_batch(
            quote_ids.clone(),
            SplitTarget::default(),
            None,
            PaymentMethod::Bolt11,
        )
        .await;

    assert!(matches!(
        result,
        Err(cdk::error::Error::BatchCurrencyUnitMismatch)
    ));

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
        .mint_batch(
            quote_ids.clone(),
            SplitTarget::default(),
            None,
            PaymentMethod::Bolt11,
        )
        .await;

    assert!(matches!(
        result,
        Err(cdk::error::Error::BatchPaymentMethodMismatch)
    ));

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
        .mint_batch(
            quote_ids,
            SplitTarget::default(),
            None,
            PaymentMethod::Bolt11,
        )
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
        .mint_batch(
            quote_ids,
            SplitTarget::default(),
            None,
            PaymentMethod::Bolt11,
        )
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
        .mint_batch(vec![], SplitTarget::default(), None, PaymentMethod::Bolt11)
        .await;

    assert!(matches!(result, Err(cdk::error::Error::BatchEmpty)));

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
            PaymentMethod::Bolt11,
        )
        .await;

    assert!(result.is_err());
    match result {
        Err(cdk::error::Error::UnknownQuote) => (),
        _ => panic!("Expected UnknownQuote error"),
    }

    Ok(())
}

#[tokio::test]
async fn test_wallet_batch_mint_rejects_empty_quotes() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit.clone(), Arc::new(localstore), seed, None)?;

    let result = wallet
        .mint_batch(vec![], SplitTarget::default(), None, PaymentMethod::Bolt11)
        .await;

    assert!(matches!(result, Err(cdk::error::Error::BatchEmpty)));
    Ok(())
}

#[tokio::test]
async fn test_wallet_batch_mint_rejects_oversized_batch() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit.clone(), Arc::new(localstore), seed, None)?;

    // Try to create a batch with 101 quotes
    let quote_ids: Vec<String> = (0..101).map(|i| format!("quote_{}", i)).collect();
    let result = wallet
        .mint_batch(
            quote_ids,
            SplitTarget::default(),
            None,
            PaymentMethod::Bolt11,
        )
        .await;

    assert!(matches!(result, Err(cdk::error::Error::BatchSizeExceeded)));
    Ok(())
}

#[tokio::test]
async fn test_wallet_batch_mint_rejects_over_limit() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), seed, None)?;

    // Create 101 quote IDs
    let quote_ids: Vec<String> = (0..101).map(|i| format!("quote_{}", i)).collect();

    // Try to mint batch with over 100 quotes
    let result = wallet
        .mint_batch(
            quote_ids,
            SplitTarget::default(),
            None,
            PaymentMethod::Bolt11,
        )
        .await;

    assert!(matches!(result, Err(cdk::error::Error::BatchSizeExceeded)));

    Ok(())
}

#[tokio::test]
async fn test_wallet_batch_mint_requires_all_paid_state() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), seed, None)?;

    let quote1 = wallet.mint_quote(Amount::from(100), None).await?;
    let quote2 = wallet.mint_quote(Amount::from(200), None).await?;

    // Mark quote1 as paid
    let mut quote_info = wallet.localstore.get_mint_quote(&quote1.id).await?.unwrap();
    quote_info.state = MintQuoteState::Paid;
    wallet.localstore.add_mint_quote(quote_info).await?;

    // Try to mint batch with mixed states
    let quote_ids = vec![quote1.id.clone(), quote2.id.clone()];
    let result = wallet
        .mint_batch(
            quote_ids,
            SplitTarget::default(),
            None,
            PaymentMethod::Bolt11,
        )
        .await;

    // Should fail because quote2 is not paid
    assert!(matches!(result, Err(cdk::error::Error::UnpaidQuote)));

    Ok(())
}

#[tokio::test]
async fn test_batch_mint_payment_method_validation() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit.clone(), Arc::new(localstore), seed, None)?;

    // Create quote with Bolt11 (wallet default)
    let quote1 = wallet.mint_quote(Amount::from(100), None).await?;

    // Create quote manually with Bolt12 payment method
    let quote2 = cdk_common::wallet::MintQuote::new(
        "quote_bolt12".to_string(),
        "https://fake.thesimplekid.dev".parse()?,
        PaymentMethod::Bolt12,
        Some(Amount::from(50)),
        unit.clone(),
        "lnbc500n...".to_string(),
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

    // batch mint with mixed payment methods
    let quote_ids = vec![quote1.id.clone(), quote2.id.clone()];
    let result = wallet
        .mint_batch(
            quote_ids,
            SplitTarget::default(),
            None,
            PaymentMethod::Bolt11,
        )
        .await;

    // Should fail with BatchPaymentMethodMismatch
    assert!(matches!(
        result,
        Err(cdk::error::Error::BatchPaymentMethodMismatch)
    ));

    Ok(())
}

#[tokio::test]
async fn test_batch_mint_enforces_url_payment_method() -> anyhow::Result<()> {
    let seed = rand::random::<[u8; 64]>();
    let mint_url = "https://fake.thesimplekid.dev";
    let unit = cdk::nuts::CurrencyUnit::Sat;

    let localstore = memory::empty().await?;
    let wallet = Wallet::new(mint_url, unit.clone(), Arc::new(localstore), seed, None)?;

    // Create two Bolt12 quotes manually (not compatible with Bolt11 endpoint)
    let quote1 = cdk_common::wallet::MintQuote::new(
        "quote_bolt12_1".to_string(),
        "https://fake.thesimplekid.dev".parse()?,
        PaymentMethod::Bolt12,
        Some(Amount::from(100)),
        unit.clone(),
        "lnbc1000n...".to_string(),
        1000,
        None,
    );
    wallet.localstore.add_mint_quote(quote1.clone()).await?;

    let quote2 = cdk_common::wallet::MintQuote::new(
        "quote_bolt12_2".to_string(),
        "https://fake.thesimplekid.dev".parse()?,
        PaymentMethod::Bolt12,
        Some(Amount::from(200)),
        unit.clone(),
        "lnbc2000n...".to_string(),
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

    // Quotes are Bolt12 but endpoint is Bolt11
    let quote_ids = vec![quote1.id.clone(), quote2.id.clone()];
    let result = wallet
        .mint_batch(
            quote_ids,
            SplitTarget::default(),
            None,
            PaymentMethod::Bolt11,
        )
        .await;

    // Should fail with BatchPaymentMethodEndpointMismatch error
    assert!(matches!(
        result,
        Err(cdk::error::Error::BatchPaymentMethodEndpointMismatch)
    ));

    Ok(())
}
