//! Mahayana-owned authentication primitives.
//!
//! This crate intentionally contains no Codex product types. It owns the small
//! JWT parsing surface required by Fabushi account-session refresh logic so the
//! default Mahayana product graph does not need the upstream Codex login crate.

pub mod token_data {
    use base64::Engine as _;
    use chrono::{DateTime, Utc};
    use serde::Deserialize;
    use serde::de::DeserializeOwned;
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum TokenDataError {
        #[error("invalid JWT format")]
        InvalidFormat,
        #[error(transparent)]
        Base64(#[from] base64::DecodeError),
        #[error(transparent)]
        Json(#[from] serde_json::Error),
    }

    #[derive(Debug, Deserialize)]
    struct StandardJwtClaims {
        #[serde(default)]
        exp: Option<i64>,
    }

    fn decode_jwt_payload<T: DeserializeOwned>(jwt: &str) -> Result<T, TokenDataError> {
        let mut parts = jwt.split('.');
        let (_header, payload, _signature) = match (parts.next(), parts.next(), parts.next()) {
            (Some(header), Some(payload), Some(signature))
                if !header.is_empty()
                    && !payload.is_empty()
                    && !signature.is_empty()
                    && parts.next().is_none() =>
            {
                (header, payload, signature)
            }
            _ => return Err(TokenDataError::InvalidFormat),
        };
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload)?;
        Ok(serde_json::from_slice(&payload)?)
    }

    /// Parses the standard JWT `exp` claim without accepting or depending on
    /// any provider-specific identity claims.
    pub fn parse_jwt_expiration(jwt: &str) -> Result<Option<DateTime<Utc>>, TokenDataError> {
        let claims: StandardJwtClaims = decode_jwt_payload(jwt)?;
        Ok(claims
            .exp
            .and_then(|expiration| DateTime::<Utc>::from_timestamp(expiration, 0)))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn jwt(payload: serde_json::Value) -> String {
            let header =
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
            let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&payload).expect("serialize JWT payload"));
            format!("{header}.{payload}.signature")
        }

        #[test]
        fn parses_expiration_without_provider_claims() {
            let token = jwt(serde_json::json!({"exp": 1_800_000_000_i64, "vendor": "ignored"}));
            let expiration = parse_jwt_expiration(&token)
                .expect("valid token")
                .expect("exp claim");
            assert_eq!(expiration.timestamp(), 1_800_000_000);
        }

        #[test]
        fn accepts_token_without_expiration() {
            let token = jwt(serde_json::json!({"sub": "mahayana-user"}));
            assert_eq!(parse_jwt_expiration(&token).expect("valid token"), None);
        }

        #[test]
        fn rejects_malformed_tokens() {
            assert!(matches!(
                parse_jwt_expiration("not-a-jwt"),
                Err(TokenDataError::InvalidFormat)
            ));
            assert!(matches!(
                parse_jwt_expiration("a.b.c.d"),
                Err(TokenDataError::InvalidFormat)
            ));
        }
    }
}
