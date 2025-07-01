use std::collections::HashMap;

use bitcoin_hashes::sha256::Hash;
use cdk_common::nut04::MintMethodOptions;
use cdk_common::nutXX::{MintQuoteMiningShareRequest, MintQuoteMiningShareResponse};
use cdk_common::wallet::{MintQuote as WalletMintQuote, Transaction, TransactionDirection};
use cdk_common::{CurrencyUnit, Id};
use tracing::instrument;
use uuid::Uuid;

use super::MintQuote;
use crate::amount::SplitTarget;
use crate::dhke::construct_proofs;
use crate::nuts::nut00::ProofsMethods;
use crate::nuts::{
    nut12, MintQuoteBolt11Request, MintQuoteBolt11Response, MintRequest, PreMintSecrets, Proofs,
    SecretKey, SpendingConditions, State,
};
use crate::types::ProofInfo;
use crate::util::unix_time;
use crate::wallet::MintQuoteState;
use crate::{Amount, Error, Wallet};

impl Wallet {
    /// Mint Quote
    /// # Synopsis
    /// ```rust,no_run
    /// use std::sync::Arc;
    ///
    /// use cdk::amount::Amount;
    /// use cdk::nuts::CurrencyUnit;
    /// use cdk::wallet::Wallet;
    /// use cdk_sqlite::wallet::memory;
    /// use rand::random;
    ///
    /// #[tokio::main]
    /// async fn main() -> anyhow::Result<()> {
    ///     let seed = random::<[u8; 32]>();
    ///     let mint_url = "https://fake.thesimplekid.dev";
    ///     let unit = CurrencyUnit::Sat;
    ///
    ///     let localstore = memory::empty().await?;
    ///     let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), &seed, None)?;
    ///     let amount = Amount::from(100);
    ///
    ///     let quote = wallet.mint_quote(amount, None).await?;
    ///     Ok(())
    /// }
    /// ```
    #[instrument(skip(self))]
    pub async fn mint_quote(
        &self,
        amount: Amount,
        description: Option<String>,
    ) -> Result<MintQuote, Error> {
        let mint_url = self.mint_url.clone();
        let unit = self.unit.clone();

        // If we have a description, we check that the mint supports it.
        if description.is_some() {
            let settings = self
                .localstore
                .get_mint(mint_url.clone())
                .await?
                .ok_or(Error::IncorrectMint)?
                .nuts
                .nut04
                .get_settings(&unit, &crate::nuts::PaymentMethod::Bolt11)
                .ok_or(Error::UnsupportedUnit)?;

            match settings.options {
                Some(MintMethodOptions::Bolt11 { description }) if description => (),
                _ => return Err(Error::InvoiceDescriptionUnsupported),
            }
        }

        let secret_key = SecretKey::generate();

        let request = MintQuoteBolt11Request {
            amount,
            unit: unit.clone(),
            description,
            pubkey: Some(secret_key.public_key()),
        };

        let quote_res = self.client.post_mint_quote(request).await?;

        let quote = MintQuote {
            mint_url,
            id: quote_res.quote,
            amount,
            unit,
            request: quote_res.request,
            state: quote_res.state,
            expiry: quote_res.expiry.unwrap_or(0),
            secret_key: Some(secret_key),
        };

        self.localstore.add_mint_quote(quote.clone()).await?;

        Ok(quote)
    }

