use serde::{Deserialize, Serialize};

pub const PAYMENT_SCHEMA: &str = "mahayana.miniapp.payment.v1";

/// Untrusted request from a Mini App. Price, currency, merchant/developer,
/// platform fee and provider are intentionally absent: the trusted host must
/// resolve them from the server-side catalog for `mini_app_id + sku`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePaymentRequest {
    pub sku: String,
    pub idempotency_key: String,
}

impl CreatePaymentRequest {
    pub fn validate(&self) -> Result<(), PaymentContractError> {
        validate_id("sku", &self.sku)?;
        validate_id("idempotencyKey", &self.idempotency_key)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenCheckoutRequest {
    pub payment_id: String,
}

impl OpenCheckoutRequest {
    pub fn validate(&self) -> Result<(), PaymentContractError> {
        validate_id("paymentId", &self.payment_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentStatusRequest {
    pub payment_id: String,
}

impl PaymentStatusRequest {
    pub fn validate(&self) -> Result<(), PaymentContractError> {
        validate_id("paymentId", &self.payment_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckoutStatus {
    Created,
    RequiresAction,
    Processing,
    Succeeded,
    Failed,
    Cancelled,
    PartiallyRefunded,
    Refunded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentView {
    pub schema: String,
    pub payment_id: String,
    pub sku: String,
    pub display_amount: String,
    pub currency: String,
    pub status: CheckoutStatus,
}

impl PaymentView {
    pub fn validate(&self) -> Result<(), PaymentContractError> {
        if self.schema != PAYMENT_SCHEMA {
            return Err(PaymentContractError::UnsupportedSchema(self.schema.clone()));
        }
        validate_id("paymentId", &self.payment_id)?;
        validate_id("sku", &self.sku)?;
        if self.display_amount.trim().is_empty() || self.currency.trim().is_empty() {
            return Err(PaymentContractError::MissingField("amount/currency"));
        }
        Ok(())
    }
}

fn validate_id(field: &'static str, value: &str) -> Result<(), PaymentContractError> {
    if value.trim().is_empty() || value.len() > 256 {
        return Err(PaymentContractError::InvalidIdentifier(field));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaymentContractError {
    #[error("invalid payment identifier: {0}")]
    InvalidIdentifier(&'static str),
    #[error("unsupported Mini App payment schema: {0}")]
    UnsupportedSchema(String),
    #[error("Mini App payment response is missing {0}")]
    MissingField(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_request_contains_no_client_controlled_price() {
        let json = serde_json::json!({
            "sku": "ai.pro.month",
            "idempotencyKey": "checkout-1"
        });
        let request: CreatePaymentRequest = serde_json::from_value(json).unwrap();
        request.validate().unwrap();
    }

    #[test]
    fn injected_price_is_rejected_by_deny_unknown_fields() {
        let json = serde_json::json!({
            "sku": "ai.pro.month",
            "idempotencyKey": "checkout-1",
            "amount": 1,
            "currency": "FBC"
        });
        assert!(serde_json::from_value::<CreatePaymentRequest>(json).is_err());
    }
}
