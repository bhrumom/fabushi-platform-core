use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementRegion {
    MainlandChina,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettlementSource {
    WechatOrder,
    AlipayOrder,
    AppleStoreProceeds,
    GoogleStoreProceeds,
    WebMarketplace,
    OtherExternalProceeds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutPurpose {
    OriginalOrderSplit,
    ExternalProceedsPayout,
    MarketplacePayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayoutProvider {
    StripeConnect,
    AdyenPlatform,
    PaypalMultiparty,
    PaypalPayouts,
    WechatPlatform,
    AlipayPlatform,
    LianlianAccountPlus,
    HuifuDougong,
}

impl PayoutProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StripeConnect => "stripe_connect",
            Self::AdyenPlatform => "adyen_platform",
            Self::PaypalMultiparty => "paypal_multiparty",
            Self::PaypalPayouts => "paypal_payouts",
            Self::WechatPlatform => "wechat_platform",
            Self::AlipayPlatform => "alipay_platform",
            Self::LianlianAccountPlus => "lianlian_account_plus",
            Self::HuifuDougong => "huifu_dougong",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligiblePayoutAccount {
    pub payout_account_id: String,
    pub provider: PayoutProvider,
    pub state_active: bool,
    pub kyc_verified: bool,
    pub payouts_enabled: bool,
    pub currencies: Vec<String>,
    pub purposes: Vec<PayoutPurpose>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementWaterfallInput {
    pub gross_amount: i64,
    pub tax_amount: i64,
    pub provider_fee_amount: i64,
    pub refund_amount: i64,
    pub chargeback_amount: i64,
    pub platform_fee_bps: u16,
    pub reserve_bps: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementWaterfall {
    pub gross_amount: i64,
    pub tax_amount: i64,
    pub provider_fee_amount: i64,
    pub refund_amount: i64,
    pub chargeback_amount: i64,
    pub net_receipts: i64,
    pub platform_fee_amount: i64,
    pub reserve_amount: i64,
    pub developer_payable_amount: i64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PayoutPolicyError {
    #[error("settlement amounts must be non-negative")]
    NegativeAmount,
    #[error("settlement deductions exceed gross amount")]
    DeductionsExceedGross,
    #[error("basis points must be between 0 and 10000")]
    InvalidBasisPoints,
    #[error("no eligible payout provider is active for region, purpose and currency")]
    NoEligibleProvider,
}

fn proportional(amount: i64, bps: u16) -> i64 {
    ((amount as i128) * i128::from(bps) / 10_000i128) as i64
}

pub fn settlement_waterfall(
    input: SettlementWaterfallInput,
) -> Result<SettlementWaterfall, PayoutPolicyError> {
    if [
        input.gross_amount,
        input.tax_amount,
        input.provider_fee_amount,
        input.refund_amount,
        input.chargeback_amount,
    ]
    .iter()
    .any(|amount| *amount < 0)
    {
        return Err(PayoutPolicyError::NegativeAmount);
    }
    if input.platform_fee_bps > 10_000 || input.reserve_bps > 10_000 {
        return Err(PayoutPolicyError::InvalidBasisPoints);
    }
    let deductions = input
        .tax_amount
        .saturating_add(input.provider_fee_amount)
        .saturating_add(input.refund_amount)
        .saturating_add(input.chargeback_amount);
    if deductions > input.gross_amount {
        return Err(PayoutPolicyError::DeductionsExceedGross);
    }
    let net_receipts = input.gross_amount - deductions;
    let platform_fee_amount = proportional(net_receipts, input.platform_fee_bps);
    let after_platform_fee = net_receipts - platform_fee_amount;
    let reserve_amount = proportional(after_platform_fee, input.reserve_bps);
    let developer_payable_amount = after_platform_fee - reserve_amount;
    Ok(SettlementWaterfall {
        gross_amount: input.gross_amount,
        tax_amount: input.tax_amount,
        provider_fee_amount: input.provider_fee_amount,
        refund_amount: input.refund_amount,
        chargeback_amount: input.chargeback_amount,
        net_receipts,
        platform_fee_amount,
        reserve_amount,
        developer_payable_amount,
    })
}

pub fn payout_purpose(source: SettlementSource) -> PayoutPurpose {
    match source {
        SettlementSource::WechatOrder | SettlementSource::AlipayOrder => {
            PayoutPurpose::OriginalOrderSplit
        }
        SettlementSource::AppleStoreProceeds
        | SettlementSource::GoogleStoreProceeds
        | SettlementSource::OtherExternalProceeds => PayoutPurpose::ExternalProceedsPayout,
        SettlementSource::WebMarketplace => PayoutPurpose::MarketplacePayout,
    }
}

pub fn preferred_providers(
    region: SettlementRegion,
    source: SettlementSource,
) -> &'static [PayoutProvider] {
    use PayoutProvider::*;
    match (region, source) {
        (SettlementRegion::MainlandChina, SettlementSource::WechatOrder) => &[WechatPlatform],
        (SettlementRegion::MainlandChina, SettlementSource::AlipayOrder) => &[AlipayPlatform],
        (
            SettlementRegion::MainlandChina,
            SettlementSource::AppleStoreProceeds
            | SettlementSource::GoogleStoreProceeds
            | SettlementSource::OtherExternalProceeds,
        ) => &[LianlianAccountPlus, HuifuDougong],
        (SettlementRegion::MainlandChina, SettlementSource::WebMarketplace) => {
            &[LianlianAccountPlus, HuifuDougong]
        }
        (SettlementRegion::Global, SettlementSource::WebMarketplace) => &[
            StripeConnect,
            AdyenPlatform,
            PaypalMultiparty,
            PaypalPayouts,
        ],
        (
            SettlementRegion::Global,
            SettlementSource::AppleStoreProceeds
            | SettlementSource::GoogleStoreProceeds
            | SettlementSource::OtherExternalProceeds,
        ) => &[StripeConnect, AdyenPlatform, PaypalPayouts],
        (
            SettlementRegion::Global,
            SettlementSource::WechatOrder | SettlementSource::AlipayOrder,
        ) => &[StripeConnect, AdyenPlatform, PaypalPayouts],
    }
}

pub fn select_payout_account<'a>(
    region: SettlementRegion,
    source: SettlementSource,
    currency: &str,
    accounts: &'a [EligiblePayoutAccount],
) -> Result<&'a EligiblePayoutAccount, PayoutPolicyError> {
    let purpose = payout_purpose(source);
    for provider in preferred_providers(region, source) {
        if let Some(account) = accounts.iter().find(|account| {
            account.provider == *provider
                && account.state_active
                && account.kyc_verified
                && account.payouts_enabled
                && account
                    .currencies
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(currency))
                && account.purposes.contains(&purpose)
        }) {
            return Ok(account);
        }
    }
    Err(PayoutPolicyError::NoEligibleProvider)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(
        id: &str,
        provider: PayoutProvider,
        currency: &str,
        purpose: PayoutPurpose,
    ) -> EligiblePayoutAccount {
        EligiblePayoutAccount {
            payout_account_id: id.into(),
            provider,
            state_active: true,
            kyc_verified: true,
            payouts_enabled: true,
            currencies: vec![currency.into()],
            purposes: vec![purpose],
        }
    }

    #[test]
    fn platform_fee_is_charged_on_reconciled_net_receipts() {
        let result = settlement_waterfall(SettlementWaterfallInput {
            gross_amount: 10_000,
            tax_amount: 0,
            provider_fee_amount: 1_200,
            refund_amount: 0,
            chargeback_amount: 0,
            platform_fee_bps: 500,
            reserve_bps: 0,
        })
        .unwrap();
        assert_eq!(result.net_receipts, 8_800);
        assert_eq!(result.platform_fee_amount, 440);
        assert_eq!(result.developer_payable_amount, 8_360);
    }

    #[test]
    fn reserve_is_taken_after_platform_fee() {
        let result = settlement_waterfall(SettlementWaterfallInput {
            gross_amount: 10_000,
            tax_amount: 0,
            provider_fee_amount: 1_200,
            refund_amount: 0,
            chargeback_amount: 0,
            platform_fee_bps: 500,
            reserve_bps: 1_000,
        })
        .unwrap();
        assert_eq!(result.platform_fee_amount, 440);
        assert_eq!(result.reserve_amount, 836);
        assert_eq!(result.developer_payable_amount, 7_524);
    }

    #[test]
    fn mainland_wechat_order_never_falls_through_to_mass_payout() {
        let accounts = vec![
            account(
                "ll",
                PayoutProvider::LianlianAccountPlus,
                "CNY",
                PayoutPurpose::OriginalOrderSplit,
            ),
            account(
                "wx",
                PayoutProvider::WechatPlatform,
                "CNY",
                PayoutPurpose::OriginalOrderSplit,
            ),
        ];
        let selected = select_payout_account(
            SettlementRegion::MainlandChina,
            SettlementSource::WechatOrder,
            "CNY",
            &accounts,
        )
        .unwrap();
        assert_eq!(selected.payout_account_id, "wx");
    }

    #[test]
    fn mainland_external_store_proceeds_prefer_lianlian_then_huifu() {
        let accounts = vec![
            account(
                "huifu",
                PayoutProvider::HuifuDougong,
                "CNY",
                PayoutPurpose::ExternalProceedsPayout,
            ),
            account(
                "lianlian",
                PayoutProvider::LianlianAccountPlus,
                "CNY",
                PayoutPurpose::ExternalProceedsPayout,
            ),
        ];
        let selected = select_payout_account(
            SettlementRegion::MainlandChina,
            SettlementSource::AppleStoreProceeds,
            "CNY",
            &accounts,
        )
        .unwrap();
        assert_eq!(selected.payout_account_id, "lianlian");
    }

    #[test]
    fn global_marketplace_prefers_stripe_then_adyen_then_paypal() {
        let accounts = vec![
            account(
                "adyen",
                PayoutProvider::AdyenPlatform,
                "USD",
                PayoutPurpose::MarketplacePayout,
            ),
            account(
                "stripe",
                PayoutProvider::StripeConnect,
                "USD",
                PayoutPurpose::MarketplacePayout,
            ),
        ];
        let selected = select_payout_account(
            SettlementRegion::Global,
            SettlementSource::WebMarketplace,
            "USD",
            &accounts,
        )
        .unwrap();
        assert_eq!(selected.payout_account_id, "stripe");
    }

    #[test]
    fn global_external_proceeds_can_use_paypal_payouts_without_fake_multiparty() {
        let accounts = vec![account(
            "paypal-payouts",
            PayoutProvider::PaypalPayouts,
            "USD",
            PayoutPurpose::ExternalProceedsPayout,
        )];
        let selected = select_payout_account(
            SettlementRegion::Global,
            SettlementSource::AppleStoreProceeds,
            "USD",
            &accounts,
        )
        .unwrap();
        assert_eq!(selected.payout_account_id, "paypal-payouts");
    }

    #[test]
    fn missing_kyc_or_currency_fails_closed() {
        let mut blocked = account(
            "stripe",
            PayoutProvider::StripeConnect,
            "EUR",
            PayoutPurpose::MarketplacePayout,
        );
        blocked.kyc_verified = false;
        assert_eq!(
            select_payout_account(
                SettlementRegion::Global,
                SettlementSource::WebMarketplace,
                "USD",
                &[blocked]
            ),
            Err(PayoutPolicyError::NoEligibleProvider)
        );
    }

    #[test]
    fn invalid_waterfall_is_rejected() {
        assert_eq!(
            settlement_waterfall(SettlementWaterfallInput {
                gross_amount: 100,
                tax_amount: 60,
                provider_fee_amount: 50,
                refund_amount: 0,
                chargeback_amount: 0,
                platform_fee_bps: 500,
                reserve_bps: 0,
            }),
            Err(PayoutPolicyError::DeductionsExceedGross)
        );
    }
}
