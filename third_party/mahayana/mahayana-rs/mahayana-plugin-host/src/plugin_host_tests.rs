use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::path::PathBuf;

#[test]
fn runtime_selection_uses_mahayana_platform_priority() {
    let variants = vec![
        PluginRuntimeVariant {
            id: "http".into(),
            server: "remote".into(),
            platforms: vec![HostPlatform::Cli, HostPlatform::Desktop],
            priority: 10,
        },
        PluginRuntimeVariant {
            id: "local".into(),
            server: "local".into(),
            platforms: vec![HostPlatform::Cli],
            priority: 20,
        },
    ];

    assert_eq!(
        select_runtime(
            HostPlatform::Cli,
            &["local".into(), "remote".into()],
            &variants,
        )
        .expect("select runtime"),
        SelectedRuntime {
            variant_id: Some("local".into()),
            server: "local".into(),
        }
    );
}

#[test]
fn runtime_selection_falls_back_when_bundled_cli_is_unavailable() {
    let variants = vec![
        PluginRuntimeVariant {
            id: "account-http".into(),
            server: "remote".into(),
            platforms: vec![HostPlatform::Desktop],
            priority: 100,
        },
        PluginRuntimeVariant {
            id: "local-cli".into(),
            server: "local".into(),
            platforms: vec![HostPlatform::Desktop],
            priority: 300,
        },
    ];

    assert_eq!(
        select_runtime_with_availability(
            HostPlatform::Desktop,
            &["local".into(), "remote".into()],
            &variants,
            |server| server == "remote",
        )
        .expect("select fallback runtime"),
        SelectedRuntime {
            variant_id: Some("account-http".into()),
            server: "remote".into(),
        }
    );
}

#[test]
fn namespaced_tui_command_keeps_json_arguments() {
    assert_eq!(
        PluginCommandInvocation::parse_tui(r#"/weather:forecast {"city":"北京"}"#)
            .expect("parse command"),
        PluginCommandInvocation {
            plugin_id: "weather".into(),
            command: "forecast".into(),
            arguments: json!({"city": "北京"}),
        }
    );
}

#[test]
fn official_plugins_use_legacy_manifest_and_mahayana_extension_together() {
    let plugins_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../.agents/plugins/plugins");

    // Repository-level plugin fixtures are integration fixtures, not runtime
    // dependencies. A native-only vendor-isolation workspace intentionally does
    // not contain them and the crate must remain independently testable there.
    if !plugins_root.is_dir() {
        return;
    }

    // Validate the stable official Mahayana plugin baseline explicitly instead
    // of scanning every project/plugin fixture in the shared repository. Other
    // projects may add manifests with capabilities outside this host contract.
    let expected = [
        "bot-father",
        "chatgpt-auto-confirm",
        "computer-cleaner",
        "faliu-flashcards",
        "global-dharma",
        "hermes-installer",
        "mahayana-assistant",
        "platform-publish",
    ];

    for name in expected {
        let path = plugins_root.join(name);
        assert!(path.is_dir(), "missing official plugin fixture at {path:?}");
        let plugin = LocalPlugin::load(&path).expect("valid combined plugin manifests");
        assert!(plugin.mahayana.is_some(), "missing extension at {path:?}");
        assert_eq!(plugin.legacy.name, name);
    }
}
