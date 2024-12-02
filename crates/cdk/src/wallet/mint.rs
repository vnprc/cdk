use tracing::instrument;

use super::MintQuote;
use crate::amount::SplitTarget;
use crate::dhke::construct_proofs;
use crate::nuts::nut00::ProofsMethods;
use crate::nuts::{
    nut12, BlindSignature, MintBolt11Request, MintQuoteBolt11Request, MintQuoteBolt11Response,
    PreMint, PreMintSecrets, SpendingConditions, State,
};
use crate::nuts::{CurrencyUnit, Id};
use crate::types::ProofInfo;
use crate::util::unix_time;
use crate::wallet::MintQuoteState;
use crate::{Amount, Error, Wallet};

impl Wallet {
    /// Mint Quote
    /// # Synopsis
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use cdk::amount::Amount;
    /// use cdk::cdk_database::WalletMemoryDatabase;
    /// use cdk::nuts::CurrencyUnit;
    /// use cdk::wallet::Wallet;
    /// use rand::Rng;
    ///
    /// #[tokio::main]
    /// async fn main() -> anyhow::Result<()> {
    ///     let seed = rand::thread_rng().gen::<[u8; 32]>();
    ///     let mint_url = "https://testnut.cashu.space";
    ///     let unit = CurrencyUnit::Sat;
    ///
    ///     let localstore = WalletMemoryDatabase::default();
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
            let mint_method_settings = self
                .localstore
                .get_mint(mint_url.clone())
                .await?
                .ok_or(Error::IncorrectMint)?
                .nuts
                .nut04
                .get_settings(&unit, &crate::nuts::PaymentMethod::Bolt11)
                .ok_or(Error::UnsupportedUnit)?;

