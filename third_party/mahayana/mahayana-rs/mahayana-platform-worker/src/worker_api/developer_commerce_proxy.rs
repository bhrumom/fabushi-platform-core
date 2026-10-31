use super::*;
use wasm_bindgen::JsValue;
use worker::{Headers, RequestInit};

const DEFAULT_COMMERCE_CONTROL_BASE_URL: &str = "https://commerce.ombhrum.com";

fn commerce_control_base_url(env: &Env) -> Result<String> {
    let value = env
        .var("COMMERCE_CONTROL_BASE_URL")
        .ok()
        .map(|value| value.to_string())
        .unwrap_or_else(|| DEFAULT_COMMERCE_CONTROL_BASE_URL.to_string());
    let parsed = Url::parse(value.trim_end_matches('/'))
        .map_err(|_| worker::Error::RustError("invalid COMMERCE_CONTROL_BASE_URL".into()))?;
    if parsed.scheme() != "https:"
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(worker::Error::RustError(
            "COMMERCE_CONTROL_BASE_URL must be a clean HTTPS origin".into(),
        ));
    }
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn allowed_commerce_proxy_path(path: &str) -> bool {
    path == "/v1/developer/commerce/profile"
        || path == "/v1/developer/commerce/miniapps"
        || path.starts_with("/v1/developer/commerce/miniapps/")
        || path == "/v1/developer/commerce/payout"
        || path.starts_with("/v1/developer/commerce/payout/")
        || (path.starts_with("/v1/pay/intents/") && path.ends_with("/apple/advanced-commerce"))
}

pub async fn developer_commerce_proxy(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let incoming = request.url()?;
    if !allowed_commerce_proxy_path(incoming.path()) {
        return error_response(404, "not_found", "unsupported Developer Commerce route");
    }

    let authorization = request
        .headers()
        .get("Authorization")?
        .filter(|value| value.starts_with("Bearer "))
        .ok_or_else(|| {
            worker::Error::RustError("missing Developer Commerce bearer token".into())
        })?;

    let mut target = format!(
        "{}{}",
        commerce_control_base_url(&context.env)?,
        incoming.path()
    );
    if let Some(query) = incoming.query() {
        target.push('?');
        target.push_str(query);
    }

    let headers = Headers::new();
    headers.set("Authorization", &authorization)?;
    if let Some(content_type) = request.headers().get("Content-Type")? {
        headers.set("Content-Type", &content_type)?;
    }
    if let Some(idempotency_key) = request.headers().get("Idempotency-Key")? {
        headers.set("Idempotency-Key", &idempotency_key)?;
    }

    let method = request.method();
    let body = if matches!(method, Method::Get | Method::Head) {
        None
    } else {
        let bytes = request.bytes().await?;
        Some(String::from_utf8(bytes).map_err(|_| {
            worker::Error::RustError(
                "Developer Commerce proxy accepts UTF-8 request bodies only".into(),
            )
        })?)
    };
    let mut init = RequestInit::new();
    init.with_method(method).with_headers(headers);
    if let Some(body) = body {
        init.with_body(Some(JsValue::from_str(&body)));
    }
    let outbound = Request::new_with_init(&target, &init)?;
    let response = Fetch::Request(outbound).send().await?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::allowed_commerce_proxy_path;

    #[test]
    fn proxy_surface_is_narrow() {
        assert!(allowed_commerce_proxy_path(
            "/v1/developer/commerce/profile"
        ));
        assert!(allowed_commerce_proxy_path(
            "/v1/developer/commerce/miniapps/demo/products"
        ));
        assert!(allowed_commerce_proxy_path("/v1/developer/commerce/payout"));
        assert!(allowed_commerce_proxy_path(
            "/v1/developer/commerce/payout/request"
        ));
        assert!(allowed_commerce_proxy_path(
            "/v1/pay/intents/pay-1/apple/advanced-commerce"
        ));
        assert!(!allowed_commerce_proxy_path("/v1/pay/admin/products"));
        assert!(!allowed_commerce_proxy_path("/v1/pay/admin/payouts"));
        assert!(!allowed_commerce_proxy_path("/api/auth/user-info"));
    }
}
