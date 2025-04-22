use cdk_common::nut04::{MintMiningShareRequest, MintQuoteMiningShareRequest, MintQuoteMiningShareResponse};
use cdk_common::BlindedMessage;
use tracing::{instrument, debug};
use uuid::Uuid;

use super::verification::Verification;
use super::{
    nut04, CurrencyUnit, Mint, MintQuote, MintQuoteBolt11Request, MintQuoteBolt11Response,
    NotificationPayload, PaymentMethod, PublicKey,
};
use crate::nuts::MintQuoteState;
use crate::types::LnKey;
use crate::util::unix_time;
use crate::{Amount, Error};

impl Mint {
    /// Checks that minting is enabled, request is supported unit and within range
    async fn check_mint_request_acceptable(
        &self,
        amount: Amount,
        unit: &CurrencyUnit,
    ) -> Result<(), Error> {
        let mint_info = self.localstore.get_mint_info().await?;
        let nut04 = &mint_info.nuts.nut04;

        if nut04.disabled {
            return Err(Error::MintingDisabled);
        }

        match nut04.get_settings(unit, &PaymentMethod::Bolt11) {
            Some(settings) => {
                if settings
                    .max_amount
                    .map_or(false, |max_amount| amount > max_amount)
                {
                    return Err(Error::AmountOutofLimitRange(
                        settings.min_amount.unwrap_or_default(),
                        settings.max_amount.unwrap_or_default(),
                        amount,
                    ));
                }

                if settings
                    .min_amount
                    .map_or(false, |min_amount| amount < min_amount)
                {
                    return Err(Error::AmountOutofLimitRange(
                        settings.min_amount.unwrap_or_default(),
                        settings.max_amount.unwrap_or_default(),
                        amount,
                    ));
                }
            }
            None => {
                return Err(Error::UnsupportedUnit);
            }
        }

        Ok(())
    }

    /// Create new mint bolt11 quote
    #[instrument(skip_all)]
    pub async fn get_mint_bolt11_quote(
        &self,
        mint_quote_request: MintQuoteBolt11Request,
    ) -> Result<MintQuoteBolt11Response<Uuid>, Error> {
        let MintQuoteBolt11Request {
            amount,
            unit,
            description,
            pubkey,
        } = mint_quote_request;

        self.check_mint_request_acceptable(amount, &unit).await?;

        let ln = self
            .ln
            .get(&LnKey::new(unit.clone(), PaymentMethod::Bolt11))
            .ok_or_else(|| {
                tracing::info!("Bolt11 mint request for unsupported unit");

                Error::UnsupportedUnit
            })?;

        let mint_ttl = self.localstore.get_quote_ttl().await?.mint_ttl;

        let quote_expiry = unix_time() + mint_ttl;

        if description.is_some() && !ln.get_settings().invoice_description {
            tracing::error!("Backend does not support invoice description");
            return Err(Error::InvoiceDescriptionUnsupported);
        }

        let create_invoice_response = ln
            .create_invoice(
                amount,
                &unit,
                description.unwrap_or("".to_string()),
                quote_expiry,
            )
            .await
            .map_err(|err| {
                tracing::error!("Could not create invoice: {}", err);
                Error::InvalidPaymentRequest
            })?;

        let quote = MintQuote::new(
            create_invoice_response.request.to_string(),
            unit.clone(),
            amount,
            create_invoice_response.expiry.unwrap_or(0),
            create_invoice_response.request_lookup_id.clone(),
            pubkey,
        );

        tracing::debug!(
            "New mint quote {} for {} {} with request id {}",
            quote.id,
            amount,
            unit,
            create_invoice_response.request_lookup_id,
        );

        self.localstore.add_mint_quote(quote.clone()).await?;

        let quote: MintQuoteBolt11Response<Uuid> = quote.into();

        self.pubsub_manager
            .broadcast(NotificationPayload::MintQuoteBolt11Response(quote.clone()));

        Ok(quote)
    }

