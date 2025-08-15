//! Mining Share Wallet Issue Functions
//!
//! This module implements wallet-side functions for creating and processing
//! mining share mint quotes.

use bitcoin::hashes::sha256;
use cdk_common::nuts::{BlindSignature, PreMintSecrets, Proof};
use tracing::instrument;

use cdk_common::amount::SplitTarget;
use cdk_common::common::ProofInfo;
use cdk_common::nuts::State;
use cdk_common::wallet::MintQuote;
use cdk_common::Amount;

use crate::wallet::Error;
use crate::Wallet;

impl Wallet {
    /// Creates a mint quote for mining shares with blinded secrets
    ///
    /// Generates and stores quote and premint secrets locally, then returns
    /// the request structure for transmission to pool via stratum protocol.
    #[instrument(skip_all)]
    pub async fn mint_mining_share_quote(
        &self,
        amount: Amount,
        header_hash: sha256::Hash,
    ) -> Result<cdk_common::nuts::nutXX::MintQuoteMiningShareRequest, Error> {
        let unit = self.unit.clone();

        // Validate amount is within mining share limits (1-256)
        if amount == Amount::ZERO || amount > Amount::from(256) {
            return Err(Error::AmountOutofLimitRange(
                Amount::from(1),
                Amount::from(256),
                amount,
            ));
        }

        let quote = MintQuote {
            // TODO is there a better id value? mint will overwrite this
            id: header_hash.to_string(),
            mint_url: self.mint_url.clone(),
            payment_method: cdk_common::PaymentMethod::MiningShare,
            amount: Some(amount),
            unit: unit.clone(),
            request: header_hash.to_string(), // Use header hash as request identifier
            state: cdk_common::MintQuoteState::Paid, // Mining shares are immediately paid
            expiry: 0,                        // No expiry for mining shares
            secret_key: None,
            amount_issued: Amount::ZERO,
            amount_paid: amount,
        };

        // Get active keyset for this mint and unit (local only, no network call)
        let active_keyset = self.get_active_mint_keyset_local().await?;

        let amount_split_target = SplitTarget::default();

        // Calculate how many secrets we'll need (mining shares don't use spending conditions)
        let num_secrets = amount
            .split_targeted(&amount_split_target)
            .unwrap_or_default()
            .len() as u32;

        // Atomically increment counter and get the new value
        let new_count = self
            .localstore
            .increment_keyset_counter(&active_keyset.id, num_secrets)
            .await?;

        // Calculate the starting counter value for this batch (before our increment)
        let count = new_count - num_secrets;

        // Generate premint secrets using the counter value
        let premint_secrets = PreMintSecrets::from_xpriv(
            active_keyset.id,
            count,
            self.xpriv,
            amount,
            &amount_split_target,
        )?;

        // Store the quote and secrets locally
        self.localstore.add_mint_quote(quote.clone()).await?;
        self.localstore
            .add_premint_secrets(&quote.id, premint_secrets.clone())
            .await?;

        // Create request structure for stratum transmission
        let mining_share_request = cdk_common::nuts::nutXX::MintQuoteMiningShareRequest {
            amount,
            unit: unit.clone(),
            header_hash,
            description: None,
            pubkey: None, // TODO: NUT-20 support
            keyset_id: active_keyset.id,
        };

        tracing::debug!(
            "Created mining share mint quote {} for {} {} with header hash {}",
            quote.id,
            amount,
            unit,
            header_hash
        );

        Ok(mining_share_request)
    }

    /// Retrieves mining share proofs using stored premint secrets
    ///
    /// Retrieve blinded secrets, query the mint for blind signatures,
    /// construct and store proofs, delete separate secret store
    #[instrument(skip_all)]
    pub async fn mint_mining_share(&self, quote_id: &str) -> Result<Vec<Proof>, Error> {
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

        // Retrieve stored secrets
        let premint_secrets = self
            .localstore
            .get_premint_secrets(quote_id)
            .await?
            .ok_or(Error::PreMintSecretsNotFound)?;

        // Create mint request
        let mint_request = cdk_common::nuts::MintRequest {
            quote: quote_id.to_string(),
            outputs: premint_secrets.blinded_messages().to_vec(),
            signature: None,
        };

        // Submit the mint request
        let mint_response = self.client.post_mint(mint_request).await?;

        // Construct proofs from the response
        let proofs = self.construct_proofs(&premint_secrets, &mint_response.signatures)?;

        // Store the proofs
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

        // Clean up premint secrets
        self.localstore.remove_premint_secrets(quote_id).await?;

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
        // 1. Query the mint for quote IDs by pubkey
        tracing::debug!("Looking up quotes for pubkey: {}", pubkey);
        let lookup_response = self.lookup_mint_quotes_by_pubkeys(&[pubkey]).await?;

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
                    // Use the actual quoted amount and keyset_id from the lookup response
                    let amount = quote_info.amount;
                    let keyset_id = quote_info.keyset_id;
                    tracing::debug!(
                        "Quote {} amount: {} using keyset: {}",
                        quote_info.quote,
                        amount,
                        keyset_id
                    );

                    // Step 2: Generate blinded secrets and save quote locally
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

                    let premint_secrets = PreMintSecrets::from_xpriv(
                        keyset_id,
                        count,
                        self.xpriv,
                        amount,
                        &amount_split_target,
                    )?;

                    // Save quote locally
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
                    };

                    self.localstore.add_mint_quote(wallet_quote).await?;
                    self.localstore
                        .add_premint_secrets(&quote_info.quote, premint_secrets.clone())
                        .await?;

                    // Step 3: Request mint with blinded messages
                    let mut mint_request = cdk_common::nuts::MintRequest {
                        quote: quote_info.quote.clone(),
                        outputs: premint_secrets.blinded_messages().to_vec(),
                        signature: None,
                    };

                    // Sign the request if we have a secret key (required for pubkey-locked quotes)
                    if let Some(ref sk) = secret_key {
                        mint_request.sign(sk.clone())?;
                    }

                    match self.client.post_mint(mint_request).await {
                        Ok(mint_response) => {
                            // Step 4: Construct proofs and store them
                            let proofs =
                                self.construct_proofs(&premint_secrets, &mint_response.signatures)?;

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

                            // Clean up: remove quote and premint secrets
                            let _ = self.localstore.remove_mint_quote(&quote_info.quote).await;
                            let _ = self
                                .localstore
                                .remove_premint_secrets(&quote_info.quote)
                                .await;

                            tracing::debug!(
                                "Successfully minted {} proofs from quote: {}",
                                proofs.len(),
                                quote_info.quote
                            );
                            all_proofs.extend(proofs);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to mint from quote {}: {}", quote_info.quote, e);
                            // Clean up on failure
                            let _ = self.localstore.remove_mint_quote(&quote_info.quote).await;
                            let _ = self
                                .localstore
                                .remove_premint_secrets(&quote_info.quote)
                                .await;
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
