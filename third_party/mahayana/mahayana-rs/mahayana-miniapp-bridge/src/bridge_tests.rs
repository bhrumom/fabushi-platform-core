use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

struct TestHost;

impl HostBridge for TestHost {
    fn invoke(&self, method: &str, params: Value) -> Result<Value, BridgeError> {
        Ok(json!({"method": method, "params": params}))
    }
}

#[test]
fn routes_allowed_mcp_calls() {
    let router = BridgeRouter::new(TestHost, [HostPermission::McpCall]);
    let response = router.handle(BridgeRequest::new(
        1,
        method::MCP_CALL_TOOL,
        json!({"tool": "forecast"}),
    ));

    assert_eq!(
        response,
        BridgeResponse {
            jsonrpc: "2.0".into(),
            id: json!(1),
            result: Some(json!({
                "method": "mcp.callTool",
                "params": {"tool": "forecast"}
            })),
            error: None,
        }
    );
}

#[test]
fn rejects_payment_calls_without_permission() {
    let router = BridgeRouter::new(TestHost, []);
    for payment_method in [
        method::COMMERCE_PURCHASE,
        method::PAY_CREATE_INTENT,
        method::PAY_OPEN_CHECKOUT,
        method::PAY_GET_STATUS,
    ] {
        let response = router.handle(BridgeRequest::new(
            "request-1",
            payment_method,
            json!({"sku": "weather.pro"}),
        ));
        assert_eq!(response.result, None);
        assert_eq!(response.error.expect("permission error").code, -32001);
    }
}

#[test]
fn payment_methods_route_only_after_commerce_permission() {
    let router = BridgeRouter::new(TestHost, [HostPermission::CommercePurchase]);
    let response = router.handle(BridgeRequest::new(
        "checkout-1",
        method::PAY_CREATE_INTENT,
        json!({"sku": "weather.pro", "idempotencyKey": "checkout-1"}),
    ));

    assert_eq!(response.error, None);
    assert_eq!(
        response.result,
        Some(json!({
            "method": "pay.createIntent",
            "params": {"sku": "weather.pro", "idempotencyKey": "checkout-1"}
        }))
    );
}