    /// Create new mint mining share quote
    #[instrument(skip_all)]
    pub async fn create_paid_mint_mining_share_quote(
        &self,
        mint_quote_request: MintQuoteMiningShareRequest,
        blinded_messages: Vec<BlindedMessage>,
    ) -> Result<MintQuoteMiningShareResponse<Uuid>, Error> {
        let MintQuoteMiningShareRequest {
            amount,
            unit,
            header_hash,
            description,
            pubkey,
        } = mint_quote_request;

        // TODO fix this function, it is hard coded to bolt11
        // self.check_mint_request_acceptable(amount, &unit).await?;

        let mint_ttl = self.localstore.get_quote_ttl().await?.mint_ttl;

        let quote_expiry = unix_time() + mint_ttl;

        let quote = MintQuote::new(
            header_hash.to_string(),
            unit.clone(),
            amount,
            quote_expiry,
            // TODO is there a better request lookup ID?
            header_hash.to_string(),
            pubkey,
        );

        tracing::debug!(
            "New mint quote {} for {} {} with request id {}",
            quote.id,
            amount,
            unit,
            header_hash,
        );

        self.localstore.add_mint_quote(quote.clone()).await?;
        // TODO implement block template validation and make this a separate API call
        self.pay_mint_quote(&quote).await?;

        let mint_mining_share_request = MintMiningShareRequest { 
            quote: quote.id,
            outputs: blinded_messages,
            signature: None,
        };

        self.process_mining_mint_request(mint_mining_share_request).await?;
        debug!("Successfully processed mining mint request for quote {}", quote.id);
      
        // Update the local quote state to Issued since was updated in process_mining_mint_request
        let mut quote: MintQuoteMiningShareResponse<Uuid> = quote.into();
        quote.state = MintQuoteState::Issued;

        self.pubsub_manager
            .broadcast(NotificationPayload::MintQuoteMiningShareResponse(quote.clone()));

        Ok(quote)
    }

    /// Check mint quote
    #[instrument(skip(self))]
    pub async fn check_mint_quote(
        &self,
        quote_id: &Uuid,
    ) -> Result<MintQuoteBolt11Response<Uuid>, Error> {
        let quote = self
            .localstore
            .get_mint_quote(quote_id)
            .await?
            .ok_or(Error::UnknownQuote)?;

        // Since the pending state is not part of the NUT it should not be part of the
        // response. In practice the wallet should not be checking the state of
        // a quote while waiting for the mint response.
        let state = match quote.state {
            MintQuoteState::Pending => MintQuoteState::Paid,
            MintQuoteState::Unpaid => self.check_mint_quote_paid(quote_id).await?,
            s => s,
        };

        Ok(MintQuoteBolt11Response {
            quote: quote.id,
            request: quote.request,
            state,
            expiry: Some(quote.expiry),
            pubkey: quote.pubkey,
        })
    }

    /// Update mint quote
    #[instrument(skip_all)]
    pub async fn update_mint_quote(&self, quote: MintQuote) -> Result<(), Error> {
        self.localstore.add_mint_quote(quote).await?;
        Ok(())
    }

    /// Get mint quotes
    #[instrument(skip_all)]
    pub async fn mint_quotes(&self) -> Result<Vec<MintQuote>, Error> {
        let quotes = self.localstore.get_mint_quotes().await?;
        Ok(quotes)
    }

    /// Get pending mint quotes
    #[instrument(skip_all)]
    pub async fn get_pending_mint_quotes(&self) -> Result<Vec<MintQuote>, Error> {
        let mint_quotes = self
            .localstore
            .get_mint_quotes_with_state(MintQuoteState::Pending)
            .await?;

        Ok(mint_quotes)
    }

    /// Get pending mint quotes
    #[instrument(skip_all)]
    pub async fn get_unpaid_mint_quotes(&self) -> Result<Vec<MintQuote>, Error> {
        let mint_quotes = self
            .localstore
            .get_mint_quotes_with_state(MintQuoteState::Unpaid)
            .await?;

        Ok(mint_quotes)
    }

    /// Remove mint quote
    #[instrument(skip_all)]
    pub async fn remove_mint_quote(&self, quote_id: &Uuid) -> Result<(), Error> {
        self.localstore.remove_mint_quote(quote_id).await?;

        Ok(())
    }

    /// Flag mint quote as paid
    #[instrument(skip_all)]
    pub async fn pay_mint_quote_for_request_id(
        &self,
        request_lookup_id: &str,
    ) -> Result<(), Error> {
        if let Ok(Some(mint_quote)) = self
            .localstore
            .get_mint_quote_by_request_lookup_id(request_lookup_id)
            .await
        {
            self.pay_mint_quote(&mint_quote).await?;
        }
        Ok(())
    }

