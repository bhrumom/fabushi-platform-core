use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleCatalogProduct {
    pub package_name: String,
    pub product_id: String,
    pub display_name: String,
    pub description: String,
    pub product_kind: String,
    pub currency: String,
    pub amount_minor: i64,
    #[serde(default)]
    pub product_tax_category_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleSyncRequest {
    pub method: String,
    pub url: String,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleMoney {
    pub currency_code: String,
    pub units: String,
    #[serde(default)]
    pub nanos: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleConvertedRegionPrice {
    pub region_code: String,
    pub price: GoogleMoney,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleConvertedOtherRegionsPrice {
    pub usd_price: GoogleMoney,
    pub eur_price: GoogleMoney,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleRegionsVersion {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoogleConvertedPrices {
    pub converted_region_prices: BTreeMap<String, GoogleConvertedRegionPrice>,
    pub converted_other_regions_price: GoogleConvertedOtherRegionsPrice,
    pub region_version: GoogleRegionsVersion,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GoogleCatalogError {
    #[error("invalid Google Play product")]
    InvalidProduct,
    #[error("converted Google Play prices are empty")]
    EmptyConvertedPrices,
    #[error("invalid Google Play regions version")]
    InvalidRegionsVersion,
}

pub fn build_google_price_conversion_request(
    product: &GoogleCatalogProduct,
) -> Result<GoogleSyncRequest, GoogleCatalogError> {
    validate_product(product)?;
    let mut body = serde_json::json!({
        "price": money(&product.currency, product.amount_minor)?,
    });
    if let Some(code) = product.product_tax_category_code.as_deref() {
        body["productTaxCategoryCode"] = serde_json::Value::String(code.to_string());
    }
    Ok(GoogleSyncRequest {
        method: "POST".into(),
        url: format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/pricing:convertRegionPrices",
            product.package_name
        ),
        body,
    })
}

pub fn build_google_sync_request(
    product: &GoogleCatalogProduct,
    converted: &GoogleConvertedPrices,
) -> Result<GoogleSyncRequest, GoogleCatalogError> {
    validate_product(product)?;
    if converted.converted_region_prices.is_empty() {
        return Err(GoogleCatalogError::EmptyConvertedPrices);
    }
    let region_version = converted.region_version.version.trim();
    if region_version.is_empty() || !region_version.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(GoogleCatalogError::InvalidRegionsVersion);
    }

    let mut regions = converted
        .converted_region_prices
        .iter()
        .map(|(code, value)| {
            serde_json::json!({
                "regionCode": if value.region_code.is_empty() { code } else { &value.region_code },
                "price": value.price,
            })
        })
        .collect::<Vec<_>>();
    regions.sort_by(|left, right| {
        left["regionCode"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["regionCode"].as_str().unwrap_or_default())
    });

    if product.product_kind == "subscription" {
        return Ok(GoogleSyncRequest {
            method: "POST".into(),
            url: format!(
                "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/subscriptions?productId={}&regionsVersion.version={}",
                product.package_name, product.product_id, region_version
            ),
            body: serde_json::json!({
                "packageName": product.package_name,
                "productId": product.product_id,
                "listings": [{
                    "languageCode": "en-US",
                    "title": product.display_name,
                    "benefits": [product.description],
                }],
                "basePlans": [{
                    "basePlanId": "monthly",
                    "autoRenewingBasePlanType": { "billingPeriodDuration": "P1M" },
                    "regionalConfigs": regions,
                    "otherRegionsConfig": {
                        "usdPrice": converted.converted_other_regions_price.usd_price,
                        "eurPrice": converted.converted_other_regions_price.eur_price,
                    }
                }]
            }),
        });
    }

    Ok(GoogleSyncRequest {
        method: "PATCH".into(),
        url: format!(
            "https://androidpublisher.googleapis.com/androidpublisher/v3/applications/{}/onetimeproducts/{}?updateMask=listings,purchaseOptions&regionsVersion.version={}",
            product.package_name, product.product_id, region_version
        ),
        body: serde_json::json!({
            "packageName": product.package_name,
            "productId": product.product_id,
            "listings": [{
                "languageCode": "en-US",
                "title": product.display_name,
                "description": product.description,
            }],
            "purchaseOptions": [{
                "purchaseOptionId": "buy",
                "buyOption": {},
                "regionalPricingAndAvailabilityConfigs": regions.into_iter().map(|mut value| {
                    value["availability"] = serde_json::Value::String("AVAILABLE".into());
                    value
                }).collect::<Vec<_>>(),
                "newRegionsConfig": {
                    "availability": "AVAILABLE_IF_RELEASED",
                    "usdPrice": converted.converted_other_regions_price.usd_price,
                    "eurPrice": converted.converted_other_regions_price.eur_price,
                }
            }]
        }),
    })
}

fn validate_product(product: &GoogleCatalogProduct) -> Result<(), GoogleCatalogError> {
    if product.package_name.trim().is_empty()
        || product.product_id.trim().is_empty()
        || product.display_name.trim().is_empty()
        || product.amount_minor <= 0
        || product.currency.len() != 3
    {
        return Err(GoogleCatalogError::InvalidProduct);
    }
    Ok(())
}

fn money(currency: &str, amount_minor: i64) -> Result<GoogleMoney, GoogleCatalogError> {
    let exponent = match currency {
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3_u32,
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF" | "UGX"
        | "UYI" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0_u32,
        _ => 2_u32,
    };
    let divisor = 10_i64.pow(exponent);
    let units = amount_minor / divisor;
    let remainder = amount_minor % divisor;
    let nanos = if exponent == 0 {
        0
    } else {
        (remainder * 1_000_000_000_i64 / divisor) as i32
    };
    Ok(GoogleMoney {
        currency_code: currency.to_string(),
        units: units.to_string(),
        nanos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn product(kind: &str) -> GoogleCatalogProduct {
        GoogleCatalogProduct {
            package_name: "com.ombhrum.fabushi".into(),
            product_id: "global_dharma.prayer_wheel".into(),
            display_name: "Prayer Wheel".into(),
            description: "Access".into(),
            product_kind: kind.into(),
            currency: "CNY".into(),
            amount_minor: 3000,
            product_tax_category_code: Some("C003-00-1".into()),
        }
    }

    fn converted() -> GoogleConvertedPrices {
        let mut converted_region_prices = BTreeMap::new();
        converted_region_prices.insert(
            "CN".into(),
            GoogleConvertedRegionPrice {
                region_code: "CN".into(),
                price: GoogleMoney {
                    currency_code: "CNY".into(),
                    units: "30".into(),
                    nanos: 0,
                },
            },
        );
        converted_region_prices.insert(
            "US".into(),
            GoogleConvertedRegionPrice {
                region_code: "US".into(),
                price: GoogleMoney {
                    currency_code: "USD".into(),
                    units: "4".into(),
                    nanos: 490_000_000,
                },
            },
        );
        GoogleConvertedPrices {
            converted_region_prices,
            converted_other_regions_price: GoogleConvertedOtherRegionsPrice {
                usd_price: GoogleMoney {
                    currency_code: "USD".into(),
                    units: "4".into(),
                    nanos: 490_000_000,
                },
                eur_price: GoogleMoney {
                    currency_code: "EUR".into(),
                    units: "4".into(),
                    nanos: 290_000_000,
                },
            },
            region_version: GoogleRegionsVersion {
                version: "2025/02".replace('/', ""),
            },
        }
    }

    #[test]
    fn asks_google_to_convert_base_fiat_price_for_all_regions() {
        let req = build_google_price_conversion_request(&product("digital_durable")).unwrap();
        assert_eq!(req.method, "POST");
        assert!(req.url.ends_with("/pricing:convertRegionPrices"));
        assert_eq!(req.body["price"]["currencyCode"], "CNY");
        assert_eq!(req.body["price"]["units"], "30");
    }

    #[test]
    fn one_time_catalog_contains_all_converted_regions() {
        let req = build_google_sync_request(&product("digital_durable"), &converted()).unwrap();
        assert_eq!(req.method, "PATCH");
        assert_eq!(
            req.body["purchaseOptions"][0]["regionalPricingAndAvailabilityConfigs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(req.url.contains("regionsVersion.version="));
    }

    #[test]
    fn subscription_uses_same_converted_region_set_and_regions_version() {
        let req = build_google_sync_request(&product("subscription"), &converted()).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.body["basePlans"][0]["autoRenewingBasePlanType"]["billingPeriodDuration"],
            "P1M"
        );
        assert_eq!(
            req.body["basePlans"][0]["regionalConfigs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn empty_google_conversion_response_fails_closed() {
        let mut value = converted();
        value.converted_region_prices.clear();
        assert_eq!(
            build_google_sync_request(&product("digital_durable"), &value),
            Err(GoogleCatalogError::EmptyConvertedPrices)
        );
    }
}
