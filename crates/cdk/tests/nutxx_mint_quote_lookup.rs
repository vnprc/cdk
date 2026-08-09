#![allow(clippy::unwrap_used)]

//! NUT-XX: mint quote lookup by public key
//!
//! <https://github.com/cashubtc/nuts/blob/get-quotes-by-pubkeys/xx.md>

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bip39::Mnemonic;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use bitcoin::hashes::Hash;
use cdk::amount::SplitTarget;
use cdk::mint::{Mint, MintBuilder, MintInput, MintMeltLimits, QuoteId};
use cdk::nuts::nut00::KnownMethod;
use cdk::nuts::nutxx::{mint_quote_lookup_msg_to_sign, MAX_LOOKUP_PUBKEYS};
use cdk::nuts::{
    CurrencyUnit, MintQuoteBolt11Request, MintQuoteState, MintRequest, PaymentMethod,
    PreMintSecrets, PublicKey, SecretKey,
};
use cdk::types::{FeeReserve, QuoteTTL};
use cdk::{Amount, Error, MintQuoteRequest, MintQuoteResponse};
use cdk_fake_wallet::FakeWallet;

async fn test_mint() -> Mint {
    let db = Arc::new(cdk_sqlite::mint::memory::empty().await.unwrap());
    let mut builder = MintBuilder::new(db.clone());

    let backend = FakeWallet::new(
        FeeReserve {
            min_fee_reserve: 1.into(),
            percent_fee_reserve: 1.0,
        },
        HashMap::default(),
        HashSet::default(),
        2,
        CurrencyUnit::Sat,
    );

    builder
        .add_payment_processor(
            CurrencyUnit::Sat,
            PaymentMethod::Known(KnownMethod::Bolt11),
            MintMeltLimits::new(1, 10_000),
            Arc::new(backend),
        )
        .await
        .unwrap();

    let mnemonic = Mnemonic::generate(12).unwrap();
    builder = builder
        .with_name("nutxx test mint".to_string())
        .with_description("nutxx test mint".to_string())
        .with_urls(vec!["https://test-mint".to_string()]);

    let mint = builder
        .build_with_seed(db.clone(), &mnemonic.to_seed_normalized(""))
        .await
        .unwrap();
    mint.set_quote_ttl(QuoteTTL::new(10_000, 10_000))
        .await
        .unwrap();
    mint
}

/// Create a NUT-20 locked bolt11 mint quote owned by `pubkey`, for a fixed amount of 100 sat.
async fn locked_quote(mint: &Mint, pubkey: PublicKey) -> MintQuoteResponse<QuoteId> {
    mint.get_mint_quote(MintQuoteRequest::Bolt11(MintQuoteBolt11Request {
        amount: Amount::new(100, CurrencyUnit::Sat).into(),
        unit: CurrencyUnit::Sat,
        description: None,
        pubkey: Some(pubkey),
    }))
    .await
    .unwrap()
}

/// Sign the lookup message the way a spec-conformant wallet does.
async fn sign_lookup(
    mint: &Mint,
    secret_key: &SecretKey,
) -> bitcoin::secp256k1::schnorr::Signature {
    let mint_pubkey = mint.mint_info().await.unwrap().pubkey.unwrap();
    let msg = mint_quote_lookup_msg_to_sign(&mint_pubkey, &secret_key.public_key());
    secret_key.sign(&msg).unwrap()
}

