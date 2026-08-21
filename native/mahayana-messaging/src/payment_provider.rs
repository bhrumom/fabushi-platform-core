use crate::actor::ActorId;
use crate::payment::{CustomerInfo, Invoice, Money, PaymentOrder, PaymentStatus};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutSession {
    pub id: String,
    pub provider_id: String,
    pub invoice_id: String,
    pub order_id: String,
    pub client_secret: Option<String>,
    pub redirect_url: Option<String>,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    pub provider_payment_id: String,
    pub status: PaymentStatus,
    pub receipt_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefundResult {
    pub provider_refund_id: String,
    pub refunded: Money,
    pub status: PaymentStatus,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PaymentProviderError {
    #[error("payment provider rejected the request: {0}")]
    Rejected(String),
    #[error("payment provider is temporarily unavailable: {0}")]
    Unavailable(String),
    #[error("payment provider requires customer action: {0}")]
    RequiresAction(String),
    #[error("payment provider response is invalid: {0}")]
    InvalidResponse(String),
}

pub trait PaymentProvider: Send + Sync {
    fn id(&self) -> &'static str;

    fn create_checkout(
        &self,
        invoice: &Invoice,
        order: &PaymentOrder,
        buyer_id: &ActorId,
        customer: Option<&CustomerInfo>,
    ) -> Result<CheckoutSession, PaymentProviderError>;

    fn capture(
        &self,
        session: &CheckoutSession,
        order: &PaymentOrder,
    ) -> Result<CaptureResult, PaymentProviderError>;

    fn refund(
        &self,
        order: &PaymentOrder,
        amount: Money,
        reason: Option<&str>,
    ) -> Result<RefundResult, PaymentProviderError>;

    fn query(&self, order: &PaymentOrder) -> Result<CaptureResult, PaymentProviderError>;
}