    /// Create new mint mining share quote
    #[instrument(skip_all)]
    pub async fn get_mint_mining_share_quote(
        &self,
        mint_quote_request: MintQuoteMiningShareRequest,
    ) -> Result<WalletMintQuote, Error> {
        let MintQuoteMiningShareRequest {
            amount,
            unit,
            header_hash,
            description,
            pubkey,
        } = mint_quote_request;

        // self.check_mint_request_acceptable(amount, &unit).await?;

        // let ln = self.get_payment_processor(unit.clone(), PaymentMethod::Bolt11)?;

        //TODO 5 minutes? idk, just pick a number
        let mint_ttl = 600_000; //self.localstore.get_quote_ttl().await?.mint_ttl;

        let quote_expiry = unix_time() + mint_ttl;

        // let settings = ln.get_settings().await?;
        // let settings: Bolt11Settings = serde_json::from_value(settings)?;

        // if description.is_some() && !settings.invoice_description {
        //     tracing::error!("Backend does not support invoice description");
        //     return Err(Error::InvoiceDescriptionUnsupported);
        // }

        // let create_invoice_response = ln
        //     .create_incoming_payment_request(
        //         amount,
        //         &unit,
        //         description.unwrap_or("".to_string()),
        //         Some(quote_expiry),
        //     )
        //     .await
        //     .map_err(|err| {
        //         tracing::error!("Could not create invoice: {}", err);
        //         Error::InvalidPaymentRequest
        //     })?;

        // TODO get ttl from somewhere else
        let mint_ttl = 500_000;
        let expiry = unix_time() + mint_ttl;
        let now = unix_time();

        let quote = MintQuote {
            // will be overwritten by the mint
            id: header_hash.to_string(),
            mint_url: self.mint_url.clone(),
            amount: amount,
            unit: unit.clone(),
            request: header_hash.to_string(),
            state: MintQuoteState::Paid,
            expiry,
            secret_key: None,
        };

        tracing::debug!(
            "New mint quote {} for {} {} with request id {}",
            quote.id,
            amount,
            unit,
            header_hash.to_string(),
        );

        self.localstore.add_mint_quote(quote.clone()).await?;

        // self.pubsub_manager
        //     .broadcast(NotificationPayload::MintQuoteBolt11Response(quote.clone()));

        Ok(quote)
    }

    /// Check mint quote status
    #[instrument(skip(self, quote_id))]
    pub async fn mint_quote_state(
        &self,
        quote_id: &str,
    ) -> Result<MintQuoteBolt11Response<String>, Error> {
        let response = self.client.get_mint_quote_status(quote_id).await?;

        match self.localstore.get_mint_quote(quote_id).await? {
            Some(quote) => {
                let mut quote = quote;

                quote.state = response.state;
                self.localstore.add_mint_quote(quote).await?;
            }
            None => {
                tracing::info!("Quote mint {} unknown", quote_id);
            }
        }

        Ok(response)
    }

    /// Check status of pending mint quotes
    #[instrument(skip(self))]
    pub async fn check_all_mint_quotes(&self) -> Result<Amount, Error> {
        let mint_quotes = self.localstore.get_mint_quotes().await?;
        let mut total_amount = Amount::ZERO;

        for mint_quote in mint_quotes {
            let mint_quote_response = self.mint_quote_state(&mint_quote.id).await?;

            if mint_quote_response.state == MintQuoteState::Paid {
                let proofs = self
                    .mint(&mint_quote.id, SplitTarget::default(), None)
                    .await?;
                total_amount += proofs.total_amount()?;
            } else if mint_quote.expiry.le(&unix_time()) {
                self.localstore.remove_mint_quote(&mint_quote.id).await?;
            }
        }
        Ok(total_amount)
    }

