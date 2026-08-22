//! Single product composition for the Rust-native Mahayana Harness.

use mahayana_core::BuildProfile;
use mahayana_harness::MahayanaHarness;
use mahayana_harness_services::HarnessServices;

use crate::HarnessFeatureController;

#[derive(Clone)]
pub struct ProductHarness {
    pub core: HarnessFeatureController,
    pub services: HarnessServices,
}

impl ProductHarness {
    pub fn new(build_profile: BuildProfile) -> Self {
        let harness = MahayanaHarness::new(build_profile);
        Self {
            core: HarnessFeatureController::from_harness(harness.clone()),
            services: HarnessServices::new(harness),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_and_services_share_one_runtime_state() {
        let product = ProductHarness::new(BuildProfile::DesktopFull);
        let session = product.core.create_session("shared").unwrap();
        product
            .core
            .append_session_event(
                &session.id,
                "user/message",
                serde_json::json!({"text": "shared needle"}),
            )
            .unwrap();
        let hits = product.services.search_sessions("needle", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, session.id);
    }
}
