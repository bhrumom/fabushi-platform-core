use crate::{
    CreatePaymentIntent, Money, PaymentCurrency, PaymentIntent, PaymentLedgerEntry, PaymentRail,
    ProductKind,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustedCatalogProduct {
    pub mini_app_id: String,
    pub developer_id: String,
    pub sku: String,
    pub product_kind: ProductKind,
    pub price: Money,
    pub platform_fee_bps: u16,
    pub allowed_rails: Vec<PaymentRail>,
    pub active: bool,
}

impl TrustedCatalogProduct {
    pub fn validate(&self) -> Result<(), PaymentInfrastructureError> {
        if self.mini_app_id.trim().is_empty()
            || self.developer_id.trim().is_empty()
            || self.sku.trim().is_empty()
        {
            return Err(PaymentInfrastructureError::InvalidCatalogProduct);
        }
        if self.price.amount <= 0 || self.platform_fee_bps > 10_000 {
            return Err(PaymentInfrastructureError::InvalidCatalogProduct);
        }
        if self.allowed_rails.is_empty() {
            return Err(PaymentInfrastructureError::NoPaymentRail);
        }
        Ok(())
    }
}

pub trait PaymentCatalog {
    fn resolve_product(
        &self,
        mini_app_id: &str,
        sku: &str,
    ) -> Result<TrustedCatalogProduct, PaymentInfrastructureError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MiniAppCheckoutContext {
    pub idempotency_key: String,
    pub user_id: String,
    pub mini_app_id: String,
    pub sku: String,
    pub requested_rail: PaymentRail,
    pub created_at_ms: i64,
}

pub fn resolve_trusted_payment_intent<C: PaymentCatalog>(
    catalog: &C,
    checkout: &MiniAppCheckoutContext,
) -> Result<CreatePaymentIntent, PaymentInfrastructureError> {
    if checkout.idempotency_key.trim().is_empty()
        || checkout.user_id.trim().is_empty()
        || checkout.mini_app_id.trim().is_empty()
        || checkout.sku.trim().is_empty()
    {
        return Err(PaymentInfrastructureError::InvalidCheckoutContext);
    }

    let product = catalog.resolve_product(&checkout.mini_app_id, &checkout.sku)?;
    product.validate()?;
    if product.mini_app_id != checkout.mini_app_id || product.sku != checkout.sku {
        return Err(PaymentInfrastructureError::CatalogIdentityMismatch);
    }
    if !product.active {
        return Err(PaymentInfrastructureError::ProductInactive);
    }
    if !product.allowed_rails.contains(&checkout.requested_rail) {
        return Err(PaymentInfrastructureError::RailNotAllowed(
            checkout.requested_rail,
        ));
    }

    Ok(CreatePaymentIntent {
        idempotency_key: checkout.idempotency_key.clone(),
        user_id: checkout.user_id.clone(),
        mini_app_id: checkout.mini_app_id.clone(),
        developer_id: product.developer_id,
        sku: product.sku,
        product_kind: product.product_kind,
        rail: checkout.requested_rail,
        amount: product.price,
        platform_fee_bps: product.platform_fee_bps,
        created_at_ms: checkout.created_at_ms,
    })
}

/// Persistence contract for a production implementation. Implementations must
/// commit a payment mutation and its ledger entry atomically.
pub trait PaymentRepository {
    fn load_payment(
        &self,
        payment_id: &str,
    ) -> Result<Option<PaymentIntent>, PaymentInfrastructureError>;

    fn commit_payment_and_ledger(
        &mut self,
        payment: &PaymentIntent,
        ledger_entry: Option<&PaymentLedgerEntry>,
    ) -> Result<(), PaymentInfrastructureError>;

    fn claim_provider_event(
        &mut self,
        provider: &str,
        event_id: &str,
    ) -> Result<bool, PaymentInfrastructureError>;
}

/// Credits are a platform liability, not a developer payout balance. A
/// production implementation must debit credits atomically with the payment
/// transition and reject insufficient balances.
pub trait CreditsLedger {
    fn balance(
        &self,
        user_id: &str,
        currency: &PaymentCurrency,
    ) -> Result<i64, PaymentInfrastructureError>;

    fn debit_once(
        &mut self,
        user_id: &str,
        payment_id: &str,
        amount: &Money,
    ) -> Result<(), PaymentInfrastructureError>;

    fn credit_once(
        &mut self,
        user_id: &str,
        reference_id: &str,
        amount: &Money,
    ) -> Result<(), PaymentInfrastructureError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedProviderCallback {
    pub provider: String,
    pub event_id: String,
    pub received_at_ms: i64,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum VerifiedProviderEvent {
    PaymentSucceeded {
        payment_id: String,
        provider_reference: String,
        occurred_at_ms: i64,
    },
    PaymentFailed {
        payment_id: String,
        provider_reference: String,
        occurred_at_ms: i64,
    },
    PaymentCancelled {
        payment_id: String,
        provider_reference: String,
        occurred_at_ms: i64,
    },
    RefundSucceeded {
        payment_id: String,
        provider_reference: String,
        refund_reference: String,
        amount: i64,
        occurred_at_ms: i64,
    },
    ChargebackOpened {
        payment_id: String,
        provider_reference: String,
        dispute_reference: String,
        amount: i64,
        occurred_at_ms: i64,
    },
}

/// Provider adapters own signature verification and provider-specific parsing.
/// The core never accepts an unverified webhook as a payment outcome.
pub trait PaymentProviderAdapter {
    fn provider_name(&self) -> &'static str;

    fn verify_callback(
        &self,
        callback: &SignedProviderCallback,
    ) -> Result<VerifiedProviderEvent, PaymentInfrastructureError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PayoutStatus {
    Created,
    RequiresIdentity,
    Pending,
    Processing,
    Paid,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeveloperPayout {
    pub payout_id: String,
    pub idempotency_key: String,
    pub developer_id: String,
    pub amount: Money,
    pub status: PayoutStatus,
    pub payout_account_id: String,
    pub provider_reference: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl DeveloperPayout {
    pub fn validate(&self) -> Result<(), PaymentInfrastructureError> {
        if self.payout_id.trim().is_empty()
            || self.idempotency_key.trim().is_empty()
            || self.developer_id.trim().is_empty()
            || self.payout_account_id.trim().is_empty()
            || self.amount.amount <= 0
        {
            return Err(PaymentInfrastructureError::InvalidPayout);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettlementPolicy {
    pub hold_period_seconds: u64,
    pub reserve_bps: u16,
}

impl SettlementPolicy {
    pub fn validate(self) -> Result<(), PaymentInfrastructureError> {
        if self.reserve_bps > 10_000 {
            return Err(PaymentInfrastructureError::InvalidSettlementPolicy);
        }
        Ok(())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PaymentInfrastructureError {
    #[error("invalid trusted catalog product")]
    InvalidCatalogProduct,
    #[error("trusted catalog product has no allowed payment rail")]
    NoPaymentRail,
    #[error("invalid Mini App checkout context")]
    InvalidCheckoutContext,
    #[error("catalog product identity does not match the requesting Mini App")]
    CatalogIdentityMismatch,
    #[error("catalog product is inactive")]
    ProductInactive,
    #[error("requested payment rail is not allowed: {0:?}")]
    RailNotAllowed(PaymentRail),
    #[error("payment product was not found")]
    ProductNotFound,
    #[error("payment repository failed: {0}")]
    Repository(String),
    #[error("credits balance is insufficient")]
    InsufficientCredits,
    #[error("provider callback signature or payload is invalid")]
    InvalidProviderCallback,
    #[error("provider operation failed: {0}")]
    Provider(String),
    #[error("developer payout is invalid")]
    InvalidPayout,
    #[error("settlement policy is invalid")]
    InvalidSettlementPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StaticCatalog {
        product: TrustedCatalogProduct,
    }

    impl PaymentCatalog for StaticCatalog {
        fn resolve_product(
            &self,
            mini_app_id: &str,
            sku: &str,
        ) -> Result<TrustedCatalogProduct, PaymentInfrastructureError> {
            if self.product.mini_app_id == mini_app_id && self.product.sku == sku {
                Ok(self.product.clone())
            } else {
                Err(PaymentInfrastructureError::ProductNotFound)
            }
        }
    }

    fn catalog(active: bool) -> StaticCatalog {
        StaticCatalog {
            product: TrustedCatalogProduct {
                mini_app_id: "miniapp-1".into(),
                developer_id: "developer-1".into(),
                sku: "pro.month".into(),
                product_kind: ProductKind::Subscription,
                price: Money {
                    currency: PaymentCurrency::credits(),
                    amount: 1_000,
                },
                platform_fee_bps: 1_500,
                allowed_rails: vec![PaymentRail::Credits],
                active,
            },
        }
    }

    fn checkout() -> MiniAppCheckoutContext {
        MiniAppCheckoutContext {
            idempotency_key: "checkout-1".into(),
            user_id: "user-1".into(),
            mini_app_id: "miniapp-1".into(),
            sku: "pro.month".into(),
            requested_rail: PaymentRail::Credits,
            created_at_ms: 100,
        }
    }

    #[test]
    fn catalog_is_the_only_source_of_price_and_merchant_identity() {
        let intent = resolve_trusted_payment_intent(&catalog(true), &checkout()).unwrap();
        assert_eq!(intent.amount.amount, 1_000);
        assert_eq!(intent.developer_id, "developer-1");
        assert_eq!(intent.platform_fee_bps, 1_500);
    }

    #[test]
    fn inactive_product_and_unapproved_rail_are_rejected() {
        assert_eq!(
            resolve_trusted_payment_intent(&catalog(false), &checkout()).unwrap_err(),
            PaymentInfrastructureError::ProductInactive
        );

        let mut request = checkout();
        request.requested_rail = PaymentRail::WebProvider;
        assert_eq!(
            resolve_trusted_payment_intent(&catalog(true), &request).unwrap_err(),
            PaymentInfrastructureError::RailNotAllowed(PaymentRail::WebProvider)
        );
    }

    #[test]
    fn payout_and_settlement_policy_are_strictly_validated() {
        let payout = DeveloperPayout {
            payout_id: "payout-1".into(),
            idempotency_key: "payout-key-1".into(),
            developer_id: "developer-1".into(),
            amount: Money {
                currency: PaymentCurrency("USD".into()),
                amount: 5_000,
            },
            status: PayoutStatus::Created,
            payout_account_id: "acct-1".into(),
            provider_reference: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        payout.validate().unwrap();
        assert_eq!(
            SettlementPolicy {
                hold_period_seconds: 21 * 24 * 60 * 60,
                reserve_bps: 10_001,
            }
            .validate()
            .unwrap_err(),
            PaymentInfrastructureError::InvalidSettlementPolicy
        );
    }
}
