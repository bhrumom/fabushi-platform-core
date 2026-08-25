#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntitlementAccessInput<'a> {
    pub status: &'a str,
    pub product_kind: Option<&'a str>,
    pub granted_at: i64,
    pub entitlement_expires_at: Option<i64>,
    pub subscription_status: Option<&'a str>,
    pub subscription_period_end: Option<i64>,
    pub subscription_period_seconds: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct EntitlementAccessDecision {
    pub allowed: bool,
    pub effective_expires_at: Option<i64>,
    pub reason: &'static str,
}

pub(crate) fn evaluate_entitlement_access(
    input: EntitlementAccessInput<'_>,
    now: i64,
) -> EntitlementAccessDecision {
    if input.status != "active" {
        return EntitlementAccessDecision {
            allowed: false,
            effective_expires_at: input.entitlement_expires_at,
            reason: "entitlement_inactive",
        };
    }

    if input.product_kind == Some("subscription") {
        if input
            .subscription_status
            .is_some_and(|status| !matches!(status, "trialing" | "active" | "past_due"))
        {
            return EntitlementAccessDecision {
                allowed: false,
                effective_expires_at: input
                    .subscription_period_end
                    .or(input.entitlement_expires_at),
                reason: "subscription_inactive",
            };
        }

        let effective_expires_at = input
            .subscription_period_end
            .or(input.entitlement_expires_at)
            .or_else(|| {
                input
                    .subscription_period_seconds
                    .filter(|seconds| *seconds > 0)
                    .and_then(|seconds| input.granted_at.checked_add(seconds))
            });
        let Some(expires_at) = effective_expires_at else {
            return EntitlementAccessDecision {
                allowed: false,
                effective_expires_at: None,
                reason: "subscription_expiry_unknown",
            };
        };
        if expires_at <= now {
            return EntitlementAccessDecision {
                allowed: false,
                effective_expires_at: Some(expires_at),
                reason: "subscription_expired",
            };
        }
        return EntitlementAccessDecision {
            allowed: true,
            effective_expires_at: Some(expires_at),
            reason: "active_subscription",
        };
    }

    if input
        .entitlement_expires_at
        .is_some_and(|expires_at| expires_at <= now)
    {
        return EntitlementAccessDecision {
            allowed: false,
            effective_expires_at: input.entitlement_expires_at,
            reason: "entitlement_expired",
        };
    }

    EntitlementAccessDecision {
        allowed: true,
        effective_expires_at: input.entitlement_expires_at,
        reason: if input.entitlement_expires_at.is_some() {
            "active_timed_entitlement"
        } else {
            "active_durable_entitlement"
        },
    }
}

pub(crate) fn active_purchase_rails(
    allowed_rails: &[String],
    active_providers_csv: &str,
) -> Vec<String> {
    let mut rails = Vec::new();
    for provider in active_providers_csv
        .split(',')
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
    {
        let rail = match provider {
            "apple_advanced_commerce" => "apple_in_app_purchase",
            "google_play" => "google_play_billing",
            "web_provider" => "web_provider",
            "merchant_provider" => "merchant_provider",
            "credits" => "credits",
            _ => continue,
        };
        if allowed_rails.iter().any(|allowed| allowed == rail)
            && !rails.iter().any(|existing| existing == rail)
        {
            rails.push(rail.to_string());
        }
    }
    rails
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn subscription() -> EntitlementAccessInput<'static> {
        EntitlementAccessInput {
            status: "active",
            product_kind: Some("subscription"),
            granted_at: NOW - 100,
            entitlement_expires_at: None,
            subscription_status: Some("active"),
            subscription_period_end: None,
            subscription_period_seconds: Some(2_592_000),
        }
    }

    #[test]
    fn durable_without_expiry_is_active() {
        let decision = evaluate_entitlement_access(
            EntitlementAccessInput {
                status: "active",
                product_kind: Some("digital_durable"),
                granted_at: NOW - 10,
                entitlement_expires_at: None,
                subscription_status: None,
                subscription_period_end: None,
                subscription_period_seconds: None,
            },
            NOW,
        );
        assert!(decision.allowed);
        assert_eq!(decision.effective_expires_at, None);
    }

    #[test]
    fn subscription_uses_provider_period_end_first() {
        let mut input = subscription();
        input.subscription_period_end = Some(NOW + 50);
        input.entitlement_expires_at = Some(NOW + 10);
        let decision = evaluate_entitlement_access(input, NOW);
        assert!(decision.allowed);
        assert_eq!(decision.effective_expires_at, Some(NOW + 50));
    }

    #[test]
    fn subscription_falls_back_to_catalog_period() {
        let decision = evaluate_entitlement_access(subscription(), NOW);
        assert!(decision.allowed);
        assert_eq!(decision.effective_expires_at, Some(NOW - 100 + 2_592_000));
    }

    #[test]
    fn expired_catalog_period_fails_closed() {
        let mut input = subscription();
        input.granted_at = NOW - 2_592_001;
        let decision = evaluate_entitlement_access(input, NOW);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "subscription_expired");
    }

    #[test]
    fn subscription_without_any_expiry_fails_closed() {
        let mut input = subscription();
        input.subscription_period_seconds = None;
        let decision = evaluate_entitlement_access(input, NOW);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "subscription_expiry_unknown");
    }

    #[test]
    fn cancelled_subscription_is_not_active_even_inside_period() {
        let mut input = subscription();
        input.subscription_status = Some("cancelled");
        input.subscription_period_end = Some(NOW + 1_000);
        let decision = evaluate_entitlement_access(input, NOW);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "subscription_inactive");
    }

    #[test]
    fn revoked_entitlement_is_not_active() {
        let decision = evaluate_entitlement_access(
            EntitlementAccessInput {
                status: "revoked",
                product_kind: Some("digital_durable"),
                granted_at: NOW - 10,
                entitlement_expires_at: None,
                subscription_status: None,
                subscription_period_end: None,
                subscription_period_seconds: None,
            },
            NOW,
        );
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "entitlement_inactive");
    }

    #[test]
    fn purchase_rails_require_active_provider_binding() {
        let allowed = vec![
            "apple_in_app_purchase".to_string(),
            "google_play_billing".to_string(),
            "web_provider".to_string(),
        ];
        assert_eq!(
            active_purchase_rails(&allowed, "web_provider"),
            vec!["web_provider".to_string()]
        );
        assert_eq!(
            active_purchase_rails(&allowed, "apple_advanced_commerce,google_play,web_provider"),
            allowed
        );
    }
}
