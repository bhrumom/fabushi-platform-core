//! Deterministic accounting and payment state machine for Fabushi Pay.
//!
//! Provider SDKs and credentials deliberately live outside this module. The
//! core only accepts outcomes that a trusted adapter has already authenticated.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;

pub const CREDITS_CURRENCY: &str = "FBC";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentCurrency(pub String);

impl PaymentCurrency {
    pub fn credits() -> Self {
        Self(CREDITS_CURRENCY.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Money {
    pub currency: PaymentCurrency,
    pub amount: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProductKind {
    DigitalConsumable,
    DigitalDurable,
    Subscription,
    Physical,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaymentRail {
    Credits,
    AppleInAppPurchase,
    GooglePlayBilling,
    WebProvider,
    MerchantProvider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaymentStatus {
    Created,
    RequiresAction,
    Processing,
    Succeeded,
    Failed,
    Cancelled,
    PartiallyRefunded,
    Refunded,
}

impl PaymentStatus {
    fn blocks_first_success(self) -> bool {
        matches!(self, Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentLedgerLine {
    pub account_id: String,
    pub currency: PaymentCurrency,
    /// Signed minor-unit amount. Every currency in an entry must sum to zero.
    pub amount: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentLedgerEntry {
    pub entry_id: String,
    pub reference_type: String,
    pub reference_id: String,
    pub created_at_ms: i64,
    pub lines: Vec<PaymentLedgerLine>,
}

impl PaymentLedgerEntry {
    pub fn validate(&self) -> Result<(), PayError> {
        if self.lines.len() < 2 {
            return Err(PayError::NotEnoughLedgerLines);
        }
        let mut totals = BTreeMap::<&PaymentCurrency, i128>::new();
        for line in &self.lines {
            if line.amount == 0 {
                return Err(PayError::ZeroLedgerLine);
            }
            *totals.entry(&line.currency).or_default() += i128::from(line.amount);
        }
        if let Some((currency, amount)) = totals.into_iter().find(|(_, total)| *total != 0) {
            return Err(PayError::UnbalancedLedger {
                currency: currency.0.clone(),
                amount,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreatePaymentIntent {
    pub idempotency_key: String,
    pub user_id: String,
    pub mini_app_id: String,
    pub developer_id: String,
    pub sku: String,
    pub product_kind: ProductKind,
    pub rail: PaymentRail,
    pub amount: Money,
    pub platform_fee_bps: u16,
    pub created_at_ms: i64,
}

impl CreatePaymentIntent {
    pub fn validate(&self) -> Result<(), PayError> {
        if [
            self.idempotency_key.as_str(),
            self.user_id.as_str(),
            self.mini_app_id.as_str(),
            self.developer_id.as_str(),
            self.sku.as_str(),
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        {
            return Err(PayError::MissingStableId);
        }
        if self.amount.amount <= 0 {
            return Err(PayError::InvalidAmount(self.amount.amount));
        }
        if self.platform_fee_bps > 10_000 {
            return Err(PayError::InvalidFeeBps(self.platform_fee_bps));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaymentIntent {
    pub payment_id: String,
    pub idempotency_key: String,
    pub user_id: String,
    pub mini_app_id: String,
    pub developer_id: String,
    pub sku: String,
    pub product_kind: ProductKind,
    pub rail: PaymentRail,
    pub amount: Money,
    pub platform_fee_bps: u16,
    pub status: PaymentStatus,
    pub provider_reference: Option<String>,
    pub refunded_amount: i64,
    /// Net developer revenue currently held in Available for this payment.
    pub released_developer_amount: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl PaymentIntent {
    pub fn fee_for(&self, gross: i64) -> i64 {
        proportional(gross, self.platform_fee_bps)
    }

    pub fn platform_fee(&self) -> i64 {
        self.fee_for(self.amount.amount)
    }

    pub fn developer_net(&self) -> i64 {
        self.amount.amount.saturating_sub(self.platform_fee())
    }

    pub fn refundable_amount(&self) -> i64 {
        self.amount.amount.saturating_sub(self.refunded_amount)
    }

    pub fn refunded_developer_net(&self) -> i64 {
        self.refunded_amount
            .saturating_sub(self.fee_for(self.refunded_amount))
    }

    pub fn developer_net_after_refunds(&self) -> i64 {
        self.developer_net()
            .saturating_sub(self.refunded_developer_net())
    }

    pub fn pending_developer_amount(&self) -> i64 {
        self.developer_net_after_refunds()
            .saturating_sub(self.released_developer_amount)
    }

    fn matches_create(&self, request: &CreatePaymentIntent) -> bool {
        self.user_id == request.user_id
            && self.mini_app_id == request.mini_app_id
            && self.developer_id == request.developer_id
            && self.sku == request.sku
            && self.product_kind == request.product_kind
            && self.rail == request.rail
            && self.amount == request.amount
            && self.platform_fee_bps == request.platform_fee_bps
    }
}

fn proportional(amount: i64, bps: u16) -> i64 {
    let value = i128::from(amount) * i128::from(bps) / 10_000;
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderOutcome {
    pub provider_reference: String,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefundRequest {
    pub idempotency_key: String,
    pub payment_id: String,
    pub amount: i64,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettlementRelease {
    pub idempotency_key: String,
    pub payment_id: String,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentMutation {
    pub payment: PaymentIntent,
    pub ledger_entry: Option<PaymentLedgerEntry>,
}

#[derive(Debug, Clone)]
struct MutationRecord {
    operation: &'static str,
    payment_id: String,
    entry: PaymentLedgerEntry,
}

#[derive(Default)]
pub struct PayEngine {
    payments: HashMap<String, PaymentIntent>,
    payment_by_idempotency: HashMap<String, String>,
    mutation_results: HashMap<String, MutationRecord>,
}

impl PayEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, payment_id: &str) -> Option<&PaymentIntent> {
        self.payments.get(payment_id)
    }

    pub fn create_payment(
        &mut self,
        request: CreatePaymentIntent,
    ) -> Result<PaymentIntent, PayError> {
        request.validate()?;
        if let Some(payment_id) = self.payment_by_idempotency.get(&request.idempotency_key) {
            let existing = self.payments.get(payment_id).cloned().ok_or_else(|| {
                PayError::CorruptIdempotencyIndex(request.idempotency_key.clone())
            })?;
            if !existing.matches_create(&request) {
                return Err(PayError::IdempotencyConflict(request.idempotency_key));
            }
            return Ok(existing);
        }
        let payment_id = stable_id(&[
            "payment",
            &request.idempotency_key,
            &request.user_id,
            &request.mini_app_id,
            &request.sku,
        ]);
        let payment = PaymentIntent {
            payment_id,
            idempotency_key: request.idempotency_key.clone(),
            user_id: request.user_id,
            mini_app_id: request.mini_app_id,
            developer_id: request.developer_id,
            sku: request.sku,
            product_kind: request.product_kind,
            rail: request.rail,
            amount: request.amount,
            platform_fee_bps: request.platform_fee_bps,
            status: PaymentStatus::Created,
            provider_reference: None,
            refunded_amount: 0,
            released_developer_amount: 0,
            created_at_ms: request.created_at_ms,
            updated_at_ms: request.created_at_ms,
        };
        self.payment_by_idempotency
            .insert(request.idempotency_key, payment.payment_id.clone());
        self.payments
            .insert(payment.payment_id.clone(), payment.clone());
        Ok(payment)
    }

    pub fn mark_requires_action(
        &mut self,
        payment_id: &str,
        now_ms: i64,
    ) -> Result<PaymentIntent, PayError> {
        self.transition(payment_id, PaymentStatus::RequiresAction, now_ms)
    }

    pub fn mark_processing(
        &mut self,
        payment_id: &str,
        now_ms: i64,
    ) -> Result<PaymentIntent, PayError> {
        self.transition(payment_id, PaymentStatus::Processing, now_ms)
    }

    pub fn fail(&mut self, payment_id: &str, now_ms: i64) -> Result<PaymentIntent, PayError> {
        self.transition(payment_id, PaymentStatus::Failed, now_ms)
    }

    pub fn cancel(&mut self, payment_id: &str, now_ms: i64) -> Result<PaymentIntent, PayError> {
        self.transition(payment_id, PaymentStatus::Cancelled, now_ms)
    }

    pub fn succeed(
        &mut self,
        payment_id: &str,
        outcome: ProviderOutcome,
    ) -> Result<PaymentMutation, PayError> {
        let payment = self.payment(payment_id)?;
        if matches!(
            payment.status,
            PaymentStatus::Succeeded | PaymentStatus::PartiallyRefunded | PaymentStatus::Refunded
        ) {
            if payment.provider_reference.as_deref() == Some(outcome.provider_reference.as_str()) {
                return Ok(PaymentMutation {
                    payment,
                    ledger_entry: None,
                });
            }
            return Err(PayError::ProviderReferenceConflict);
        }
        if payment.status.blocks_first_success() {
            return Err(PayError::InvalidTransition {
                from: payment.status,
                to: PaymentStatus::Succeeded,
            });
        }
        if outcome.provider_reference.trim().is_empty() {
            return Err(PayError::MissingStableId);
        }

        let mut updated = payment.clone();
        updated.status = PaymentStatus::Succeeded;
        updated.provider_reference = Some(outcome.provider_reference.clone());
        updated.updated_at_ms = outcome.occurred_at_ms;
        let entry = balanced_entry(
            "payment",
            &payment.payment_id,
            &outcome.provider_reference,
            outcome.occurred_at_ms,
            vec![
                line(
                    provider_account(&payment.amount.currency),
                    &payment.amount.currency,
                    -payment.amount.amount,
                ),
                line(
                    developer_pending(&payment.developer_id),
                    &payment.amount.currency,
                    payment.developer_net(),
                ),
                line(
                    "platform-revenue".into(),
                    &payment.amount.currency,
                    payment.platform_fee(),
                ),
            ],
        )?;
        self.payments
            .insert(payment.payment_id.clone(), updated.clone());
        Ok(PaymentMutation {
            payment: updated,
            ledger_entry: Some(entry),
        })
    }

    pub fn refund(&mut self, request: RefundRequest) -> Result<PaymentMutation, PayError> {
        ensure_key(&request.idempotency_key)?;
        if let Some(entry) =
            self.replay_mutation(&request.idempotency_key, "refund", &request.payment_id)?
        {
            let payment = self.payment(&request.payment_id)?;
            return Ok(PaymentMutation {
                payment,
                ledger_entry: Some(entry),
            });
        }
        if request.amount <= 0 {
            return Err(PayError::InvalidAmount(request.amount));
        }
        let payment = self.payment(&request.payment_id)?;
        if !matches!(
            payment.status,
            PaymentStatus::Succeeded | PaymentStatus::PartiallyRefunded
        ) {
            return Err(PayError::RefundNotAllowed(payment.status));
        }
        if request.amount > payment.refundable_amount() {
            return Err(PayError::RefundExceedsPayment {
                requested: request.amount,
                remaining: payment.refundable_amount(),
            });
        }

        let new_refunded = payment.refunded_amount.saturating_add(request.amount);
        // Cumulative-difference rounding guarantees that a full refund reverses
        // exactly the original fee even when many tiny partial refunds occur.
        let fee_refund = payment
            .fee_for(new_refunded)
            .saturating_sub(payment.fee_for(payment.refunded_amount));
        let developer_refund = request.amount.saturating_sub(fee_refund);
        let pending_debit = developer_refund.min(payment.pending_developer_amount());
        let available_debit = developer_refund.saturating_sub(pending_debit);
        if available_debit > payment.released_developer_amount {
            return Err(PayError::RefundAccountingInvariant);
        }

        let mut lines = Vec::with_capacity(4);
        if pending_debit > 0 {
            lines.push(line(
                developer_pending(&payment.developer_id),
                &payment.amount.currency,
                -pending_debit,
            ));
        }
        if available_debit > 0 {
            lines.push(line(
                developer_available(&payment.developer_id),
                &payment.amount.currency,
                -available_debit,
            ));
        }
        if fee_refund > 0 {
            lines.push(line(
                "platform-revenue".into(),
                &payment.amount.currency,
                -fee_refund,
            ));
        }
        lines.push(line(
            provider_account(&payment.amount.currency),
            &payment.amount.currency,
            request.amount,
        ));
        let entry = balanced_entry(
            "refund",
            &payment.payment_id,
            &request.idempotency_key,
            request.occurred_at_ms,
            lines,
        )?;

        let mut updated = payment.clone();
        updated.refunded_amount = new_refunded;
        updated.released_developer_amount = updated
            .released_developer_amount
            .saturating_sub(available_debit);
        updated.status = if new_refunded == payment.amount.amount {
            PaymentStatus::Refunded
        } else {
            PaymentStatus::PartiallyRefunded
        };
        updated.updated_at_ms = request.occurred_at_ms;
        self.payments
            .insert(payment.payment_id.clone(), updated.clone());
        self.record_mutation(
            request.idempotency_key,
            "refund",
            payment.payment_id,
            entry.clone(),
        );
        Ok(PaymentMutation {
            payment: updated,
            ledger_entry: Some(entry),
        })
    }

    pub fn release_settlement(
        &mut self,
        request: SettlementRelease,
    ) -> Result<PaymentLedgerEntry, PayError> {
        ensure_key(&request.idempotency_key)?;
        if let Some(entry) = self.replay_mutation(
            &request.idempotency_key,
            "settlement-release",
            &request.payment_id,
        )? {
            return Ok(entry);
        }
        let payment = self.payment(&request.payment_id)?;
        if !matches!(
            payment.status,
            PaymentStatus::Succeeded | PaymentStatus::PartiallyRefunded
        ) {
            return Err(PayError::SettlementNotAllowed(payment.status));
        }
        let releasable = payment.pending_developer_amount();
        if releasable <= 0 {
            return Err(PayError::NothingToSettle);
        }
        let entry = balanced_entry(
            "settlement-release",
            &payment.payment_id,
            &request.idempotency_key,
            request.occurred_at_ms,
            vec![
                line(
                    developer_pending(&payment.developer_id),
                    &payment.amount.currency,
                    -releasable,
                ),
                line(
                    developer_available(&payment.developer_id),
                    &payment.amount.currency,
                    releasable,
                ),
            ],
        )?;
        let mut updated = payment.clone();
        updated.released_developer_amount =
            updated.released_developer_amount.saturating_add(releasable);
        updated.updated_at_ms = request.occurred_at_ms;
        self.payments.insert(payment.payment_id.clone(), updated);
        self.record_mutation(
            request.idempotency_key,
            "settlement-release",
            payment.payment_id,
            entry.clone(),
        );
        Ok(entry)
    }

    fn payment(&self, payment_id: &str) -> Result<PaymentIntent, PayError> {
        self.payments
            .get(payment_id)
            .cloned()
            .ok_or_else(|| PayError::PaymentNotFound(payment_id.into()))
    }

    fn replay_mutation(
        &self,
        key: &str,
        operation: &'static str,
        payment_id: &str,
    ) -> Result<Option<PaymentLedgerEntry>, PayError> {
        let Some(record) = self.mutation_results.get(key) else {
            return Ok(None);
        };
        if record.operation != operation || record.payment_id != payment_id {
            return Err(PayError::IdempotencyConflict(key.into()));
        }
        Ok(Some(record.entry.clone()))
    }

    fn record_mutation(
        &mut self,
        key: String,
        operation: &'static str,
        payment_id: String,
        entry: PaymentLedgerEntry,
    ) {
        self.mutation_results.insert(
            key,
            MutationRecord {
                operation,
                payment_id,
                entry,
            },
        );
    }

    fn transition(
        &mut self,
        payment_id: &str,
        next: PaymentStatus,
        now_ms: i64,
    ) -> Result<PaymentIntent, PayError> {
        let payment = self
            .payments
            .get_mut(payment_id)
            .ok_or_else(|| PayError::PaymentNotFound(payment_id.into()))?;
        let allowed = matches!(
            (payment.status, next),
            (PaymentStatus::Created, PaymentStatus::RequiresAction)
                | (PaymentStatus::Created, PaymentStatus::Processing)
                | (PaymentStatus::Created, PaymentStatus::Failed)
                | (PaymentStatus::Created, PaymentStatus::Cancelled)
                | (PaymentStatus::RequiresAction, PaymentStatus::Processing)
                | (PaymentStatus::RequiresAction, PaymentStatus::Failed)
                | (PaymentStatus::RequiresAction, PaymentStatus::Cancelled)
                | (PaymentStatus::Processing, PaymentStatus::Failed)
        );
        if !allowed {
            return Err(PayError::InvalidTransition {
                from: payment.status,
                to: next,
            });
        }
        payment.status = next;
        payment.updated_at_ms = now_ms;
        Ok(payment.clone())
    }
}

fn stable_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn ensure_key(key: &str) -> Result<(), PayError> {
    if key.trim().is_empty() {
        Err(PayError::MissingStableId)
    } else {
        Ok(())
    }
}

fn provider_account(currency: &PaymentCurrency) -> String {
    format!("provider-clearing:{}", currency.0)
}

fn developer_pending(developer_id: &str) -> String {
    format!("developer-pending:{developer_id}")
}

fn developer_available(developer_id: &str) -> String {
    format!("developer-available:{developer_id}")
}

fn line(account_id: String, currency: &PaymentCurrency, amount: i64) -> PaymentLedgerLine {
    PaymentLedgerLine {
        account_id,
        currency: currency.clone(),
        amount,
    }
}

fn balanced_entry(
    reference_type: &str,
    reference_id: &str,
    event_key: &str,
    created_at_ms: i64,
    lines: Vec<PaymentLedgerLine>,
) -> Result<PaymentLedgerEntry, PayError> {
    let lines = lines
        .into_iter()
        .filter(|line| line.amount != 0)
        .collect::<Vec<_>>();
    let entry = PaymentLedgerEntry {
        entry_id: stable_id(&["ledger", reference_type, reference_id, event_key]),
        reference_type: reference_type.into(),
        reference_id: reference_id.into(),
        created_at_ms,
        lines,
    };
    entry.validate()?;
    Ok(entry)
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PayError {
    #[error("payment amount must be positive, got {0}")]
    InvalidAmount(i64),
    #[error("payment or idempotency identifiers must not be empty")]
    MissingStableId,
    #[error("platform fee basis points must be <= 10000, got {0}")]
    InvalidFeeBps(u16),
    #[error("ledger entries require at least two lines")]
    NotEnoughLedgerLines,
    #[error("ledger lines must not have zero amount")]
    ZeroLedgerLine,
    #[error("ledger entry is unbalanced for {currency}: {amount}")]
    UnbalancedLedger { currency: String, amount: i128 },
    #[error("payment not found: {0}")]
    PaymentNotFound(String),
    #[error("invalid payment transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: PaymentStatus,
        to: PaymentStatus,
    },
    #[error("provider reference conflicts with an already successful payment")]
    ProviderReferenceConflict,
    #[error("idempotency key was reused with different semantics: {0}")]
    IdempotencyConflict(String),
    #[error("refund is not allowed while payment is {0:?}")]
    RefundNotAllowed(PaymentStatus),
    #[error("refund {requested} exceeds remaining refundable amount {remaining}")]
    RefundExceedsPayment { requested: i64, remaining: i64 },
    #[error("refund accounting invariant was violated")]
    RefundAccountingInvariant,
    #[error("settlement release is not allowed while payment is {0:?}")]
    SettlementNotAllowed(PaymentStatus),
    #[error("payment has no developer revenue left to settle")]
    NothingToSettle,
    #[error("corrupt payment idempotency index: {0}")]
    CorruptIdempotencyIndex(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(key: &str) -> CreatePaymentIntent {
        CreatePaymentIntent {
            idempotency_key: key.into(),
            user_id: "user-1".into(),
            mini_app_id: "miniapp-1".into(),
            developer_id: "dev-1".into(),
            sku: "pro.month".into(),
            product_kind: ProductKind::Subscription,
            rail: PaymentRail::Credits,
            amount: Money {
                currency: PaymentCurrency::credits(),
                amount: 1_000,
            },
            platform_fee_bps: 1_500,
            created_at_ms: 10,
        }
    }

    fn succeed(engine: &mut PayEngine, payment_id: &str) {
        engine
            .succeed(
                payment_id,
                ProviderOutcome {
                    provider_reference: "provider-1".into(),
                    occurred_at_ms: 20,
                },
            )
            .unwrap();
    }

    #[test]
    fn creation_is_idempotent_and_conflicting_reuse_is_rejected() {
        let mut engine = PayEngine::new();
        let first = engine.create_payment(request("same")).unwrap();
        let second = engine.create_payment(request("same")).unwrap();
        assert_eq!(first.payment_id, second.payment_id);
        assert_eq!(first.payment_id.len(), 64);

        let mut changed = request("same");
        changed.sku = "other.sku".into();
        assert_eq!(
            engine.create_payment(changed).unwrap_err(),
            PayError::IdempotencyConflict("same".into())
        );
    }

    #[test]
    fn success_posts_balanced_pending_revenue_and_replay_is_safe() {
        let mut engine = PayEngine::new();
        let payment = engine.create_payment(request("one")).unwrap();
        let outcome = ProviderOutcome {
            provider_reference: "provider-1".into(),
            occurred_at_ms: 20,
        };
        let first = engine
            .succeed(&payment.payment_id, outcome.clone())
            .unwrap();
        assert_eq!(first.payment.platform_fee(), 150);
        assert_eq!(first.payment.developer_net(), 850);
        first.ledger_entry.unwrap().validate().unwrap();
        assert!(
            engine
                .succeed(&payment.payment_id, outcome)
                .unwrap()
                .ledger_entry
                .is_none()
        );
    }

    #[test]
    fn release_cannot_be_repeated_with_a_different_key() {
        let mut engine = PayEngine::new();
        let payment = engine.create_payment(request("one")).unwrap();
        succeed(&mut engine, &payment.payment_id);
        let first = engine
            .release_settlement(SettlementRelease {
                idempotency_key: "release-1".into(),
                payment_id: payment.payment_id.clone(),
                occurred_at_ms: 100,
            })
            .unwrap();
        let replay = engine
            .release_settlement(SettlementRelease {
                idempotency_key: "release-1".into(),
                payment_id: payment.payment_id.clone(),
                occurred_at_ms: 100,
            })
            .unwrap();
        assert_eq!(first, replay);
        assert_eq!(
            engine
                .release_settlement(SettlementRelease {
                    idempotency_key: "release-2".into(),
                    payment_id: payment.payment_id,
                    occurred_at_ms: 110
                })
                .unwrap_err(),
            PayError::NothingToSettle
        );
    }

    #[test]
    fn refund_after_release_debits_available_not_pending() {
        let mut engine = PayEngine::new();
        let payment = engine.create_payment(request("one")).unwrap();
        succeed(&mut engine, &payment.payment_id);
        engine
            .release_settlement(SettlementRelease {
                idempotency_key: "release-1".into(),
                payment_id: payment.payment_id.clone(),
                occurred_at_ms: 100,
            })
            .unwrap();
        let mutation = engine
            .refund(RefundRequest {
                idempotency_key: "refund-1".into(),
                payment_id: payment.payment_id.clone(),
                amount: 200,
                occurred_at_ms: 120,
            })
            .unwrap();
        let entry = mutation.ledger_entry.unwrap();
        entry.validate().unwrap();
        assert!(
            entry
                .lines
                .iter()
                .any(|line| line.account_id == "developer-available:dev-1" && line.amount == -170)
        );
        assert_eq!(
            engine
                .get(&payment.payment_id)
                .unwrap()
                .released_developer_amount,
            680
        );
    }

    #[test]
    fn cumulative_rounding_fully_reverses_platform_fee() {
        let mut engine = PayEngine::new();
        let mut create = request("rounding");
        create.amount.amount = 7;
        create.platform_fee_bps = 1_500;
        let payment = engine.create_payment(create).unwrap();
        succeed(&mut engine, &payment.payment_id);
        assert_eq!(engine.get(&payment.payment_id).unwrap().platform_fee(), 1);

        let mut reversed_fee = 0;
        for index in 0..7 {
            let mutation = engine
                .refund(RefundRequest {
                    idempotency_key: format!("refund-{index}"),
                    payment_id: payment.payment_id.clone(),
                    amount: 1,
                    occurred_at_ms: 30 + index,
                })
                .unwrap();
            if let Some(entry) = mutation.ledger_entry {
                reversed_fee += entry
                    .lines
                    .iter()
                    .filter(|line| line.account_id == "platform-revenue")
                    .map(|line| -line.amount)
                    .sum::<i64>();
            }
        }
        assert_eq!(reversed_fee, 1);
        assert_eq!(
            engine.get(&payment.payment_id).unwrap().status,
            PaymentStatus::Refunded
        );
    }

    #[test]
    fn zero_value_split_lines_are_not_posted() {
        for fee_bps in [0, 10_000] {
            let mut engine = PayEngine::new();
            let mut create = request(&format!("fee-{fee_bps}"));
            create.platform_fee_bps = fee_bps;
            let payment = engine.create_payment(create).unwrap();
            let mutation = engine
                .succeed(
                    &payment.payment_id,
                    ProviderOutcome {
                        provider_reference: format!("provider-{fee_bps}"),
                        occurred_at_ms: 20,
                    },
                )
                .unwrap();
            let entry = mutation.ledger_entry.unwrap();
            entry.validate().unwrap();
            assert_eq!(entry.lines.len(), 2);
            assert!(entry.lines.iter().all(|line| line.amount != 0));
        }
    }

    #[test]
    fn mutation_idempotency_cannot_cross_payments_or_operations() {
        let mut engine = PayEngine::new();
        let first = engine.create_payment(request("first")).unwrap();
        let mut second_request = request("second");
        second_request.sku = "other".into();
        let second = engine.create_payment(second_request).unwrap();
        succeed(&mut engine, &first.payment_id);
        engine
            .release_settlement(SettlementRelease {
                idempotency_key: "mutation-1".into(),
                payment_id: first.payment_id,
                occurred_at_ms: 40,
            })
            .unwrap();
        succeed(&mut engine, &second.payment_id);
        assert_eq!(
            engine
                .refund(RefundRequest {
                    idempotency_key: "mutation-1".into(),
                    payment_id: second.payment_id,
                    amount: 1,
                    occurred_at_ms: 50
                })
                .unwrap_err(),
            PayError::IdempotencyConflict("mutation-1".into())
        );
    }
}
