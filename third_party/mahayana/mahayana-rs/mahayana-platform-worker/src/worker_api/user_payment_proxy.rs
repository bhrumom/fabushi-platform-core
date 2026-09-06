use super::*;
use wasm_bindgen::JsValue;
use worker::{Headers, RequestInit};

const DEFAULT_FABUSHI_PAY_BASE_URL: &str = "https://pay.ombhrum.com";

fn fabushi_pay_base_url(env: &Env) -> Result<String> {
    let value = env
        .var("FABUSHI_PAY_BASE_URL")
        .ok()
        .map(|value| value.to_string())
        .unwrap_or_else(|| DEFAULT_FABUSHI_PAY_BASE_URL.to_string());
    let parsed = Url::parse(value.trim_end_matches('/'))
        .map_err(|_| worker::Error::RustError("invalid FABUSHI_PAY_BASE_URL".into()))?;
    if parsed.scheme() != "https:"
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(worker::Error::RustError(
            "FABUSHI_PAY_BASE_URL must be a clean HTTPS origin".into(),
        ));
    }
    Ok(parsed.to_string().trim_end_matches('/').to_string())
}

fn path_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn allowed_user_payment_proxy_path(method: Method, path: &str) -> bool {
    let segments = path.trim_matches('/').split('/').collect::<Vec<_>>();
    match (method, segments.as_slice()) {
        (Method::Post, ["v1", "miniapps", mini_app_id, "pay", "intents"]) => {
            path_segment(mini_app_id)
        }
        (Method::Get, ["v1", "pay", "intents", payment_id]) => path_segment(payment_id),
        (Method::Post, ["v1", "pay", "intents", payment_id, "checkout"]) => {
            path_segment(payment_id)
        }
        _ => false,
    }
}

pub async fn user_payment_proxy(
    mut request: Request,
    context: RouteContext<()>,
) -> Result<Response> {
    let _user_id = authenticated_user(&request, &context.env)?;
    let incoming = request.url()?;
    if !allowed_user_payment_proxy_path(request.method(), incoming.path()) {
        return error_response(404, "not_found", "unsupported user payment route");
    }

    let authorization = request
        .headers()
        .get("Authorization")?
        .filter(|value| value.starts_with("Bearer "))
        .ok_or_else(|| worker::Error::RustError("missing user payment bearer token".into()))?;

    let target = format!("{}{}", fabushi_pay_base_url(&context.env)?, incoming.path());
    let headers = Headers::new();
    headers.set("Authorization", &authorization)?;
    headers.set("Accept", "application/json")?;
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
            worker::Error::RustError("user payment proxy accepts UTF-8 request bodies only".into())
        })?)
    };
    let mut init = RequestInit::new();
    init.with_method(method).with_headers(headers);
    if let Some(body) = body {
        init.with_body(Some(JsValue::from_str(&body)));
    }
    Ok(Fetch::Request(Request::new_with_init(&target, &init)?)
        .send()
        .await?)
}

#[cfg(test)]
mod tests {
    use super::allowed_user_payment_proxy_path;
    use worker::Method;

    #[test]
    fn user_payment_surface_is_narrow() {
        assert!(allowed_user_payment_proxy_path(
            Method::Post,
            "/v1/miniapps/global-dharma/pay/intents"
        ));
        assert!(allowed_user_payment_proxy_path(
            Method::Get,
            "/v1/pay/intents/pay-1"
        ));
        assert!(allowed_user_payment_proxy_path(
            Method::Post,
            "/v1/pay/intents/pay-1/checkout"
        ));
        assert!(!allowed_user_payment_proxy_path(
            Method::Post,
            "/v1/pay/admin/products"
        ));
        assert!(!allowed_user_payment_proxy_path(
            Method::Post,
            "/v1/pay/intents/pay-1/apple/verify"
        ));
        assert!(!allowed_user_payment_proxy_path(
            Method::Post,
            "/v1/developer/commerce/profile"
        ));
    }
}
