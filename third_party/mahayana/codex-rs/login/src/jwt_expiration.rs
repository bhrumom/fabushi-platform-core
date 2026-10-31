use base64::Engine;
use chrono::DateTime;
use chrono::Utc;
use serde::Deserialize;
use thiserror::Error;

#[derive(Deserialize)]
struct StandardJwtClaims {
    #[serde(default)]
    exp: Option<i64>,
}

#[derive(Debug, Error)]
pub enum IdTokenInfoError {
    #[error("invalid ID token format")]
    InvalidFormat,
    #[error(transparent)]
    Base64(#[from] base64::DecodeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn decode_jwt_payload(jwt: &str) -> Result<StandardJwtClaims, IdTokenInfoError> {
    let mut parts = jwt.split('.');
    let (_header_b64, payload_b64, _signature_b64) =
        match (parts.next(), parts.next(), parts.next()) {
            (Some(header), Some(payload), Some(signature))
                if !header.is_empty() && !payload.is_empty() && !signature.is_empty() =>
            {
                (header, payload, signature)
            }
            _ => return Err(IdTokenInfoError::InvalidFormat),
        };
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload_b64)?;
    Ok(serde_json::from_slice(&payload)?)
}

pub fn parse_jwt_expiration(jwt: &str) -> Result<Option<DateTime<Utc>>, IdTokenInfoError> {
    let claims = decode_jwt_payload(jwt)?;
    Ok(claims
        .exp
        .and_then(|expiration| DateTime::<Utc>::from_timestamp(expiration, 0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;

    fn token(payload: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{\"alg\":\"none\"}"#);
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        format!("{header}.{payload}.signature")
    }

    #[test]
    fn parses_expiration_without_loading_full_login_stack() {
        let expiration = parse_jwt_expiration(&token(json!({"exp": 1_800_000_000})))
            .unwrap()
            .unwrap();
        assert_eq!(expiration.timestamp(), 1_800_000_000);
    }

    #[test]
    fn accepts_token_without_expiration() {
        assert_eq!(parse_jwt_expiration(&token(json!({}))).unwrap(), None);
    }

    #[test]
    fn rejects_malformed_tokens() {
        assert!(matches!(
            parse_jwt_expiration("not-a-jwt"),
            Err(IdTokenInfoError::InvalidFormat)
        ));
    }
}
