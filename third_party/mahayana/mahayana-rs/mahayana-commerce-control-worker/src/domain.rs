use serde::{Deserialize, Serialize};

pub const THIRTY_DAYS_SECONDS: i64 = 2_592_000;
const MAX_PRICE_MINOR: i64 = 100_000_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeveloperProductDraft {
    pub sku: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub product_kind: String,
    pub entitlement_capability: String,
    pub currency: String,
    pub amount: i64,
    #[serde(default)]
    pub tax_code: Option<String>,
    #[serde(default)]
    pub subscription_period_seconds: Option<i64>,
    #[serde(default)]
    pub rails: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderBindingPlan {
    pub provider: String,
    pub external_product_ref: Option<String>,
    pub generic_product_id: Option<String>,
    pub sync_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfiguration {
    pub apple_advanced_commerce_enabled: bool,
    pub apple_one_time_generic_product_id: Option<String>,
    pub apple_subscription_generic_product_id: Option<String>,
    pub google_catalog_sync_enabled: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CatalogError {
    #[error("invalid identifier")]
    InvalidIdentifier,
    #[error("invalid product kind")]
    InvalidProductKind,
    #[error("invalid currency")]
    InvalidCurrency,
    #[error("invalid price")]
    InvalidPrice,
    #[error("invalid tax code")]
    InvalidTaxCode,
    #[error("invalid subscription period")]
    InvalidSubscriptionPeriod,
    #[error("invalid payment rail")]
    InvalidRail,
    #[error("credits rail requires FBC pricing")]
    CreditsCurrencyMismatch,
}

pub fn validate_product_draft(input: &DeveloperProductDraft) -> Result<(), CatalogError> {
    if !is_identifier(&input.sku) || !is_identifier(&input.entitlement_capability) {
        return Err(CatalogError::InvalidIdentifier);
    }
    if input.display_name.trim().is_empty() || input.display_name.chars().count() > 30 {
        return Err(CatalogError::InvalidIdentifier);
    }
    if input.description.chars().count() > 45 {
        return Err(CatalogError::InvalidIdentifier);
    }
    if !matches!(
        input.product_kind.as_str(),
        "digital_consumable" | "digital_durable" | "subscription" | "physical" | "service"
    ) {
        return Err(CatalogError::InvalidProductKind);
    }
    if !is_currency(&input.currency) {
        return Err(CatalogError::InvalidCurrency);
    }
    if input.amount <= 0 || input.amount > MAX_PRICE_MINOR {
        return Err(CatalogError::InvalidPrice);
    }
    if input.product_kind == "subscription" {
        if input.subscription_period_seconds != Some(THIRTY_DAYS_SECONDS) {
            return Err(CatalogError::InvalidSubscriptionPeriod);
        }
    } else if input.subscription_period_seconds.is_some() {
        return Err(CatalogError::InvalidSubscriptionPeriod);
    }
    if let Some(code) = input.tax_code.as_deref() {
        if code.trim().is_empty()
            || code.len() > 64
            || !code
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        {
            return Err(CatalogError::InvalidTaxCode);
        }
    }
    let rails = normalized_rails(input)?;
    if rails.iter().any(|rail| rail == "credits") && input.currency != "FBC" {
        return Err(CatalogError::CreditsCurrencyMismatch);
    }
    Ok(())
}

pub fn normalized_rails(input: &DeveloperProductDraft) -> Result<Vec<String>, CatalogError> {
    let defaults: &[&str] = match input.product_kind.as_str() {
        "digital_consumable" | "digital_durable" | "subscription" => {
            &["apple_advanced_commerce", "google_play", "web_provider"]
        }
        "physical" | "service" => &["merchant_provider", "web_provider"],
        _ => return Err(CatalogError::InvalidProductKind),
    };
    let source: Vec<String> = if input.rails.is_empty() {
        defaults.iter().map(|rail| (*rail).to_string()).collect()
    } else {
        input.rails.clone()
    };
    let mut result = Vec::new();
    for rail in source {
        let normalized = match rail.trim() {
            "apple" | "apple_advanced_commerce" => "apple_advanced_commerce",
            "google" | "google_play" => "google_play",
            "web" | "web_provider" => "web_provider",
            "merchant" | "merchant_provider" => "merchant_provider",
            "credits" => "credits",
            _ => return Err(CatalogError::InvalidRail),
        };
        if !result.iter().any(|existing| existing == normalized) {
            result.push(normalized.to_string());
        }
    }
    Ok(result)
}

pub fn plan_provider_bindings(
    mini_app_id: &str,
    input: &DeveloperProductDraft,
    configuration: &ProviderConfiguration,
) -> Result<Vec<ProviderBindingPlan>, CatalogError> {
    validate_product_draft(input)?;
    if !is_identifier(mini_app_id) {
        return Err(CatalogError::InvalidIdentifier);
    }
    let mut result = Vec::new();
    for rail in normalized_rails(input)? {
        let binding = match rail.as_str() {
            "apple_advanced_commerce" => {
                let generic = if input.product_kind == "subscription" {
                    configuration.apple_subscription_generic_product_id.clone()
                } else {
                    configuration.apple_one_time_generic_product_id.clone()
                };
                let active = configuration.apple_advanced_commerce_enabled
                    && generic.is_some()
                    && input.tax_code.is_some();
                ProviderBindingPlan {
                    provider: rail,
                    external_product_ref: generic.clone(),
                    generic_product_id: generic,
                    sync_state: if active {
                        "active"
                    } else {
                        "pending_configuration"
                    }
                    .into(),
                }
            }
            "google_play" => ProviderBindingPlan {
                provider: rail,
                external_product_ref: Some(google_product_id(mini_app_id, &input.sku)),
                generic_product_id: None,
                sync_state: if configuration.google_catalog_sync_enabled {
                    "pending_sync"
                } else {
                    "pending_configuration"
                }
                .into(),
            },
            "web_provider" => ProviderBindingPlan {
                provider: rail,
                external_product_ref: Some(format!("fabushi.{mini_app_id}.{}", input.sku)),
                generic_product_id: None,
                sync_state: "active".into(),
            },
            "merchant_provider" => ProviderBindingPlan {
                provider: rail,
                external_product_ref: Some(format!("fabushi.merchant.{mini_app_id}.{}", input.sku)),
                generic_product_id: None,
                sync_state: "active".into(),
            },
            "credits" => ProviderBindingPlan {
                provider: rail,
                external_product_ref: None,
                generic_product_id: None,
                sync_state: "active".into(),
            },
            _ => return Err(CatalogError::InvalidRail),
        };
        result.push(binding);
    }
    Ok(result)
}

pub fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

pub fn is_currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|b| b.is_ascii_uppercase())
}

pub fn google_product_id(mini_app_id: &str, sku: &str) -> String {
    let mut value: String = format!("{}.{}", mini_app_id, sku)
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    value.truncate(128);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monthly() -> DeveloperProductDraft {
        DeveloperProductDraft {
            sku: "prayer-wheel.monthly".into(),
            display_name: "Prayer Wheel Monthly".into(),
            description: "30 day access".into(),
            product_kind: "subscription".into(),
            entitlement_capability: "local-prayer-wheel".into(),
            currency: "CNY".into(),
            amount: 3000,
            tax_code: Some("C003-00-1".into()),
            subscription_period_seconds: Some(THIRTY_DAYS_SECONDS),
            rails: vec![],
        }
    }

    #[test]
    fn digital_fiat_defaults_to_store_and_web() {
        assert_eq!(
            normalized_rails(&monthly()).unwrap(),
            vec!["apple_advanced_commerce", "google_play", "web_provider"]
        );
    }

    #[test]
    fn developer_has_no_platform_fee_or_owner_authority() {
        let value = serde_json::to_value(monthly()).unwrap();
        assert!(value.get("developerId").is_none());
        assert!(value.get("ownerUserId").is_none());
        assert!(value.get("platformFeeBps").is_none());
    }

    #[test]
    fn credits_are_optional_not_a_fiat_intermediary() {
        let mut input = monthly();
        input.rails = vec!["credits".into()];
        assert_eq!(
            validate_product_draft(&input),
            Err(CatalogError::CreditsCurrencyMismatch)
        );
    }

    #[test]
    fn apple_fails_closed_without_program_configuration_or_tax_code() {
        let mut input = monthly();
        input.tax_code = None;
        let plans = plan_provider_bindings(
            "global-dharma",
            &input,
            &ProviderConfiguration {
                apple_advanced_commerce_enabled: true,
                apple_one_time_generic_product_id: Some(
                    "com.ombhrum.fabushi.miniapp.onetime".into(),
                ),
                apple_subscription_generic_product_id: Some(
                    "com.ombhrum.fabushi.miniapp.subscription".into(),
                ),
                google_catalog_sync_enabled: false,
            },
        )
        .unwrap();
        assert_eq!(plans[0].sync_state, "pending_configuration");
    }
}
