//! Mining Share Wallet Issue Functions
//!
//! This module implements wallet-side functions for processing
//! mining share mint quotes.

use cdk_common::nuts::{BlindSignature, PreMintSecrets, Proof};
use tracing::instrument;

use cdk_common::amount::SplitTarget;
use cdk_common::common::ProofInfo;
use cdk_common::nuts::State;
use cdk_common::Amount;

use crate::wallet::Error;
use crate::Wallet;

impl Wallet {
    /// Retrieves mining share proofs using stored premint secrets
    ///
    /// Retrieve blinded secrets, query the mint for blind signatures,
    /// construct and store proofs, delete separate secret store
    #[instrument(skip_all)]
    pub async fn mint_mining_share(
        &self,
        quote_id: &str,
        secret_key: Option<crate::nuts::SecretKey>,
    ) -> Result<Vec<Proof>, Error> {
        // Retrieve the quote
        let quote = self
            .localstore
            .get_mint_quote(quote_id)
            .await?
            .ok_or(Error::UnknownQuote)?;

        // Verify it's a mining share quote and is paid
        if quote.payment_method != cdk_common::PaymentMethod::MiningShare {
            return Err(Error::InvalidPaymentMethod);
        }

        if quote.state != cdk_common::MintQuoteState::Paid {
            return Err(Error::UnpaidQuote);
        }

        // Generate premint secrets
        let keyset_id = match quote.keyset_id {
            Some(id) => id,
            None => {
                // Fallback to active keyset
                // TODO or should we just throw an error?
                let active_keyset_id = self.get_active_mint_keyset().await?.id;
                tracing::error!(
                    "Quote {} missing keyset_id, falling back to active keyset {}. This may cause minting failures if the mint expects a different keyset.",
                    quote_id,
                    active_keyset_id
                );
                active_keyset_id
            }
        };

        let amount = quote.amount.ok_or(Error::AmountUndefined)?;
        let premint_secrets = self
            .generate_premint_secrets_for_amount(amount, keyset_id)
            .await?;

        // Create mint request
        let mint_request =
            self.create_mint_request(quote_id, &premint_secrets, secret_key.as_ref())?;

        // Submit the mint request
        let mint_response = self.client.post_mint(mint_request).await?;

        // Store proofs constructed from the response
        let proofs = self
            .store_proofs_from_construction(&premint_secrets, &mint_response.signatures)
            .await?;

        tracing::debug!(
            "Successfully minted {} mining share proofs for quote {}",
            proofs.len(),
            quote_id
        );

        Ok(proofs)
    }

    /// Constructs proofs from premint secrets and blind signatures
    fn construct_proofs(
        &self,
        premint_secrets: &PreMintSecrets,
        signatures: &[BlindSignature],
    ) -> Result<Vec<Proof>, Error> {
        if premint_secrets.blinded_messages().len() != signatures.len() {
            return Err(Error::MismatchedSignatureCount);
        }

        let mut proofs = Vec::new();

        for (secret, signature) in premint_secrets.iter().zip(signatures.iter()) {
            let proof = Proof {
                amount: secret.amount,
                keyset_id: signature.keyset_id,
                secret: secret.secret.clone(),
                c: signature.c,
                witness: None, // Mining shares don't use witnesses initially
                dleq: None,
            };
            proofs.push(proof);
        }

        Ok(proofs)
    }

    /// Stores proofs constructed from premint secrets and signatures
    async fn store_proofs_from_construction(
        &self,
        premint_secrets: &PreMintSecrets,
        signatures: &[BlindSignature],
    ) -> Result<Vec<Proof>, Error> {
        let proofs = self.construct_proofs(premint_secrets, signatures)?;

        let proof_infos: Result<Vec<_>, _> = proofs
            .iter()
            .map(|proof| {
                ProofInfo::new(
                    proof.clone(),
                    self.mint_url.clone(),
                    State::Unspent,
                    self.unit.clone(),
                )
            })
            .collect();

        self.localstore.update_proofs(proof_infos?, vec![]).await?;
        Ok(proofs)
    }

    /// Creates a mint request with optional signature
    fn create_mint_request(
        &self,
        quote_id: &str,
        premint_secrets: &PreMintSecrets,
        secret_key: Option<&crate::nuts::SecretKey>,
    ) -> Result<cdk_common::nuts::MintRequest<String>, Error> {
        let mut mint_request = cdk_common::nuts::MintRequest {
            quote: quote_id.to_string(),
            outputs: premint_secrets.blinded_messages().to_vec(),
            signature: None,
        };

        if let Some(sk) = secret_key {
            mint_request.sign(sk.clone())?;
        }

        Ok(mint_request)
    }

    /// Handles mint errors with appropriate logging and returns whether to skip
    fn handle_mint_error(&self, quote_id: &str, error: &Error) -> bool {
        let error_msg = error.to_string();
        if error_msg.contains("already issued") || error_msg.contains("IssuedQuote") {
            tracing::debug!("Quote {} already issued, skipping", quote_id);
            true
        } else if error_msg.contains("UnknownQuote") {
            tracing::debug!("Quote {} not found or expired, skipping", quote_id);
            true
        } else {
            tracing::warn!("Failed to mint from quote {}: {}", quote_id, error);
            false
        }
    }

