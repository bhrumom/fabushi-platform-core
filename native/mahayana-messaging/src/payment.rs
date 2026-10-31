use crate::actor::ActorId;
use crate::conversation::ConversationId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Money {
    pub currency: String,
    pub amount_minor: i64,
}

impl Money {
    pub fn new(currency: impl Into<String>, amount_minor: i64) -> Self {
        Self {
            currency: currency.into().to_uppercase(),
            amount_minor,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.currency.len() == 3
            && self.currency.bytes().all(|byte| byte.is_ascii_uppercase())
            && self.amount_minor >= 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceLine {
    pub label: String,
    pub amount: Money,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InvoiceKind {
    OneTime,
    Subscription,
    Donation,
    DigitalGoods,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invoice {
    pub id: String,
    pub conversation_id: ConversationId,
    pub seller_id: ActorId,
    pub title: String,
    pub description: String,
    pub kind: InvoiceKind,
    pub currency: String,
    pub prices: Vec<PriceLine>,
    pub payload: String,
    pub provider_id: String,
    pub start_parameter: Option<String>,
    pub request_name: bool,
    pub request_email: bool,
    pub request_phone: bool,
    pub request_shipping_address: bool,
    pub flexible_shipping: bool,
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
}

impl Invoice {
    pub fn checked_total_minor(&self) -> Option<i64> {
        self.prices.iter().try_fold(0i64, |total, line| {
            total.checked_add(line.amount.amount_minor)
        })
    }

    pub fn total_minor(&self) -> i64 {
        self.checked_total_minor().unwrap_or(i64::MAX)
    }

    pub fn is_valid(&self) -> bool {
        !self.id.trim().is_empty()
            && !self.title.trim().is_empty()
            && self.currency.len() == 3
            && !self.prices.is_empty()
            && self
                .prices
                .iter()
                .all(|line| line.amount.currency == self.currency && line.amount.is_valid())
            && self.checked_total_minor().is_some_and(|total| total > 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShippingAddress {
    pub country_code: String,
    pub state: String,
    pub city: String,
    pub street_line1: String,
    pub street_line2: Option<String>,
    pub postal_code: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerInfo {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub shipping_address: Option<ShippingAddress>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaymentStatus {
    Draft,
    Pending,
    RequiresAction,
    Authorized,
    Paid,
    Refunded,
    PartiallyRefunded,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentOrder {
    pub id: String,
    pub invoice_id: String,
    pub buyer_id: ActorId,
    pub status: PaymentStatus,
    pub amount: Money,
    pub customer: Option<CustomerInfo>,
    pub provider_payment_id: Option<String>,
    pub provider_receipt_url: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletBalance {
    pub asset: String,
    pub available_minor: i64,
    pub pending_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlement {
    pub id: String,
    pub owner_id: ActorId,
    pub product_id: String,
    pub order_id: String,
    pub starts_at_ms: i64,
    pub expires_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}
