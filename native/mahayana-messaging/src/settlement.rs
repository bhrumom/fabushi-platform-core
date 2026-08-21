use crate::actor::ActorId;
use crate::payment::Money;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;

pub const DEFAULT_SETTLEMENT_CLOCK_SKEW_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettlementEvent {
    pub provider_id: String,
    pub event_id: String,
    pub actor_id: ActorId,
    pub amount: Money,
    pub provider_reference: String,
    pub occurred_at_ms: i64,
}

impl SettlementEvent {
    pub fn validate(&self) -> Result<(), SettlementError> {
        if self.provider_id.trim().is_empty()
            || self.event_id.trim().is_empty()
            || self.provider_reference.trim().is_empty()
            || !self.actor_id.is_valid()
            || !self.amount.is_valid()
            || self.amount.amount_minor <= 0
            || self.occurred_at_ms <= 0
        {
            return Err(SettlementError::InvalidEvent);
        }
        Ok(())
    }

    pub fn idempotency_key(&self) -> String {
        format!("settlement:{}:{}", self.provider_id, self.event_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedSettlement {
    pub timestamp_ms: i64,
    pub payload_base64: String,
    pub signature_hex: String,
}

#[derive(Clone)]
pub struct SettlementVerifier {
    secret: Vec<u8>,
    clock_skew_ms: i64,
}

impl std::fmt::Debug for SettlementVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SettlementVerifier")
            .field("secret", &"[REDACTED]")
            .field("clock_skew_ms", &self.clock_skew_ms)
            .finish()
    }
}

impl SettlementVerifier {
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self, SettlementError> {
        Self::with_clock_skew(secret, DEFAULT_SETTLEMENT_CLOCK_SKEW_MS)
    }

    pub fn with_clock_skew(
        secret: impl AsRef<[u8]>,
        clock_skew_ms: i64,
    ) -> Result<Self, SettlementError> {
        let secret = secret.as_ref();
        if secret.len() < 32 {
            return Err(SettlementError::WeakSecret);
        }
        if clock_skew_ms <= 0 || clock_skew_ms > 60 * 60 * 1000 {
            return Err(SettlementError::InvalidClockSkew);
        }
        Ok(Self {
            secret: secret.to_vec(),
            clock_skew_ms,
        })
    }

    pub fn verify(
        &self,
        signed: &SignedSettlement,
        now_ms: i64,
    ) -> Result<SettlementEvent, SettlementError> {
        let age = now_ms
            .checked_sub(signed.timestamp_ms)
            .ok_or(SettlementError::TimestampOutOfRange)?
            .abs();
        if age > self.clock_skew_ms {
            return Err(SettlementError::TimestampOutOfRange);
        }
        let signature = decode_hex(&signed.signature_hex)?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret)
            .map_err(|_| SettlementError::WeakSecret)?;
        mac.update(signed.timestamp_ms.to_string().as_bytes());
        mac.update(b".");
        mac.update(signed.payload_base64.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| SettlementError::InvalidSignature)?;
        let payload = URL_SAFE_NO_PAD
            .decode(signed.payload_base64.as_bytes())
            .map_err(|_| SettlementError::InvalidEncoding)?;
        let event: SettlementEvent = serde_json::from_slice(&payload)?;
        event.validate()?;
        Ok(event)
    }

    #[cfg(test)]
    fn sign(&self, event: &SettlementEvent, timestamp_ms: i64) -> SignedSettlement {
        let payload_base64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(event).unwrap());
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.secret).unwrap();
        mac.update(timestamp_ms.to_string().as_bytes());
        mac.update(b".");
        mac.update(payload_base64.as_bytes());
        let signature_hex = encode_hex(&mac.finalize().into_bytes());
        SignedSettlement {
            timestamp_ms,
            payload_base64,
            signature_hex,
        }
    }
}

impl Drop for SettlementVerifier {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

fn decode_hex(value: &str) -> Result<Vec<u8>, SettlementError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SettlementError::InvalidEncoding);
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| SettlementError::InvalidEncoding)
        })
        .collect()
}

#[cfg(test)]
fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Error)]
pub enum SettlementError {
    #[error("settlement webhook secret must contain at least 32 bytes")]
    WeakSecret,
    #[error("settlement webhook clock skew is invalid")]
    InvalidClockSkew,
    #[error("settlement webhook timestamp is outside the replay window")]
    TimestampOutOfRange,
    #[error("settlement webhook signature is invalid")]
    InvalidSignature,
    #[error("settlement webhook encoding is invalid")]
    InvalidEncoding,
    #[error("settlement webhook event is invalid")]
    InvalidEvent,
    #[error("settlement webhook JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_settlement_detects_tamper_and_stale_replay() {
        let verifier = SettlementVerifier::new([7u8; 32]).unwrap();
        let event = SettlementEvent {
            provider_id: "processor:1".into(),
            event_id: "evt:1".into(),
            actor_id: ActorId::new("human:buyer"),
            amount: Money::new("USD", 500),
            provider_reference: "pi:1".into(),
            occurred_at_ms: 10_000,
        };
        let signed = verifier.sign(&event, 10_000);
        assert_eq!(verifier.verify(&signed, 10_001).unwrap(), event);
        let mut tampered = signed.clone();
        tampered.payload_base64.push('A');
        assert!(matches!(
            verifier.verify(&tampered, 10_001),
            Err(SettlementError::InvalidSignature)
        ));
        assert!(matches!(
            verifier.verify(&signed, 10_000 + DEFAULT_SETTLEMENT_CLOCK_SKEW_MS + 1),
            Err(SettlementError::TimestampOutOfRange)
        ));
    }
}