    /// Mark mint quote as paid
    #[instrument(skip_all)]
    pub async fn pay_mint_quote(&self, mint_quote: &MintQuote) -> Result<(), Error> {
        tracing::debug!(
            "Received payment notification for mint quote {}",
            mint_quote.id
        );
        if mint_quote.state != MintQuoteState::Issued && mint_quote.state != MintQuoteState::Paid {
            let unix_time = unix_time();

            if mint_quote.expiry < unix_time {
                tracing::warn!(
                    "Mint quote {} paid at {} expired at {}, leaving current state",
                    mint_quote.id,
                    mint_quote.expiry,
                    unix_time,
                );
                return Err(Error::ExpiredQuote(mint_quote.expiry, unix_time));
            }

            self.localstore
                .update_mint_quote_state(&mint_quote.id, MintQuoteState::Paid)
                .await?;
        } else {
            tracing::debug!(
                "{} Quote already {} continuing",
                mint_quote.id,
                mint_quote.state
            );
        }

        self.pubsub_manager
            .mint_quote_bolt11_status(mint_quote.clone(), MintQuoteState::Paid);

        Ok(())
    }

    // TODO this is a cheap copy, DRY this function and the next one
    /// Process mining mint request
    #[instrument(skip_all)]
    pub async fn process_mining_mint_request(
        &self,
        mint_request: nut04::MintMiningShareRequest<Uuid>,
    ) -> Result<nut04::MintMiningShareResponse, Error> {
        let mint_quote =
            if let Some(mint_quote) = self.localstore.get_mint_quote(&mint_request.quote).await? {
                mint_quote
            } else {
                return Err(Error::UnknownQuote);
            };

        let state = self
            .localstore
            .update_mint_quote_state(&mint_request.quote, MintQuoteState::Pending)
            .await?;

        let state = if state == MintQuoteState::Unpaid {
            self.check_mint_quote_paid(&mint_quote.id).await?
        } else {
            state
        };

        match state {
            MintQuoteState::Unpaid => {
                let _state = self
                    .localstore
                    .update_mint_quote_state(&mint_request.quote, MintQuoteState::Unpaid)
                    .await?;
                return Err(Error::UnpaidQuote);
            }
            MintQuoteState::Pending => {
                return Err(Error::PendingQuote);
            }
            MintQuoteState::Issued => {
                let _state = self
                    .localstore
                    .update_mint_quote_state(&mint_request.quote, MintQuoteState::Issued)
                    .await?;
                return Err(Error::IssuedQuote);
            }
            MintQuoteState::Paid => (),
        }

        // If the there is a public key provoided in mint quote request
        // verify the signature is provided for the mint request
        if let Some(pubkey) = mint_quote.pubkey {
            mint_request.verify_signature(pubkey)?;
        }

        let Verification { amount, unit } = match self.verify_outputs(&mint_request.outputs).await {
            Ok(verification) => verification,
            Err(err) => {
                tracing::debug!("Could not verify mint outputs");
                self.localstore
                    .update_mint_quote_state(&mint_request.quote, MintQuoteState::Paid)
                    .await?;

                return Err(err);
            }
        };

        // We check the the total value of blinded messages == mint quote
        if amount != mint_quote.amount {
            return Err(Error::TransactionUnbalanced(
                mint_quote.amount.into(),
                mint_request.total_amount()?.into(),
                0,
            ));
        }

        if unit != mint_quote.unit {
            return Err(Error::UnsupportedUnit);
        }

        let mut blind_signatures = Vec::with_capacity(mint_request.outputs.len());

        for blinded_message in mint_request.outputs.iter() {
            let blind_signature = self.blind_sign(blinded_message).await?;
            blind_signatures.push(blind_signature);
        }

        self.localstore
            .add_blind_signatures(
                &mint_request
                    .outputs
                    .iter()
                    .map(|p| p.blinded_secret)
                    .collect::<Vec<PublicKey>>(),
                &blind_signatures,
                Some(mint_request.quote),
            )
            .await?;

        self.localstore
            .update_mint_quote_state(&mint_request.quote, MintQuoteState::Issued)
            .await?;

        self.pubsub_manager
            .mint_mining_quote_status(mint_quote, MintQuoteState::Issued);

        Ok(nut04::MintMiningShareResponse {
            signatures: blind_signatures,
        })
    }

