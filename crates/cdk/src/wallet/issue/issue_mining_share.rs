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
            blinded_messages: premint_secrets.blinded_messages().to_vec(),
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
}
