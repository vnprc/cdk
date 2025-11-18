use std::collections::HashMap;

use cdk_common::mint::{BatchMintRequest, BatchQuoteStatusRequest};
use tracing::instrument;

use crate::amount::SplitTarget;
use crate::dhke::construct_proofs;
use crate::nuts::nut00::ProofsMethods;
use crate::nuts::{nut12, PreMintSecrets, Proofs, SpendingConditions};
use crate::types::ProofInfo;
use crate::util::unix_time;
use crate::wallet::MintQuoteState;
use crate::{Amount, Error, Wallet};

impl Wallet {
    /// Mint batch of proofs from multiple quotes
    ///
    /// # Arguments
    /// * `quote_ids` - List of quote IDs to mint from
    /// * `amount_split_target` - Target split for the amount
    /// * `spending_conditions` - Optional spending conditions (not yet supported for batches)
    ///
    /// # Returns
    /// * Vector of minted proofs in deterministic order
    ///
    /// # Errors
    /// * Returns error if quotes are from different mints
    /// * Returns error if quotes are from different payment methods
    /// * Returns error if any quote is unknown
    /// * Returns error if any quote is not in PAID state
    #[instrument(skip(self, spending_conditions), fields(quote_count = quote_ids.len()))]
    pub async fn mint_batch(
        &self,
        quote_ids: Vec<String>,
        amount_split_target: SplitTarget,
        spending_conditions: Option<SpendingConditions>,
    ) -> Result<Proofs, Error> {
        if quote_ids.is_empty() {
            return Err(Error::AmountUndefined);
        }

        // Fetch all quote details
        let mut quote_infos = Vec::new();
        for quote_id in &quote_ids {
            let quote_info = self
                .localstore
                .get_mint_quote(quote_id)
                .await?
                .ok_or(Error::UnknownQuote)?;
            quote_infos.push(quote_info);
        }

        // Validate all quotes are from same payment method
        let payment_method = &quote_infos[0].payment_method;
        for quote_info in &quote_infos {
            if &quote_info.payment_method != payment_method {
                return Err(Error::UnsupportedPaymentMethod);
            }
        }

        // Validate all quotes have same unit
        let unit = &quote_infos[0].unit;
        for quote_info in &quote_infos {
            if &quote_info.unit != unit {
                return Err(Error::UnsupportedUnit);
            }
        }

        // Validate all quotes are in PAID state
        for quote_info in &quote_infos {
            if quote_info.state != MintQuoteState::Paid {
                return Err(Error::UnknownQuote); // Quote not ready
            }
        }

        // Check all quotes are not expired
        let unix_time_now = unix_time();
        for quote_info in &quote_infos {
            if quote_info.expiry <= unix_time_now {
                tracing::warn!("Attempting to mint with expired quote.");
                // Continue anyway - server will validate expiry
            }
        }

        // Calculate total amount
        let mut total_amount = Amount::ZERO;
        for quote_info in &quote_infos {
            total_amount += quote_info.amount_mintable();
        }

        if total_amount == Amount::ZERO {
            return Err(Error::AmountUndefined);
        }

        let active_keyset_id = self.fetch_active_keyset().await?.id;
        let fee_and_amounts = self
            .get_keyset_fees_and_amounts_by_id(active_keyset_id)
            .await?;

        let premint_secrets = match &spending_conditions {
            Some(spending_conditions) => PreMintSecrets::with_conditions(
                active_keyset_id,
                total_amount,
                &amount_split_target,
                spending_conditions,
                &fee_and_amounts,
            )?,
            None => {
                // Calculate how many secrets we'll need
                let amount_split =
                    total_amount.split_targeted(&amount_split_target, &fee_and_amounts)?;
                let num_secrets = amount_split.len() as u32;

                tracing::debug!(
                    "Incrementing keyset {} counter by {} for batch mint",
                    active_keyset_id,
                    num_secrets
                );

                // Atomically get the counter range we need
                let new_counter = self
                    .localstore
                    .increment_keyset_counter(&active_keyset_id, num_secrets)
                    .await?;

                let count = new_counter - num_secrets;

                PreMintSecrets::from_seed(
                    active_keyset_id,
                    count,
                    &self.seed,
                    total_amount,
                    &amount_split_target,
                    &fee_and_amounts,
                )?
            }
        };

        // Build the batch mint request
        // NUT-20 signature support can be added here when spending_condition is available on MintQuote
        let batch_request = BatchMintRequest {
            quote: quote_ids.clone(),
            outputs: premint_secrets.blinded_messages(),
            signature: None, // NUT-20 signatures deferred - requires quote details with spending_condition
        };

        // First check all quotes status before minting
        let batch_status_request = BatchQuoteStatusRequest {
            quote: quote_ids.clone(),
        };

        let _batch_status = self
            .client
            .post_mint_batch_quote_status(batch_status_request)
            .await?;

        // Now mint the batch
        let mint_res = self.client.post_mint_batch(batch_request).await?;

        let keys = self.load_keyset_keys(active_keyset_id).await?;

        // Verify the signature DLEQ is valid for all signatures
        {
            for (sig, premint) in mint_res.signatures.iter().zip(&premint_secrets.secrets) {
                let keys = self.load_keyset_keys(sig.keyset_id).await?;
                let key = keys.amount_key(sig.amount).ok_or(Error::AmountKey)?;
                match sig.verify_dleq(key, premint.blinded_message.blinded_secret) {
                    Ok(_) | Err(nut12::Error::MissingDleqProof) => (),
                    Err(_) => return Err(Error::CouldNotVerifyDleq),
                }
            }
        }

        let proofs = construct_proofs(
            mint_res.signatures,
            premint_secrets.rs(),
            premint_secrets.secrets(),
            &keys,
        )?;

        // Remove all filled quotes from store
        for quote_id in quote_ids.iter() {
            self.localstore.remove_mint_quote(quote_id).await?;
        }

        let proof_infos = proofs
            .iter()
            .map(|proof| {
                ProofInfo::new(
                    proof.clone(),
                    self.mint_url.clone(),
                    crate::nuts::State::Unspent,
                    unit.to_owned(),
                )
            })
            .collect::<Result<Vec<ProofInfo>, _>>()?;

        // Add new proofs to store
        self.localstore.update_proofs(proof_infos, vec![]).await?;

        // Add transaction to store
        let batch_ids = quote_ids.join(",");
        self.localstore
            .add_transaction(crate::wallet::types::Transaction {
                mint_url: self.mint_url.clone(),
                direction: crate::wallet::types::TransactionDirection::Incoming,
                amount: proofs.total_amount()?,
                fee: Amount::ZERO,
                unit: self.unit.clone(),
                ys: proofs.ys()?,
                timestamp: unix_time_now,
                memo: None,
                metadata: HashMap::new(),
                quote_id: Some(batch_ids),
                payment_request: Some(quote_infos[0].request.clone()),
                payment_proof: None,
            })
            .await?;

        Ok(proofs)
    }
}