    /// Generates premint secrets for a given amount and keyset
    async fn generate_premint_secrets_for_amount(
        &self,
        amount: Amount,
        keyset_id: cdk_common::nuts::Id,
    ) -> Result<PreMintSecrets, Error> {
        let amount_split_target = SplitTarget::default();
        let num_secrets = amount
            .split_targeted(&amount_split_target)
            .unwrap_or_default()
            .len() as u32;

        let new_count = self
            .localstore
            .increment_keyset_counter(&keyset_id, num_secrets)
            .await?;
        let count = new_count - num_secrets;

        Ok(PreMintSecrets::from_xpriv(
            keyset_id,
            count,
            self.xpriv,
            amount,
            &amount_split_target,
        )?)
    }

    /// Complete mining share minting flow for stratum proxy
    ///
    /// This function handles the complete flow when a stratum proxy needs to mint tokens
    /// for quotes associated with a locking pubkey. It will:
    /// 1. Query the mint for quote IDs by pubkey (using the lookup API)
    /// 2. Generate blinded messages and mint directly from the mint
    /// 3. Construct and store the resulting proofs
    ///
    /// # Arguments
    /// * `pubkey` - The locking pubkey to lookup quotes for
    /// * `secret_key` - Optional secret key for signing mint requests (required when quotes have pubkey locks)
    ///
    /// # Returns
    /// * `Vec<Proof>` - All the minted proofs from found quotes
    #[instrument(skip_all, fields(pubkey = %pubkey))]
    pub async fn mint_tokens_for_pubkey(
        &self,
        pubkey: crate::nuts::PublicKey,
        secret_key: Option<crate::nuts::SecretKey>,
    ) -> Result<Vec<Proof>, Error> {
        // 1. Query the mint for quote IDs by pubkey (only PAID/mintable quotes)
        tracing::debug!("Looking up quotes for pubkey: {}", pubkey);
        let lookup_response = self
            .lookup_mint_quotes_by_pubkeys(
                &[pubkey],
                crate::hashpool::MintQuoteStateFilter::OnlyPaid,
            )
            .await?;

        if lookup_response.is_empty() {
            tracing::debug!("No quotes found for pubkey: {}", pubkey);
            return Ok(vec![]);
        }

        tracing::debug!(
            "Found {} quotes for pubkey: {}",
            lookup_response.len(),
            pubkey
        );

        let mut all_proofs = Vec::new();

        // 2. Process each quote: get details, generate blinded secrets, mint, store proofs
        for quote_info in lookup_response {
            tracing::debug!(
                "Processing quote: {} (method: {:?})",
                quote_info.quote,
                quote_info.method
            );

            match quote_info.method {
                crate::nuts::PaymentMethod::MiningShare => {
                    // If the quote appears in the lookup response, attempt to mint it.
                    // The mint will authoritatively reject if the quote is not mintable (e.g., already issued).

                    // Use the actual quoted amount and keyset_id from the lookup response
                    let amount = quote_info.amount;
                    let keyset_id = quote_info.keyset_id;
                    tracing::debug!(
                        "Quote {} amount: {} using keyset: {}",
                        quote_info.quote,
                        amount,
                        keyset_id
                    );

                    // Step 2: Save quote locally
                    let wallet_quote = cdk_common::wallet::MintQuote {
                        id: quote_info.quote.clone(),
                        mint_url: self.mint_url.clone(),
                        payment_method: cdk_common::PaymentMethod::MiningShare,
                        amount: Some(amount),
                        unit: self.unit.clone(),
                        request: quote_info.quote.clone(),
                        state: cdk_common::MintQuoteState::Paid,
                        expiry: 0, // Mining shares don't expire
                        secret_key: None,
                        amount_issued: Amount::ZERO,
                        amount_paid: amount,
                        keyset_id: Some(keyset_id),
                    };

                    self.localstore.add_mint_quote(wallet_quote).await?;

                    // Step 3: Use mint_mining_share to handle the minting
                    match self
                        .mint_mining_share(&quote_info.quote, secret_key.clone())
                        .await
                    {
                        Ok(proofs) => {
                            tracing::debug!(
                                "Successfully minted {} proofs from quote: {}",
                                proofs.len(),
                                quote_info.quote
                            );
                            all_proofs.extend(proofs);
                        }
                        Err(e) => {
                            self.handle_mint_error(&quote_info.quote, &e);

                            // Clean up quote on failure
                            let _ = self.localstore.remove_mint_quote(&quote_info.quote).await;
                        }
                    }
                }
                _ => {
                    tracing::debug!(
                        "Skipping non-mining-share quote: {} (method: {:?})",
                        quote_info.quote,
                        quote_info.method
                    );
                }
            }
        }

        tracing::debug!(
            "Successfully minted {} total proofs for pubkey: {}",
            all_proofs.len(),
            pubkey
        );
        Ok(all_proofs)
    }

    /// Mint tokens for multiple pubkeys
    ///
    /// Convenience function that looks up and mints tokens for multiple pubkeys at once
    #[instrument(skip_all, fields(pubkey_count = %pubkeys.len()))]
    pub async fn mint_tokens_for_pubkeys(
        &self,
        pubkeys: &[crate::nuts::PublicKey],
    ) -> Result<Vec<Proof>, Error> {
        let mut all_proofs = Vec::new();

        for pubkey in pubkeys {
            match self.mint_tokens_for_pubkey(*pubkey, None).await {
                Ok(mut proofs) => {
                    all_proofs.append(&mut proofs);
                }
                Err(e) => {
                    tracing::warn!("Failed to mint tokens for pubkey {}: {}", pubkey, e);
                    // Continue with other pubkeys even if one fails
                }
            }
        }

        Ok(all_proofs)
    }
}
