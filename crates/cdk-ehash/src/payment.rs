use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cdk_common::payment::{
    CreateIncomingPaymentResponse, Error as PaymentError, Event, IncomingPaymentOptions,
    MakePaymentResponse, MintPayment, OutgoingPaymentOptions, PaymentIdentifier,
    PaymentQuoteResponse, SettingsResponse, WaitPaymentResponse,
};
use cdk_common::{Amount, CurrencyUnit};
use futures::Stream;
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt as _;
use tracing::instrument;

use crate::error::EhashError;

/// Validates that a string is a 64-character lowercase hex string (32 bytes).
fn is_valid_header_hash(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Payment processor for the ehash custom payment method.
///
/// Ehash quotes are backed by mining shares: a miner submits a valid
/// proof-of-work share (identified by its `header_hash`) and, once the
/// Hashpool validates it, the quote is marked paid and tokens can be minted.
///
/// The processor exposes [`EhashPaymentProcessor::pay_ehash_quote`] so that
/// Hashpool's validation logic can trigger payment events from outside.
#[derive(Debug)]
pub struct EhashPaymentProcessor {
    unit: CurrencyUnit,
    sender: mpsc::Sender<WaitPaymentResponse>,
    receiver: Mutex<Option<mpsc::Receiver<WaitPaymentResponse>>>,
    wait_invoice_is_active: Arc<AtomicBool>,
    cancel_token: tokio_util::sync::CancellationToken,
}

impl EhashPaymentProcessor {
    /// Create a new processor for the given currency unit.
    pub fn new(unit: CurrencyUnit) -> Self {
        let (sender, receiver) = mpsc::channel(256);
        Self {
            unit,
            sender,
            receiver: Mutex::new(Some(receiver)),
            wait_invoice_is_active: Arc::new(AtomicBool::new(false)),
            cancel_token: tokio_util::sync::CancellationToken::new(),
        }
    }

    /// Mark an ehash quote as paid by emitting a payment event.
    ///
    /// `header_hash` must match the `request_lookup_id` stored on the quote.
    /// `amount` is the amount credited to the miner for this share.
    pub async fn pay_ehash_quote(
        &self,
        header_hash: &str,
        amount: Amount<CurrencyUnit>,
    ) -> Result<(), EhashError> {
        let event = WaitPaymentResponse {
            payment_identifier: PaymentIdentifier::CustomId(header_hash.to_string()),
            payment_amount: amount,
            payment_id: header_hash.to_string(),
        };
        self.sender
            .send(event)
            .await
            .map_err(|_| EhashError::NoReceiver)
    }
}

#[async_trait]
impl MintPayment for EhashPaymentProcessor {
    type Err = PaymentError;

    async fn get_settings(&self) -> Result<SettingsResponse, Self::Err> {
        let mut custom = std::collections::HashMap::new();
        custom.insert("ehash".to_string(), String::new());
        Ok(SettingsResponse {
            unit: self.unit.to_string(),
            bolt11: None,
            bolt12: None,
            custom,
        })
    }

    fn is_wait_invoice_active(&self) -> bool {
        self.wait_invoice_is_active.load(Ordering::SeqCst)
    }

    fn cancel_wait_invoice(&self) {
        self.cancel_token.cancel();
    }

    #[instrument(skip_all)]
    async fn wait_payment_event(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = Event> + Send>>, Self::Err> {
        let receiver = self
            .receiver
            .lock()
            .await
            .take()
            .ok_or(PaymentError::from(EhashError::NoReceiver))?;
        self.wait_invoice_is_active.store(true, Ordering::SeqCst);
        let stream = ReceiverStream::new(receiver).map(|resp| Event::PaymentReceived(resp));
        Ok(Box::pin(stream))
    }

    #[instrument(skip_all)]
    async fn create_incoming_payment_request(
        &self,
        options: IncomingPaymentOptions,
    ) -> Result<CreateIncomingPaymentResponse, Self::Err> {
        let custom = match options {
            IncomingPaymentOptions::Custom(c) => c,
            _ => return Err(PaymentError::from(EhashError::WrongPaymentOptions)),
        };

        // Parse extra_json to extract header_hash
        let extra_str = custom
            .extra_json
            .as_deref()
            .ok_or_else(|| PaymentError::from(EhashError::MissingHeaderHash))?;
        let extra: Value = serde_json::from_str(extra_str)
            .map_err(|_| PaymentError::from(EhashError::InvalidExtraJson))?;
        let header_hash = extra["header_hash"]
            .as_str()
            .ok_or_else(|| PaymentError::from(EhashError::MissingHeaderHash))?;

        if !is_valid_header_hash(header_hash) {
            return Err(PaymentError::from(EhashError::InvalidHeaderHash));
        }

        Ok(CreateIncomingPaymentResponse {
            request_lookup_id: PaymentIdentifier::CustomId(header_hash.to_string()),
            request: header_hash.to_string(),
            expiry: custom.unix_expiry,
            extra_json: None,
        })
    }

    async fn get_payment_quote(
        &self,
        _unit: &CurrencyUnit,
        _options: OutgoingPaymentOptions,
    ) -> Result<PaymentQuoteResponse, Self::Err> {
        Err(PaymentError::from(EhashError::OutgoingNotSupported))
    }

    async fn make_payment(
        &self,
        _unit: &CurrencyUnit,
        _options: OutgoingPaymentOptions,
    ) -> Result<MakePaymentResponse, Self::Err> {
        Err(PaymentError::from(EhashError::OutgoingNotSupported))
    }

    async fn check_incoming_payment_status(
        &self,
        _payment_identifier: &PaymentIdentifier,
    ) -> Result<Vec<WaitPaymentResponse>, Self::Err> {
        // Ehash shares are validated externally and pushed via pay_ehash_quote().
        // There is no way to query historical payment status from this processor alone.
        Ok(vec![])
    }

    async fn check_outgoing_payment(
        &self,
        _payment_identifier: &PaymentIdentifier,
    ) -> Result<MakePaymentResponse, Self::Err> {
        Err(PaymentError::from(EhashError::OutgoingNotSupported))
    }
}