    /// Mint
    /// # Synopsis
    /// ```rust,no_run
    /// use std::sync::Arc;
    ///
    /// use anyhow::Result;
    /// use cdk::amount::{Amount, SplitTarget};
    /// use cdk::nuts::nut00::ProofsMethods;
    /// use cdk::nuts::CurrencyUnit;
    /// use cdk::wallet::Wallet;
    /// use cdk_sqlite::wallet::memory;
    /// use rand::random;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let seed = random::<[u8; 32]>();
    ///     let mint_url = "https://fake.thesimplekid.dev";
    ///     let unit = CurrencyUnit::Sat;
    ///
    ///     let localstore = memory::empty().await?;
    ///     let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), &seed, None).unwrap();
    ///     let amount = Amount::from(100);
    ///
    ///     let quote = wallet.mint_quote(amount, None).await?;
    ///     let quote_id = quote.id;
    ///     // To be called after quote request is paid
    ///     let minted_proofs = wallet.mint(&quote_id, SplitTarget::default(), None).await?;
    ///     let minted_amount = minted_proofs.total_amount()?;
    ///
    ///     Ok(())
    /// }
    /// ```
    #[instrument(skip(self))]
    pub async fn mint(
        &self,
        quote_id: &str,
        amount_split_target: SplitTarget,
        spending_conditions: Option<SpendingConditions>,
    ) -> Result<Proofs, Error> {
        // Check that mint is in store of mints
        if self
            .localstore
            .get_mint(self.mint_url.clone())
            .await?
            .is_none()
        {
            self.get_mint_info().await?;
        }

        let quote_info = self
            .localstore
            .get_mint_quote(quote_id)
            .await?
            .ok_or(Error::UnknownQuote)?;

        let unix_time = unix_time();

        if quote_info.expiry > unix_time {
            tracing::warn!("Attempting to mint with expired quote.");
        }

        let active_keyset_id = self.get_active_mint_keyset().await?.id;

        let count = self
            .localstore
            .get_keyset_counter(&active_keyset_id)
            .await?;

        let count = count.map_or(0, |c| c + 1);

        let premint_secrets = match &spending_conditions {
            Some(spending_conditions) => PreMintSecrets::with_conditions(
                active_keyset_id,
                quote_info.amount,
                &amount_split_target,
                spending_conditions,
            )?,
            None => PreMintSecrets::from_xpriv(
                active_keyset_id,
                count,
                self.xpriv,
                quote_info.amount,
                &amount_split_target,
            )?,
        };

        let mut request = MintRequest {
            quote: quote_id.to_string(),
            outputs: premint_secrets.blinded_messages(),
            signature: None,
        };

        if let Some(secret_key) = quote_info.secret_key {
            request.sign(secret_key)?;
        }

        let mint_res = self.client.post_mint(request).await?;

        let keys = self.get_keyset_keys(active_keyset_id).await?;

        // Verify the signature DLEQ is valid
        {
            for (sig, premint) in mint_res.signatures.iter().zip(&premint_secrets.secrets) {
                let keys = self.get_keyset_keys(sig.keyset_id).await?;
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

        // Remove filled quote from store
        self.localstore.remove_mint_quote(&quote_info.id).await?;

        if spending_conditions.is_none() {
            tracing::debug!(
                "Incrementing keyset {} counter by {}",
                active_keyset_id,
                proofs.len()
            );

            // Update counter for keyset
            self.localstore
                .increment_keyset_counter(&active_keyset_id, proofs.len() as u32)
                .await?;
        }

        let proof_infos = proofs
            .iter()
            .map(|proof| {
                ProofInfo::new(
                    proof.clone(),
                    self.mint_url.clone(),
                    State::Unspent,
                    quote_info.unit.clone(),
                )
            })
            .collect::<Result<Vec<ProofInfo>, _>>()?;

        // Add new proofs to store
        self.localstore.update_proofs(proof_infos, vec![]).await?;

        // Add transaction to store
        self.localstore
            .add_transaction(Transaction {
                mint_url: self.mint_url.clone(),
                direction: TransactionDirection::Incoming,
                amount: proofs.total_amount()?,
                fee: Amount::ZERO,
                unit: self.unit.clone(),
                ys: proofs.ys()?,
                timestamp: unix_time,
                memo: None,
                metadata: HashMap::new(),
            })
            .await?;

        Ok(proofs)
    }

    fn generate_premint_secrets(
        &self,
        active_keyset_id: Id,
        quote_info_amount: Amount,
        amount_split_target: &SplitTarget,
        spending_conditions: Option<&SpendingConditions>,
        count: u32,
    ) -> Result<PreMintSecrets, Error> {
        let premint_secrets = match &spending_conditions {
            Some(spending_conditions) => PreMintSecrets::with_conditions(
                active_keyset_id,
                quote_info_amount,
                amount_split_target,
                spending_conditions,
            )?,
            None => PreMintSecrets::from_xpriv(
                active_keyset_id,
                count,
                self.xpriv,
                quote_info_amount,
                amount_split_target,
            )?,
        };

        Ok(premint_secrets)
    }

    /// Creates and stores pre-mint secrets for a given quote.
    ///
    /// This function checks whether a mint quote already exists, creates a new quote if necessary,
    /// generates pre-mint secrets, and stores them in the local wallet store.
    ///
    /// # Arguments
    ///
    /// * `amount` - The amount for which pre-mint secrets should be generated.
    /// * `quote_id` - A unique identifier for the mint quote.
    /// * `mint_url` - The URL of the mint.
    /// * `currency_unit` - nut00 currency unit
    ///
    /// # Returns
    ///
    /// Returns a `PreMintSecrets` object containing the generated secrets.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The quote ID already exists (`Error::PaidQuote`).
    /// - Adding the quote or pre-mint secrets fails.
    pub async fn create_premint_secrets(
        &self,
        amount: u64,
        quote_id: &str,
        mint_url: &str,
        currency_unit: CurrencyUnit,
    ) -> Result<PreMintSecrets, Error> {
        // Ensure the quote does not already exist
        if self.localstore.get_mint_quote(quote_id).await?.is_some() {
            return Err(Error::PaidQuote);
        }

        // TODO prob can't convert string to bytes like that
        let header_hash = Hash::hash(quote_id.as_bytes());

        let mint_quote_request = MintQuoteMiningShareRequest {
            amount: Amount::from(amount),
            unit: currency_unit,
            header_hash,
            description: None,
            pubkey: None,
        };

        // Create and store a new mint quote
        let mint_quote = self
            .get_mint_mining_share_quote(mint_quote_request)
            .await
            .unwrap();

        self.localstore.add_mint_quote(mint_quote).await?;

        let active_keyset_id = self.get_active_mint_keyset_local().await?.id;

        // Retrieve the keyset counter, defaulting to 0 if not found
        let count = self
            .localstore
            .get_keyset_counter(&active_keyset_id)
            .await?
            .unwrap_or(0);

        let premint_secrets = self.generate_premint_secrets(
            active_keyset_id,
            Amount::from(amount),
            &SplitTarget::None,
            None,
            count,
        )?;

        self.localstore
            .add_premint_secrets(quote_id, &premint_secrets)
            .await?;

        let num_secrets: u32 = premint_secrets
            .secrets
            .len()
            .try_into()
            .map_err(|_| Error::AmountOverflow)?;

        // TODO handle result
        let result = self
            .localstore
            .increment_keyset_counter(&active_keyset_id, num_secrets)
            .await;

        Ok(premint_secrets)
    }

    /// Retrieve proofs from the mint
    ///
    /// This function assumes the share hash is saved as the quote id in localstore
    /// Caller passes in mint quote id (UUID)
    /// Create POST request body and retrieve proofs from mint
    /// Verify proofs
    /// Delete quote from localstore
    /// Save proofs to localstore
    pub async fn get_mining_share_proofs(
        &self,
        quote_id: &str,
        share_hash: &str,
    ) -> Result<Proofs, Error> {
        let quote = self
            .localstore
            .get_mint_quote(share_hash)
            .await?
            .expect("No quote found");

        // TODO pass this in, it will break if the keyset changes before getting proofs
        let active_keyset_id = self.get_active_mint_keyset_local().await?.id;
        let keys = self.get_keyset_keys(active_keyset_id).await?;

        let premint_secrets = self
            .localstore
            .get_premint_secrets(share_hash)
            .await?
            .ok_or(Error::UnknownQuote)?;

        if premint_secrets.keyset_id != active_keyset_id {
            return Err(Error::UnknownKeySet);
        }

        // get blind signatures from the mint
        let request = MintRequest {
            quote: quote_id.to_string(),
            outputs: premint_secrets.blinded_messages(),
            signature: None,
        };

        // TODO add NUT-20 support
        // if let Some(secret_key) = quote.secret_key {
        //     request.sign(secret_key)?;
        // }

        let mint_res = self.client.post_mint(request).await?;

        // Verify the signature DLEQ is valid
        {
            for (sig, premint) in mint_res.signatures.iter().zip(&premint_secrets.secrets) {
                let keys = self.get_keyset_keys(sig.keyset_id).await?;
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

        // Remove filled quote from store
        self.localstore.remove_mint_quote(&quote.id).await?;

        // copied from the mint function. idk what this is, figure it out later
        // if spending_conditions.is_none() {
        tracing::debug!(
            "Incrementing keyset {} counter by {}",
            active_keyset_id,
            proofs.len()
        );

        // Update counter for keyset
        self.localstore
            .increment_keyset_counter(&active_keyset_id, proofs.len() as u32)
            .await?;
        // }

        let proof_infos = proofs
            .iter()
            .map(|proof| {
                ProofInfo::new(
                    proof.clone(),
                    self.mint_url.clone(),
                    State::Unspent,
                    quote.unit.clone(),
                )
            })
            .collect::<Result<Vec<ProofInfo>, _>>()?;

        // Add new proofs to store
        self.localstore.update_proofs(proof_infos, vec![]).await?;

        Ok(proofs)
    }
}
