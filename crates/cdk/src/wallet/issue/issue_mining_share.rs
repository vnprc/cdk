//! Mining Share Wallet Issue Functions
//!
//! This module implements wallet-side functions for processing
//! mining share mint quotes.

use cdk_common::nuts::nut12;
use cdk_common::nuts::{MintRequest, PreMintSecrets, Proof};
use cdk_common::wallet::{Transaction, TransactionDirection};
use std::collections::HashMap;
use tracing::instrument;

use crate::dhke::construct_proofs;
use crate::nuts::ProofsMethods;
use cdk_common::amount::SplitTarget;
use cdk_common::common::ProofInfo;
use cdk_common::nuts::{MintQuoteMiningShareResponse, State};
use cdk_common::util::unix_time;
use cdk_common::Amount;

use crate::wallet::Error;
use crate::Wallet;

impl Wallet {
    /// Retrieves mining share proofs using stored premint secrets
    ///
    /// Mint tokens directly from mining share quote info
    ///
    /// This function implements direct minting without requiring local quote storage.
    /// It takes the quote information from the remote lookup and mints directly.
    /// This follows the same pattern as the standard Bolt11 mint() function.
    #[instrument(skip_all)]
    pub async fn mint_mining_share(
        &self,
        quote_id: &str,
        amount: Amount,
        keyset_id: crate::nuts::Id,
        secret_key: crate::nuts::SecretKey, // Now mandatory for NUT-20 signing
    ) -> Result<Vec<Proof>, Error> {
        // Ensure we have fresh keysets
        self.refresh_keysets().await?;

        // Generate premint secrets using provided keyset and amount
        // This follows the same counter management as bolt11 minting
        let amount_split = amount.split_targeted(&SplitTarget::default())?;
        let num_secrets = amount_split.len() as u32;

        tracing::debug!(
            "Incrementing keyset {} counter by {}",
            keyset_id,
            num_secrets
        );

        // Atomically get the counter range we need
        let new_counter = self
            .localstore
            .increment_keyset_counter(&keyset_id, num_secrets)
            .await?;
        let count = new_counter - num_secrets;

        let premint_secrets = PreMintSecrets::from_seed(
            keyset_id,
            count,
            &self.seed,
            amount,
            &SplitTarget::default(),
        )?;

        // Create and sign mint request (NUT-20 compliance)
        let mut mint_request = MintRequest {
            quote: quote_id.to_string(),
            outputs: premint_secrets.blinded_messages(),
            signature: None,
        };

        // Sign the request (mandatory for mining shares with NUT-20)
        mint_request.sign(secret_key.clone())?;

        // Submit the mint request using dedicated mining share endpoint
        let mint_response = self.client.post_mint_mining_share(mint_request).await?;

        // Load keyset for DLEQ verification
        let keys = self.load_keyset_keys(keyset_id).await?;

        // Verify DLEQ proofs (same as bolt11)
        for (sig, premint) in mint_response
            .signatures
            .iter()
            .zip(&premint_secrets.secrets)
        {
            let keys = self.load_keyset_keys(sig.keyset_id).await?;
            let key = keys.amount_key(sig.amount).ok_or(Error::AmountKey)?;
            match sig.verify_dleq(key, premint.blinded_message.blinded_secret) {
                Ok(_) | Err(nut12::Error::MissingDleqProof) => (),
                Err(_) => return Err(Error::CouldNotVerifyDleq),
            }
        }

        // Construct proofs from signatures and secrets (same as bolt11)
        let proofs = construct_proofs(
            mint_response.signatures,
            premint_secrets.rs(),
            premint_secrets.secrets(),
            &keys,
        )?;

        // Store proofs in wallet
        let proof_infos = proofs
            .iter()
            .map(|proof| {
                ProofInfo::new(
                    proof.clone(),
                    self.mint_url.clone(),
                    State::Unspent,
                    self.unit.clone(),
                )
            })
            .collect::<Result<Vec<ProofInfo>, _>>()?;

        self.localstore.update_proofs(proof_infos, vec![]).await?;

        // Add transaction record (same as bolt11)
        self.localstore
            .add_transaction(Transaction {
                mint_url: self.mint_url.clone(),
                direction: TransactionDirection::Incoming,
                amount: proofs.total_amount()?,
                fee: Amount::ZERO,
                unit: self.unit.clone(),
                ys: proofs.ys()?,
                timestamp: unix_time(),
                memo: None,
                metadata: HashMap::new(),
            })
            .await?;

        tracing::debug!(
            "Successfully minted {} mining share proofs for quote {} (amount: {})",
            proofs.len(),
            quote_id,
            amount
        );

        Ok(proofs)
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

    /// Complete mining share minting flow
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
        // TODO make mandatory
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

                    // Step 2: Mint directly without local quote storage
                    match secret_key.clone() {
                        Some(sk) => {
                            match self
                                .mint_mining_share(&quote_info.quote, amount, keyset_id, sk)
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
                                    // No local quote cleanup needed since we don't store it
                                }
                            }
                        }
                        None => {
                            tracing::error!(
                                "Secret key is required for mining share minting (NUT-20)"
                            );
                            let error = Error::SignatureMissingOrInvalid;
                            self.handle_mint_error(&quote_info.quote, &error);
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

    /// Check mining share mint quote status
    #[instrument(skip(self, quote_id))]
    pub async fn mint_quote_state_mining_share(
        &self,
        quote_id: &str,
    ) -> Result<MintQuoteMiningShareResponse<String>, Error> {
        let response = self
            .client
            .get_mint_quote_status_mining_share(quote_id)
            .await?;

        match self.localstore.get_mint_quote(quote_id).await? {
            Some(quote) => {
                // Update existing local quote with current state and keyset_id
                let mut quote = quote;
                quote.state = response.state.into();
                quote.keyset_id = Some(response.keyset_id);
                self.localstore.add_mint_quote(quote).await?;
            }
            None => {
                // Create new local quote record from the API response
                tracing::info!("Creating local record for mining share quote {}", quote_id);

                let wallet_quote = cdk_common::wallet::MintQuote {
                    id: quote_id.to_string(),
                    mint_url: self.mint_url.clone(),
                    payment_method: cdk_common::PaymentMethod::MiningShare,
                    amount: response.amount,
                    unit: response.unit.clone().unwrap_or(self.unit.clone()),
                    request: response.request.clone(),
                    state: response.state.into(),
                    expiry: response.expiry.unwrap_or(0),
                    secret_key: None,
                    amount_issued: Amount::ZERO,
                    amount_paid: response.amount.unwrap_or(Amount::ZERO),
                    keyset_id: Some(response.keyset_id),
                };

                self.localstore.add_mint_quote(wallet_quote).await?;
            }
        }

        Ok(response)
    }
}
