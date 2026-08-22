use mahayana_mcp_runtime::NativeMcpRegistry;
use mahayana_platform_core::HostPlatform;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("mahayana-mcp-cli-{}", Uuid::new_v4()));
    let plugin = root.join("plugins/example");
    fs::create_dir_all(plugin.join(".mahayana-plugin")).expect("create manifest dir");
    fs::write(
        plugin.join(".mahayana-plugin/plugin.json"),
        r#"{
          "name":"example",
          "mcpServers":"./.mcp.json",
          "runtimeVariants":[
            {"server":"local","platforms":["cli","desktop"],"priority":100}
          ]
        }"#,
    )
    .expect("write manifest");
    fs::write(
        plugin.join(".mcp.json"),
        r#"{"mcpServers":{"local":{"type":"stdio","command":"./server","args":[],"cwd":"."}}}"#,
    )
    .expect("write MCP config");
    fs::write(plugin.join("server"), "fixture").expect("write server fixture");
    root
}

#[test]
fn cli_selects_cli_compatible_runtime_variant() {
    let root = fixture();
    let registry = NativeMcpRegistry::new([root.join("plugins")], None);
    let resolved = registry
        .resolve_plugin("example", HostPlatform::Cli)
        .expect("resolve CLI runtime");
    assert_eq!(resolved.server_name, "local");
    fs::remove_dir_all(root).expect("cleanup");
}
