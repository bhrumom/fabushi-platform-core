use crate::{
    ApprovalResolution, Capability, CapabilitySet, ExecutionPolicy, OpenSessionRequest,
    OperationId, ResumeOperationRequest, RunRequest, RuntimeProfile, SessionId, SessionSnapshot,
    SuspendOperationRequest,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Product surfaces that consume the same Mahayana public runtime contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductSurface {
    Electron,
    Ios,
    Android,
    Web,
    CliHeadless,
}

impl ProductSurface {
    pub fn runtime_profile(self) -> RuntimeProfile {
        match self {
            Self::Electron => RuntimeProfile::DesktopFull,
            Self::Ios | Self::Android => RuntimeProfile::MobileEmbedded,
            Self::Web => RuntimeProfile::WebWasm,
            Self::CliHeadless => RuntimeProfile::Headless,
        }
    }

    pub fn execution_policy(self) -> ExecutionPolicy {
        match self {
            Self::Ios | Self::Android => ExecutionPolicy::mobile_default(),
            Self::Electron | Self::Web | Self::CliHeadless => {
                ExecutionPolicy::interactive_default()
            }
        }
    }
}

/// One provider-neutral contract journey shared by every supported product surface.
///
/// Surface adapters may encode or transport these values differently, but they
/// must not substitute vendor public types for the Mahayana session, operation,
/// approval, suspend/resume, snapshot, policy, or capability contracts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceConformanceJourney {
    pub surface: ProductSurface,
    pub open: OpenSessionRequest,
    pub run: RunRequest,
    pub approval: ApprovalResolution,
    pub suspend: SuspendOperationRequest,
    pub resume: ResumeOperationRequest,
    pub status_snapshot: SessionSnapshot,
}

pub fn shared_conformance_journey(surface: ProductSurface) -> SurfaceConformanceJourney {
    let session_id = SessionId::from_string("surface-session");
    let operation_id = OperationId::from_string("surface-operation");
    let policy = surface.execution_policy();
    let required_capabilities = CapabilitySet::new([
        Capability::Model,
        Capability::Workspace,
        Capability::ToolProtocol,
        Capability::Mcp,
        Capability::Plugins,
    ]);
    let metadata = json!({"surface": surface});

    SurfaceConformanceJourney {
        surface,
        open: OpenSessionRequest {
            profile: surface.runtime_profile(),
            workspace_root: Some("/workspace".into()),
            model: Some("mahayana-default".into()),
            metadata: metadata.clone(),
        },
        run: RunRequest {
            session_id: session_id.clone(),
            operation_id: operation_id.clone(),
            input: "verify shared contract".into(),
            policy: policy.clone(),
            required_capabilities: required_capabilities.clone(),
            metadata: metadata.clone(),
        },
        approval: ApprovalResolution {
            approval_id: "surface-approval".into(),
            approved: true,
            metadata: metadata.clone(),
        },
        suspend: SuspendOperationRequest {
            operation_id: operation_id.clone(),
            reason: Some("surface handoff".into()),
            metadata: metadata.clone(),
        },
        resume: ResumeOperationRequest {
            session_id: session_id.clone(),
            operation_id,
            policy,
            required_capabilities,
            metadata: metadata.clone(),
        },
        status_snapshot: SessionSnapshot {
            session_id,
            backend_id: "mahayana-native".into(),
            state: json!({"status": "suspended"}),
            metadata,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_required_surface_uses_the_same_mahayana_contract_journey() {
        let cases = [
            (ProductSurface::Electron, RuntimeProfile::DesktopFull),
            (ProductSurface::Ios, RuntimeProfile::MobileEmbedded),
            (ProductSurface::Android, RuntimeProfile::MobileEmbedded),
            (ProductSurface::Web, RuntimeProfile::WebWasm),
            (ProductSurface::CliHeadless, RuntimeProfile::Headless),
        ];

        for (surface, profile) in cases {
            let journey = shared_conformance_journey(surface);
            assert_eq!(journey.open.profile, profile);
            assert_eq!(journey.run.session_id.as_str(), "surface-session");
            assert_eq!(journey.resume.session_id.as_str(), "surface-session");
            assert_eq!(journey.run.operation_id.as_str(), "surface-operation");
            assert_eq!(journey.suspend.operation_id.as_str(), "surface-operation");
            assert_eq!(journey.resume.operation_id.as_str(), "surface-operation");
            assert_eq!(
                journey.status_snapshot.session_id.as_str(),
                "surface-session"
            );
            assert!(
                journey
                    .run
                    .required_capabilities
                    .contains(Capability::Model)
            );
            assert!(
                journey
                    .run
                    .required_capabilities
                    .contains(Capability::Workspace)
            );
            assert!(journey.run.required_capabilities.contains(Capability::Mcp));
            assert!(
                journey
                    .run
                    .required_capabilities
                    .contains(Capability::Plugins)
            );
            assert_eq!(
                journey.run.required_capabilities,
                journey.resume.required_capabilities
            );
            let resume = serde_json::to_value(&journey.resume).expect("serialize resume request");
            assert!(resume.get("input").is_none());
        }
    }

    #[test]
    fn mobile_surfaces_share_the_fail_closed_mobile_process_policy() {
        for surface in [ProductSurface::Ios, ProductSurface::Android] {
            assert!(!shared_conformance_journey(surface).run.policy.allow_process);
        }
    }
}
