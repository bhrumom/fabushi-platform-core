use crate::actor::ActorId;
use crate::payment::Money;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WalletAccountId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletAccount {
    pub id: WalletAccountId,
    pub owner_id: ActorId,
    pub balances_minor: BTreeMap<String, i64>,
    pub frozen: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl WalletAccount {
    pub fn balance(&self, currency: &str) -> i64 {
        self.balances_minor
            .get(&normalize_currency(currency))
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LedgerEntryKind {
    Credit,
    Transfer,
    Refund,
    Adjustment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    pub id: String,
    pub request_id: String,
    pub kind: LedgerEntryKind,
    pub from_account_id: Option<WalletAccountId>,
    pub to_account_id: Option<WalletAccountId>,
    pub amount: Money,
    pub reference: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalletLedger {
    pub accounts: BTreeMap<WalletAccountId, WalletAccount>,
    pub entries: BTreeMap<String, LedgerEntry>,
    request_entries: BTreeMap<String, String>,
}

impl WalletLedger {
    pub fn create_account(
        &mut self,
        id: WalletAccountId,
        owner_id: ActorId,
        now_ms: i64,
    ) -> Result<(), WalletError> {
        if id.0.trim().is_empty() {
            return Err(WalletError::InvalidAccountId);
        }
        if self.accounts.contains_key(&id) {
            return Err(WalletError::DuplicateAccount(id));
        }
        self.accounts.insert(
            id.clone(),
            WalletAccount {
                id,
                owner_id,
                balances_minor: BTreeMap::new(),
                frozen: false,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            },
        );
        Ok(())
    }

    pub fn credit(
        &mut self,
        request_id: impl Into<String>,
        account_id: &WalletAccountId,
        amount: Money,
        reference: Option<String>,
        now_ms: i64,
    ) -> Result<LedgerEntry, WalletError> {
        let request_id = request_id.into();
        if let Some(existing) = self.entry_for_request(&request_id) {
            return Ok(existing.clone());
        }
        validate_amount(&amount)?;
        let account = self
            .accounts
            .get_mut(account_id)
            .ok_or_else(|| WalletError::AccountNotFound(account_id.clone()))?;
        if account.frozen {
            return Err(WalletError::AccountFrozen(account_id.clone()));
        }
        let currency = normalize_currency(&amount.currency);
        let balance = account.balances_minor.entry(currency.clone()).or_default();
        *balance = balance
            .checked_add(amount.amount_minor)
            .ok_or(WalletError::BalanceOverflow)?;
        account.updated_at_ms = now_ms;
        let entry = LedgerEntry {
            id: format!("ledger:{}", self.entries.len().saturating_add(1)),
            request_id: request_id.clone(),
            kind: LedgerEntryKind::Credit,
            from_account_id: None,
            to_account_id: Some(account_id.clone()),
            amount: Money {
                currency,
                amount_minor: amount.amount_minor,
            },
            reference,
            created_at_ms: now_ms,
        };
        self.insert_entry(entry.clone());
        Ok(entry)
    }

    pub fn transfer(
        &mut self,
        request_id: impl Into<String>,
        from_account_id: &WalletAccountId,
        to_account_id: &WalletAccountId,
        amount: Money,
        reference: Option<String>,
        now_ms: i64,
    ) -> Result<LedgerEntry, WalletError> {
        let request_id = request_id.into();
        if let Some(existing) = self.entry_for_request(&request_id) {
            return Ok(existing.clone());
        }
        validate_amount(&amount)?;
        if from_account_id == to_account_id {
            return Err(WalletError::SameAccountTransfer);
        }
        let currency = normalize_currency(&amount.currency);
        let debit_balance = self
            .accounts
            .get(from_account_id)
            .ok_or_else(|| WalletError::AccountNotFound(from_account_id.clone()))?
            .balance(&currency);
        if self
            .accounts
            .get(from_account_id)
            .is_some_and(|account| account.frozen)
        {
            return Err(WalletError::AccountFrozen(from_account_id.clone()));
        }
        if self
            .accounts
            .get(to_account_id)
            .ok_or_else(|| WalletError::AccountNotFound(to_account_id.clone()))?
            .frozen
        {
            return Err(WalletError::AccountFrozen(to_account_id.clone()));
        }
        if debit_balance < amount.amount_minor {
            return Err(WalletError::InsufficientFunds {
                account_id: from_account_id.clone(),
                currency,
                available_minor: debit_balance,
                required_minor: amount.amount_minor,
            });
        }

        {
            let from = self
                .accounts
                .get_mut(from_account_id)
                .ok_or_else(|| WalletError::AccountNotFound(from_account_id.clone()))?;
            let balance = from.balances_minor.entry(currency.clone()).or_default();
            *balance = balance
                .checked_sub(amount.amount_minor)
                .ok_or(WalletError::BalanceOverflow)?;
            from.updated_at_ms = now_ms;
        }
        {
            let to = self
                .accounts
                .get_mut(to_account_id)
                .ok_or_else(|| WalletError::AccountNotFound(to_account_id.clone()))?;
            let balance = to.balances_minor.entry(currency.clone()).or_default();
            *balance = balance
                .checked_add(amount.amount_minor)
                .ok_or(WalletError::BalanceOverflow)?;
            to.updated_at_ms = now_ms;
        }

        let entry = LedgerEntry {
            id: format!("ledger:{}", self.entries.len().saturating_add(1)),
            request_id: request_id.clone(),
            kind: LedgerEntryKind::Transfer,
            from_account_id: Some(from_account_id.clone()),
            to_account_id: Some(to_account_id.clone()),
            amount: Money {
                currency,
                amount_minor: amount.amount_minor,
            },
            reference,
            created_at_ms: now_ms,
        };
        self.insert_entry(entry.clone());
        Ok(entry)
    }

    pub fn refund_transfer(
        &mut self,
        request_id: impl Into<String>,
        original_entry_id: &str,
        now_ms: i64,
    ) -> Result<LedgerEntry, WalletError> {
        let request_id = request_id.into();
        if let Some(existing) = self.entry_for_request(&request_id) {
            return Ok(existing.clone());
        }
        let original = self
            .entries
            .get(original_entry_id)
            .cloned()
            .ok_or_else(|| WalletError::EntryNotFound(original_entry_id.to_string()))?;
        if original.kind != LedgerEntryKind::Transfer {
            return Err(WalletError::NotRefundable(original_entry_id.to_string()));
        }
        let original_from = original
            .from_account_id
            .clone()
            .ok_or_else(|| WalletError::NotRefundable(original_entry_id.to_string()))?;
        let original_to = original
            .to_account_id
            .clone()
            .ok_or_else(|| WalletError::NotRefundable(original_entry_id.to_string()))?;
        let mut entry = self.transfer(
            request_id.clone(),
            &original_to,
            &original_from,
            original.amount.clone(),
            Some(format!("refund:{original_entry_id}")),
            now_ms,
        )?;
        entry.kind = LedgerEntryKind::Refund;
        self.entries.insert(entry.id.clone(), entry.clone());
        Ok(entry)
    }

    pub fn freeze(
        &mut self,
        account_id: &WalletAccountId,
        frozen: bool,
    ) -> Result<(), WalletError> {
        let account = self
            .accounts
            .get_mut(account_id)
            .ok_or_else(|| WalletError::AccountNotFound(account_id.clone()))?;
        account.frozen = frozen;
        Ok(())
    }

    pub fn entry_for_request(&self, request_id: &str) -> Option<&LedgerEntry> {
        self.request_entries
            .get(request_id)
            .and_then(|entry_id| self.entries.get(entry_id))
    }

    fn insert_entry(&mut self, entry: LedgerEntry) {
        self.request_entries
            .insert(entry.request_id.clone(), entry.id.clone());
        self.entries.insert(entry.id.clone(), entry);
    }
}

fn validate_amount(amount: &Money) -> Result<(), WalletError> {
    if amount.currency.trim().len() < 3 || amount.amount_minor <= 0 {
        return Err(WalletError::InvalidAmount);
    }
    Ok(())
}

fn normalize_currency(currency: &str) -> String {
    currency.trim().to_ascii_uppercase()
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WalletError {
    #[error("wallet account id is invalid")]
    InvalidAccountId,
    #[error("wallet account {0:?} already exists")]
    DuplicateAccount(WalletAccountId),
    #[error("wallet account {0:?} was not found")]
    AccountNotFound(WalletAccountId),
    #[error("wallet account {0:?} is frozen")]
    AccountFrozen(WalletAccountId),
    #[error("wallet amount is invalid")]
    InvalidAmount,
    #[error("wallet transfer source and destination must differ")]
    SameAccountTransfer,
    #[error("wallet balance overflow")]
    BalanceOverflow,
    #[error("wallet account {account_id:?} has {available_minor} {currency} minor units but requires {required_minor}")]
    InsufficientFunds {
        account_id: WalletAccountId,
        currency: String,
        available_minor: i64,
        required_minor: i64,
    },
    #[error("wallet ledger entry {0} was not found")]
    EntryNotFound(String),
    #[error("wallet ledger entry {0} is not refundable")]
    NotRefundable(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd(amount_minor: i64) -> Money {
        Money {
            currency: "usd".into(),
            amount_minor,
        }
    }

    #[test]
    fn transfer_is_idempotent_and_refundable() {
        let buyer = WalletAccountId("wallet:buyer".into());
        let seller = WalletAccountId("wallet:seller".into());
        let mut ledger = WalletLedger::default();
        ledger
            .create_account(buyer.clone(), ActorId::new("human:buyer"), 1)
            .unwrap();
        ledger
            .create_account(seller.clone(), ActorId::new("human:seller"), 1)
            .unwrap();
        ledger
            .credit("credit:1", &buyer, usd(1_000), None, 2)
            .unwrap();
        let transfer = ledger
            .transfer(
                "pay:1",
                &buyer,
                &seller,
                usd(250),
                Some("invoice:1".into()),
                3,
            )
            .unwrap();
        let duplicate = ledger
            .transfer("pay:1", &buyer, &seller, usd(250), None, 4)
            .unwrap();
        assert_eq!(duplicate.id, transfer.id);
        assert_eq!(ledger.accounts[&buyer].balance("USD"), 750);
        assert_eq!(ledger.accounts[&seller].balance("USD"), 250);

        let refund = ledger.refund_transfer("refund:1", &transfer.id, 5).unwrap();
        assert_eq!(refund.kind, LedgerEntryKind::Refund);
        assert_eq!(ledger.accounts[&buyer].balance("USD"), 1_000);
        assert_eq!(ledger.accounts[&seller].balance("USD"), 0);
    }
}
