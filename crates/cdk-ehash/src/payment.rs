//! Ehash mint payment processor.

use std::collections::HashMap;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bitcoin::hashes::sha256;
use cdk_common::payment::{
    CreateIncomingPaymentResponse, CustomIncomingPaymentOptions, Error, Event, IncomingPaymentOptions,
    MakePaymentResponse, MintPayment, OutgoingPaymentOptions, PaymentIdentifier, PaymentQuoteResponse,
    SettingsResponse, WaitPaymentResponse,
};
use cdk_common::nuts::CurrencyUnit;
use futures::stream;
use futures::Stream;
use serde_json::Value;

const EHASH_METHOD: &str = "ehash";

/// Ehash payment processor that treats validated shares as immediate payments.
#[derive(Debug, Clone)]
pub struct EhashPaymentProcessor {
    unit: CurrencyUnit,
    payments: Arc<Mutex<HashMap<String, WaitPaymentResponse>>>,
}

impl EhashPaymentProcessor {
    /// Create a new ehash payment processor for the given unit.
    pub fn new(unit: CurrencyUnit) -> Self {
        Self {
            unit,
            payments: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Convenience constructor for the default EHASH unit.
    pub fn default_unit() -> Self {
        Self::new(CurrencyUnit::custom("EHASH"))
    }

    fn parse_header_hash(extra_json: Option<&str>) -> Result<String, Error> {
        let Some(extra_json) = extra_json else {
            return Err(Error::Custom("missing header_hash".to_string()));
        };

        let extra: Value = serde_json::from_str(extra_json)?;
        let header_hash = extra
            .get("header_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Custom("missing header_hash".to_string()))?;

        sha256::Hash::from_str(header_hash).map_err(|_| Error::InvalidHash)?;

        Ok(header_hash.to_string())
    }
}

#[async_trait]
impl MintPayment for EhashPaymentProcessor {
    type Err = Error;

    async fn get_settings(&self) -> Result<SettingsResponse, Self::Err> {
        let mut custom = HashMap::new();
        custom.insert(EHASH_METHOD.to_string(), String::new());

        Ok(SettingsResponse {
            unit: self.unit.to_string(),
            bolt11: None,
            bolt12: None,
            custom,
        })
    }

    fn is_wait_invoice_active(&self) -> bool {
        false
    }

    fn cancel_wait_invoice(&self) {}

    async fn wait_payment_event(&self) -> Result<Pin<Box<dyn Stream<Item = Event> + Send>>, Self::Err> {
        Ok(Box::pin(stream::empty()))
    }

    async fn create_incoming_payment_request(
        &self,
        options: IncomingPaymentOptions,
    ) -> Result<CreateIncomingPaymentResponse, Self::Err> {
        let (amount, extra_json, unix_expiry, method) = match options {
            IncomingPaymentOptions::Custom(custom_options) => {
                let CustomIncomingPaymentOptions {
                    amount,
                    extra_json,
                    unix_expiry,
                    method,
                    ..
                } = *custom_options;
                (amount, extra_json, unix_expiry, method)
            }
            _ => return Err(Error::UnsupportedPaymentOption),
        };

        if method != EHASH_METHOD {
            return Err(Error::UnsupportedPaymentOption);
        }

        let header_hash = Self::parse_header_hash(extra_json.as_deref())?;

        let payment_identifier = PaymentIdentifier::CustomId(header_hash.clone());
        let payment = WaitPaymentResponse {
            payment_identifier: payment_identifier.clone(),
            payment_amount: amount,
            payment_id: header_hash.clone(),
        };

        let mut payments = self
            .payments
            .lock()
            .expect("ehash payments lock poisoned");
        payments.insert(header_hash.clone(), payment);

        Ok(CreateIncomingPaymentResponse {
            request_lookup_id: payment_identifier,
            request: header_hash,
            expiry: unix_expiry,
            extra_json: extra_json
                .as_deref()
                .map(|value| serde_json::from_str(value))
                .transpose()?,
        })
    }

    async fn get_payment_quote(
        &self,
        _unit: &CurrencyUnit,
        _options: OutgoingPaymentOptions,
    ) -> Result<PaymentQuoteResponse, Self::Err> {
        Err(Error::UnsupportedPaymentOption)
    }

    async fn make_payment(
        &self,
        _unit: &CurrencyUnit,
        _options: OutgoingPaymentOptions,
    ) -> Result<MakePaymentResponse, Self::Err> {
        Err(Error::UnsupportedPaymentOption)
    }

    async fn check_incoming_payment_status(
        &self,
        payment_identifier: &PaymentIdentifier,
    ) -> Result<Vec<WaitPaymentResponse>, Self::Err> {
        let PaymentIdentifier::CustomId(id) = payment_identifier else {
            return Ok(Vec::new());
        };

        let payments = self
            .payments
            .lock()
            .expect("ehash payments lock poisoned");
        Ok(payments.get(id).cloned().into_iter().collect())
    }

    async fn check_outgoing_payment(
        &self,
        _payment_identifier: &PaymentIdentifier,
    ) -> Result<MakePaymentResponse, Self::Err> {
        Err(Error::UnsupportedPaymentOption)
    }
}