/// Wait for the fake Lightning backend's scheduled payment to land and the quote to be marked
/// paid, polling `check_mint_quotes` with a short sleep. `test_mint()`'s `FakeWallet` schedules
/// its payment on a real (if short) delay rather than settling synchronously, so this can't be a
/// single check. Mirrors the poll loop in `cdk::test_helpers::mint::mint_test_proofs`, which
/// isn't reusable here since it's `#[cfg(test)]`-only and not part of the compiled `cdk` rlib
/// this integration test links against.
async fn wait_until_paid(mint: &Mint, quote_id: &QuoteId) {
    loop {
        let quotes = mint
            .check_mint_quotes(std::slice::from_ref(quote_id))
            .await
            .unwrap();
        if quotes[0].state() == Some(MintQuoteState::Paid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Mint the full 100 sat against a NUT-20 locked quote, driving the real blind-signing path (as
/// `cdk::test_helpers::mint::mint_test_proofs` does for its own tests) so `amount_issued`
/// actually catches up to `amount_paid` server-side - this is what "fully issued" means for the
/// mintable filter. The resulting proofs aren't needed, only the server-side accounting effect.
async fn mint_full_amount(mint: &Mint, quote_id: &QuoteId, owner: &SecretKey) {
    let keyset_id = *mint
        .get_active_keysets()
        .get(&CurrencyUnit::Sat)
        .expect("mint has an active sat keyset");

    let keys = mint
        .keyset_pubkeys(&keyset_id)
        .unwrap()
        .keysets
        .first()
        .unwrap()
        .keys
        .clone();

    let fee_and_amounts: cdk::amount::FeeAndAmounts =
        (0, keys.iter().map(|a| a.0.to_u64()).collect::<Vec<_>>()).into();

    let premint_secrets = PreMintSecrets::random(
        keyset_id,
        Amount::from(100),
        &SplitTarget::None,
        &fee_and_amounts,
    )
    .unwrap();

    let mut request = MintRequest {
        quote: quote_id.clone(),
        outputs: premint_secrets.blinded_messages(),
        signature: None,
    };
    request.sign(owner).unwrap();

    mint.process_mint_request(MintInput::Single(request))
        .await
        .unwrap();
}

/// A mint that serves the endpoint must say so in its NUT-06 info response, otherwise wallets
/// have no way to discover it.
#[tokio::test]
async fn support_is_advertised_in_mint_info() {
    let mint = test_mint().await;
    let mint_info = mint.mint_info().await.unwrap();

    assert!(mint_info.nuts.nutxx.supported);

    let json = serde_json::to_value(&mint_info).unwrap();
    assert_eq!(json["nuts"]["XX"]["supported"], true);
}

/// A valid signature returns the quotes locked to that pubkey.
#[tokio::test]
async fn signed_lookup_returns_own_quotes() {
    let mint = test_mint().await;
    let owner = SecretKey::generate();
    let pubkey = owner.public_key();
    locked_quote(&mint, pubkey).await;

    let signature = sign_lookup(&mint, &owner).await;
    let quotes = mint
        .get_mint_quote_by_pubkey(vec![pubkey], vec![signature], false)
        .await
        .unwrap();

    assert_eq!(quotes.len(), 1);
    assert_eq!(
        quotes[0].method(),
        PaymentMethod::Known(KnownMethod::Bolt11)
    );
}

/// The signature covers SHA256(preimage), matching the NUT. `PublicKey::verify` hashes its
/// argument, so passing a digest instead of the preimage would verify a double hash and reject
/// conformant wallets.
#[tokio::test]
async fn signature_is_over_a_single_hash_of_the_preimage() {
    let mint = test_mint().await;
    let owner = SecretKey::generate();
    let pubkey = owner.public_key();
    locked_quote(&mint, pubkey).await;

    let mint_pubkey = mint.mint_info().await.unwrap().pubkey.unwrap();
    let msg = mint_quote_lookup_msg_to_sign(&mint_pubkey, &pubkey);

    // Signing the preimage is accepted...
    let signature = owner.sign(&msg).unwrap();
    assert!(mint
        .get_mint_quote_by_pubkey(vec![pubkey], vec![signature], false)
        .await
        .is_ok());

    // ...and signing the digest of the preimage is not.
    let digest = Sha256Hash::hash(&msg).to_byte_array();
    let double_hashed = owner.sign(&digest).unwrap();
    assert!(matches!(
        mint.get_mint_quote_by_pubkey(vec![pubkey], vec![double_hashed], false)
            .await,
        Err(Error::SignatureMissingOrInvalid)
    ));
}

/// "The mint MUST reject the request unless every signature is valid" — an empty or short
/// signature array must not silently skip verification.
#[tokio::test]
async fn missing_signatures_are_rejected() {
    let mint = test_mint().await;
    let victim = SecretKey::generate();
    let victim_pubkey = victim.public_key();
    locked_quote(&mint, victim_pubkey).await;

    // No signature at all.
    assert!(matches!(
        mint.get_mint_quote_by_pubkey(vec![victim_pubkey], vec![], false)
            .await,
        Err(Error::SignatureMissingOrInvalid)
    ));

    // Fewer signatures than pubkeys: one valid signature must not authorise a second pubkey.
    let attacker = SecretKey::generate();
    let attacker_signature = sign_lookup(&mint, &attacker).await;
    assert!(matches!(
        mint.get_mint_quote_by_pubkey(
            vec![attacker.public_key(), victim_pubkey],
            vec![attacker_signature],
            false
        )
        .await,
        Err(Error::SignatureMissingOrInvalid)
    ));
}

/// A signature from one key must not unlock a different key's quotes.
#[tokio::test]
async fn signature_from_another_key_is_rejected() {
    let mint = test_mint().await;
    let victim_pubkey = SecretKey::generate().public_key();
    locked_quote(&mint, victim_pubkey).await;

    let attacker = SecretKey::generate();
    let attacker_signature = sign_lookup(&mint, &attacker).await;

    assert!(matches!(
        mint.get_mint_quote_by_pubkey(vec![victim_pubkey], vec![attacker_signature], false)
            .await,
        Err(Error::SignatureMissingOrInvalid)
    ));
}

/// A signature bound to a different mint must not be replayable here.
#[tokio::test]
async fn signature_for_another_mint_is_rejected() {
    let mint = test_mint().await;
    let owner = SecretKey::generate();
    let pubkey = owner.public_key();
    locked_quote(&mint, pubkey).await;

    let other_mint_pubkey = SecretKey::generate().public_key();
    let msg = mint_quote_lookup_msg_to_sign(&other_mint_pubkey, &pubkey);
    let signature = owner.sign(&msg).unwrap();

    assert!(matches!(
        mint.get_mint_quote_by_pubkey(vec![pubkey], vec![signature], false)
            .await,
        Err(Error::SignatureMissingOrInvalid)
    ));
}

/// An anonymous caller cannot ask the mint for unbounded signature verification.
#[tokio::test]
async fn oversized_request_is_rejected() {
    let mint = test_mint().await;

    let pubkeys: Vec<PublicKey> = (0..MAX_LOOKUP_PUBKEYS + 1)
        .map(|_| SecretKey::generate().public_key())
        .collect();

    assert!(matches!(
        mint.get_mint_quote_by_pubkey(pubkeys, vec![], false).await,
        Err(Error::BatchSizeExceeded { .. })
    ));
}

/// `only_mintable` narrows the lookup to quotes that are actually mintable right now
/// (`amount_paid > amount_issued`): unpaid and fully-issued quotes are both excluded when set,
/// and nothing is excluded when it's left off.
#[tokio::test]
async fn only_mintable_filter_narrows_to_paid_unissued_quotes() {
    let mint = test_mint().await;
    let owner = SecretKey::generate();
    let pubkey = owner.public_key();

    // Unpaid: never touched after creation.
    let unpaid = locked_quote(&mint, pubkey).await;

    // Paid, not yet issued: wait for the fake backend's payment to land and be recorded.
    let paid_unissued = locked_quote(&mint, pubkey).await;
    wait_until_paid(&mint, paid_unissued.quote()).await;

    // Fully issued: paid, then minted in full so amount_issued catches up to amount_paid.
    let fully_issued = locked_quote(&mint, pubkey).await;
    wait_until_paid(&mint, fully_issued.quote()).await;
    mint_full_amount(&mint, fully_issued.quote(), &owner).await;

    let signature = sign_lookup(&mint, &owner).await;
    let mintable_only = mint
        .get_mint_quote_by_pubkey(vec![pubkey], vec![signature], true)
        .await
        .unwrap();
    assert_eq!(
        mintable_only
            .iter()
            .map(|q| q.quote().clone())
            .collect::<Vec<_>>(),
        vec![paid_unissued.quote().clone()],
        "only_mintable=true must return exactly the paid-but-unissued quote"
    );

    // A fresh signature: the one above was consumed by the previous call's request.
    let signature = sign_lookup(&mint, &owner).await;
    let everything = mint
        .get_mint_quote_by_pubkey(vec![pubkey], vec![signature], false)
        .await
        .unwrap();
    let mut everything_ids: Vec<QuoteId> = everything.iter().map(|q| q.quote().clone()).collect();
    everything_ids.sort();
    let mut expected_ids = vec![
        unpaid.quote().clone(),
        paid_unissued.quote().clone(),
        fully_issued.quote().clone(),
    ];
    expected_ids.sort();
    assert_eq!(
        everything_ids, expected_ids,
        "only_mintable=false must return all three quotes regardless of accounting state"
    );
}