    /// Process mint request
    #[instrument(skip_all)]
    pub async fn process_mint_request(
        &self,
        mint_request: nut04::MintBolt11Request<Uuid>,
    ) -> Result<nut04::MintBolt11Response, Error> {
        let mint_quote =
            if let Some(mint_quote) = self.localstore.get_mint_quote(&mint_request.quote).await? {
                mint_quote
            } else {
                return Err(Error::UnknownQuote);
            };

        let state = self
            .localstore
            .update_mint_quote_state(&mint_request.quote, MintQuoteState::Pending)
            .await?;

        let state = if state == MintQuoteState::Unpaid {
            self.check_mint_quote_paid(&mint_quote.id).await?
        } else {
            state
        };

        match state {
            MintQuoteState::Unpaid => {
                let _state = self
                    .localstore
                    .update_mint_quote_state(&mint_request.quote, MintQuoteState::Unpaid)
                    .await?;
                return Err(Error::UnpaidQuote);
            }
            MintQuoteState::Pending => {
                return Err(Error::PendingQuote);
            }
            MintQuoteState::Issued => {
                let _state = self
                    .localstore
                    .update_mint_quote_state(&mint_request.quote, MintQuoteState::Issued)
                    .await?;
                return Err(Error::IssuedQuote);
            }
            MintQuoteState::Paid => (),
        }

        // If the there is a public key provoided in mint quote request
        // verify the signature is provided for the mint request
        if let Some(pubkey) = mint_quote.pubkey {
            mint_request.verify_signature(pubkey)?;
        }

        let Verification { amount, unit } = match self.verify_outputs(&mint_request.outputs).await {
            Ok(verification) => verification,
            Err(err) => {
                tracing::debug!("Could not verify mint outputs");
                self.localstore
                    .update_mint_quote_state(&mint_request.quote, MintQuoteState::Paid)
                    .await?;

                return Err(err);
            }
        };

        // We check the the total value of blinded messages == mint quote
        if amount != mint_quote.amount {
            return Err(Error::TransactionUnbalanced(
                mint_quote.amount.into(),
                mint_request.total_amount()?.into(),
                0,
            ));
        }

        if unit != mint_quote.unit {
            return Err(Error::UnsupportedUnit);
        }

        let mut blind_signatures = Vec::with_capacity(mint_request.outputs.len());

        for blinded_message in mint_request.outputs.iter() {
            let blind_signature = self.blind_sign(blinded_message).await?;
            blind_signatures.push(blind_signature);
        }

        self.localstore
            .add_blind_signatures(
                &mint_request
                    .outputs
                    .iter()
                    .map(|p| p.blinded_secret)
                    .collect::<Vec<PublicKey>>(),
                &blind_signatures,
                Some(mint_request.quote),
            )
            .await?;

        self.localstore
            .update_mint_quote_state(&mint_request.quote, MintQuoteState::Issued)
            .await?;

        self.pubsub_manager
            .mint_quote_bolt11_status(mint_quote, MintQuoteState::Issued);

        Ok(nut04::MintBolt11Response {
            signatures: blind_signatures,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::amount::Amount;
    use crate::nuts::{CurrencyUnit, MintQuoteState};
    use cdk_common::nut04::MintQuoteMiningShareRequest;
    use cdk_integration_tests::init_pure_tests::create_and_start_test_mint;
    use bitcoin_hashes::sha256::Hash;
    use cdk_common::nut00::BlindedMessage;
    use std::collections::HashMap;
    use anyhow::Result;

    #[tokio::test]
    async fn test_mint_mining_share_quote_is_issued() -> Result<()> {
        let mint = create_and_start_test_mint().await?;

        // Rotate keyset to make it active for CurrencyUnit::Sat
        mint.rotate_keyset(CurrencyUnit::Sat, 0, 32, 0, &HashMap::new()).await?;

        // Create a header hash for the mining share request
        let header_hash = Hash::hash(b"test_header");

        // Get  active keyset ID, keyset info to get a valid public key
        let active_keyset_id = mint.localstore.get_active_keyset_id(&CurrencyUnit::Sat).await?.unwrap();
        let keyset_info = mint.localstore.get_keyset_info(&active_keyset_id).await?.unwrap();
        let keyset = mint.generate_keyset(keyset_info);
        
        // Get the public key for amount 512 (2^9)
        let amount = Amount::from(512);
        let valid_public_key = keyset.keys.get(&amount).unwrap().public_key;

        let request = MintQuoteMiningShareRequest {
            amount: Amount::from(512),
            unit: CurrencyUnit::Sat,
            header_hash,
            description: None,
            pubkey: None,
        };

        // Create a minimal blinded message with the same unit
        let blinded_message = BlindedMessage {
            amount: Amount::from(512),
            keyset_id: active_keyset_id,
            blinded_secret: valid_public_key,
            witness: None,
        };

        // Create the quote with the blinded message
        let quote = mint.create_paid_mint_mining_share_quote(request, vec![blinded_message]).await.unwrap();

        // Verify quote state is Issued
        assert_eq!(quote.state, MintQuoteState::Issued);

        Ok(())
    }
}

