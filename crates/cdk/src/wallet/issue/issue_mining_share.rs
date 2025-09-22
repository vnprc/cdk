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
                quote_id: Some(quote_id.to_string()),
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
                    amount_issued: response.amount_issued,
                    amount_paid: response.amount.unwrap_or(Amount::ZERO),
                    keyset_id: Some(response.keyset_id),
                };

                self.localstore.add_mint_quote(wallet_quote).await?;
            }
        }

        Ok(response)
    }
}
