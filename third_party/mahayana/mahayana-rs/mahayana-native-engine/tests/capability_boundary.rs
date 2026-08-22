use async_trait::async_trait;
use mahayana_kernel::{Capability, EngineBackend};
use mahayana_model::{
    ModelError, ModelEvent, ModelProviderMode, ModelRequest, ModelRuntime, SharedModelEventSink,
};
use mahayana_native_engine::{NativeEngine, NativeEngineConfig};
use serde_json::json;
use std::sync::Arc;

struct LocalModel;

#[async_trait]
impl ModelRuntime for LocalModel {
    async fn infer(
        &self,
        _request: ModelRequest,
        events: SharedModelEventSink,
    ) -> Result<(), ModelError> {
        events.emit(ModelEvent::Completed {
            output: json!({
                "output": [{
                    "type": "message",
                    "content": [{"type": "output_text", "text": "ok"}]
                }]
            }),
        })
    }

    fn provider_mode(&self) -> ModelProviderMode {
        ModelProviderMode::LocalModel
    }
}

#[test]
fn local_embedded_engine_does_not_advertise_network_process_or_git() {
    let engine = NativeEngine::new(Arc::new(LocalModel), NativeEngineConfig::embedded("local"))
        .expect("create native engine");
    let descriptor = engine.descriptor();

    assert!(descriptor.native);
    assert_eq!(descriptor.id, "mahayana-native");
    assert!(descriptor.capabilities.contains(Capability::Model));
    assert!(descriptor.capabilities.contains(Capability::Workspace));
    assert!(!descriptor.capabilities.contains(Capability::Network));
    assert!(!descriptor.capabilities.contains(Capability::Process));
    assert!(!descriptor.capabilities.contains(Capability::Git));
}

#[test]
fn desktop_engine_advertises_local_process_and_git_without_remote_network() {
    let engine = NativeEngine::new(Arc::new(LocalModel), NativeEngineConfig::desktop("local"))
        .expect("create native engine");
    let descriptor = engine.descriptor();

    assert!(descriptor.capabilities.contains(Capability::Process));
    assert!(descriptor.capabilities.contains(Capability::Git));
    assert!(!descriptor.capabilities.contains(Capability::Network));
}