            if !mint_method_settings.description {
                return Err(Error::InvoiceDescriptionUnsupported);
            }
        }

        let request = MintQuoteBolt11Request {
            amount,
            unit: unit.clone(),
            description,
        };

        let quote_res = self
            .client
            .post_mint_quote(mint_url.clone(), request)
            .await?;

        let quote = MintQuote {
            mint_url,
            id: quote_res.quote.clone(),
            amount,
            unit: unit.clone(),
            request: quote_res.request,
            state: quote_res.state,
            expiry: quote_res.expiry.unwrap_or(0),
        };

        self.localstore.add_mint_quote(quote.clone()).await?;

        Ok(quote)
    }

    /// Check mint quote status
    #[instrument(skip(self, quote_id))]
    pub async fn mint_quote_state(&self, quote_id: &str) -> Result<MintQuoteBolt11Response, Error> {
        let response = self
            .client
            .get_mint_quote_status(self.mint_url.clone(), quote_id)
            .await?;

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
                let amount = self
                    .mint(&mint_quote.id, SplitTarget::default(), None)
                    .await?;
                total_amount += amount;
            } else if mint_quote.expiry.le(&unix_time()) {
                self.localstore.remove_mint_quote(&mint_quote.id).await?;
            }
        }
        Ok(total_amount)
    }

    /// Generates blinded secrets to send to the mint for signing. This function
    /// is appropriate if the caller is providing their own network
    /// transport. Otherwise use `mint`, which makes a network request to
    /// the mint.
    ///
    /// # Parameters
    ///
    /// - `&self`: A reference to the current instance
    /// - `active_keyset_id`: The ID of the active keyset
    /// - `quote_info_amount`: The amount to be minted
    /// - `amount_split_target`: Strategy for splitting amount into discrete
    ///   tokens
    /// - `spending_conditions`: Optional spending conditions to apply to the
    ///   minted tokens
    /// - `count`: How many tokens were previously generated from this keyset +
    ///   1
    ///
    /// # Returns
    ///
    /// A `Result` containing `PreMintSecrets` if successful, or an `Error`
    /// otherwise.
    ///
    /// # Errors
    ///
    /// This function will return an error if the creation of `PreMintSecrets`
    /// fails.
    ///
    /// ```
    pub fn generate_premint_secrets(
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

    /// Mint
    /// # Synopsis
    /// ```rust
    /// use std::sync::Arc;
    ///
    /// use anyhow::Result;
    /// use cdk::amount::{Amount, SplitTarget};
    /// use cdk::cdk_database::WalletMemoryDatabase;
    /// use cdk::nuts::CurrencyUnit;
    /// use cdk::wallet::Wallet;
    /// use rand::Rng;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let seed = rand::thread_rng().gen::<[u8; 32]>();
    ///     let mint_url = "https://testnut.cashu.space";
    ///     let unit = CurrencyUnit::Sat;
    ///
    ///     let localstore = WalletMemoryDatabase::default();
    ///     let wallet = Wallet::new(mint_url, unit, Arc::new(localstore), &seed, None).unwrap();
    ///     let amount = Amount::from(100);
    ///
    ///     let quote = wallet.mint_quote(amount, None).await?;
    ///     let quote_id = quote.id;
    ///     // To be called after quote request is paid
    ///     let amount_minted = wallet.mint(&quote_id, SplitTarget::default(), None).await?;
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
    ) -> Result<Amount, Error> {
        // Check that mint is in store of mints
        if self
            .localstore
            .get_mint(self.mint_url.clone())
            .await?
            .is_none()
        {
            self.get_mint_info().await?;
        }

        let quote_info = self.localstore.get_mint_quote(quote_id).await?;

        let quote_info = if let Some(quote) = quote_info {
            if quote.expiry.le(&unix_time()) && quote.expiry.ne(&0) {
                return Err(Error::ExpiredQuote(quote.expiry, unix_time()));
            }

            quote.clone()
        } else {
            return Err(Error::UnknownQuote);
        };

        let active_keyset_id = self.get_active_mint_keyset().await?.id;

        let count = self
            .localstore
            .get_keyset_counter(&active_keyset_id)
            .await?;

        let count = count.map_or(0, |c| c + 1);

        let premint_secrets = self.generate_premint_secrets(
            active_keyset_id,
            quote_info.amount,
            &amount_split_target,
            spending_conditions.as_ref(),
            count,
        )?;

        let request = MintBolt11Request {
            quote: quote_id.to_string(),
            outputs: premint_secrets.blinded_messages(),
        };

        let mint_res = self
            .client
            .post_mint(self.mint_url.clone(), request)
            .await?;

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

        let minted_amount = proofs.total_amount()?;

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

        let proofs = proofs
            .into_iter()
            .map(|proof| {
                ProofInfo::new(
                    proof,
                    self.mint_url.clone(),
                    State::Unspent,
                    quote_info.unit.clone(),
                )
            })
            .collect::<Result<Vec<ProofInfo>, _>>()?;

        // Add new proofs to store
        self.localstore.update_proofs(proofs, vec![]).await?;

        Ok(minted_amount)
    }

    pub async fn gen_ehash_premint_secrets(&self) -> Result<PreMintSecrets, Error> {
        let active_keyset_id = self.get_active_mint_keyset_local().await?.id;

        let count = self
            .localstore
            .get_keyset_counter(&active_keyset_id)
            .await?;

        let count = count.map_or(0, |c| c + 1);

        let premint_secrets = self.generate_premint_secrets(
            active_keyset_id,
            // TODO when do we want to set amount?
            Amount::from(1),
            &SplitTarget::None,
            None,
            count,
        )?;

        Ok(premint_secrets)
    }

    pub async fn gen_ehash_proofs(
        &self,
        sig: BlindSignature,
        premint: PreMint,
    ) -> Result<Vec<ProofInfo>, Error> {
        // TODO pass this in, it will break if the keyset changes before getting proofs
        let active_keyset_id = self.get_active_mint_keyset_local().await?.id;
        let keys = self.get_keyset_keys(active_keyset_id).await?;

        // Verify the signature DLEQ is valid
        {
            let keys = self.get_keyset_keys(sig.keyset_id).await?;
            let key = keys.amount_key(sig.amount).ok_or(Error::AmountKey)?;
            match sig.verify_dleq(key, premint.blinded_message.blinded_secret) {
                Ok(_) | Err(nut12::Error::MissingDleqProof) => (),
                Err(_) => return Err(Error::CouldNotVerifyDleq),
            }
        }

        let proofs = construct_proofs(vec![sig], vec![premint.r], vec![premint.secret], &keys)?;

        let minted_amount = proofs.total_amount()?;

        // Update counter for keyset
        self.localstore
            .increment_keyset_counter(&active_keyset_id, proofs.len() as u32)
            .await?;

        let proofs = proofs
            .into_iter()
            .map(|proof| {
                ProofInfo::new(
                    proof,
                    self.mint_url.clone(),
                    State::Unspent,
                    CurrencyUnit::Custom("HASH".to_string()),
                )
            })
            .collect::<Result<Vec<ProofInfo>, _>>()?;

        // Add new proofs to store
        self.localstore
            .update_proofs(proofs.clone(), vec![])
            .await?;

        // TODO return amount instead of proofs
        Ok(proofs)
    }
}
