use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{pkcs8::DecodePublicKey, Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const EXTENSION_TRUST_FEED_PROTOCOL: &str = "fabushi.extension-trust-feed.v1";
pub const EXTENSION_TRUST_ALGORITHM: &str = "Ed25519";
pub const EXTENSION_TRUST_MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionTrustKeyStatus {
    Active,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionTrustKey {
    pub algorithm: String,
    pub key_id: String,
    pub trust_epoch: u64,
    pub public_key: String,
    pub public_key_fingerprint: String,
    pub status: ExtensionTrustKeyStatus,
    pub created_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    pub revocation_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionTrustFeedSigner {
    pub algorithm: String,
    pub key_id: String,
    pub trust_epoch: u64,
    pub public_key_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionTrustFeedPayload {
    pub protocol: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub signer: ExtensionTrustFeedSigner,
    pub keys: Vec<ExtensionTrustKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExtensionTrustFeedEnvelope {
    pub protocol: String,
    pub algorithm: String,
    pub key_id: String,
    pub trust_epoch: u64,
    pub public_key: String,
    pub public_key_fingerprint: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub payload: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionTrustFeedError {
    UnknownProtocol,
    UnsupportedAlgorithm,
    InvalidEpoch,
    InvalidEncoding,
    InvalidPublicKey,
    InvalidFingerprint,
    InvalidSignature,
    InvalidPayload,
    InvalidValidityWindow,
    FutureDated,
    Expired,
    EnvelopePayloadMismatch,
    SignerNotActive,
    InvalidTrustKey,
    SigningKeyAbsent,
    SigningKeyRevoked,
    SigningKeyMismatch,
}

impl fmt::Display for ExtensionTrustFeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnknownProtocol => "unknown extension trust-feed protocol",
            Self::UnsupportedAlgorithm => "unsupported extension trust algorithm",
            Self::InvalidEpoch => "extension trust epoch must be positive",
            Self::InvalidEncoding => "extension trust value has invalid encoding",
            Self::InvalidPublicKey => "extension trust key is not a valid Ed25519 SPKI key",
            Self::InvalidFingerprint => "extension trust key fingerprint does not match",
            Self::InvalidSignature => "extension trust-feed signature is invalid",
            Self::InvalidPayload => "extension trust-feed payload is invalid",
            Self::InvalidValidityWindow => "extension trust-feed validity window is invalid",
            Self::FutureDated => "extension trust feed is unreasonably future-dated",
            Self::Expired => "extension trust feed is expired",
            Self::EnvelopePayloadMismatch => "extension trust envelope and payload disagree",
            Self::SignerNotActive => "extension trust-feed signer is not an active trusted key",
            Self::InvalidTrustKey => "extension trust feed contains an invalid key",
            Self::SigningKeyAbsent => "release signing key is absent from verified trust feed",
            Self::SigningKeyRevoked => "release signing key is revoked",
            Self::SigningKeyMismatch => "release signing key material does not match verified trust feed",
        };
        f.write_str(message)
    }
}

impl std::error::Error for ExtensionTrustFeedError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExtensionTrustFeed {
    pub envelope: ExtensionTrustFeedEnvelope,
    pub payload: ExtensionTrustFeedPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionReleaseSigningKeyRef<'a> {
    pub algorithm: &'a str,
    pub key_id: &'a str,
    pub trust_epoch: u64,
    pub public_key: &'a str,
    pub public_key_fingerprint: &'a str,
}

impl ExtensionTrustFeedEnvelope {
    pub fn verify(self, now_ms: i64) -> Result<VerifiedExtensionTrustFeed, ExtensionTrustFeedError> {
        if self.protocol != EXTENSION_TRUST_FEED_PROTOCOL {
            return Err(ExtensionTrustFeedError::UnknownProtocol);
        }
        if self.algorithm != EXTENSION_TRUST_ALGORITHM {
            return Err(ExtensionTrustFeedError::UnsupportedAlgorithm);
        }
        if self.trust_epoch == 0 {
            return Err(ExtensionTrustFeedError::InvalidEpoch);
        }

        let public_key_der = BASE64
            .decode(self.public_key.as_bytes())
            .map_err(|_| ExtensionTrustFeedError::InvalidEncoding)?;
        validate_key_material(
            &self.algorithm,
            &self.public_key,
            &self.public_key_fingerprint,
            Some(&public_key_der),
        )?;
        let verifying_key = VerifyingKey::from_public_key_der(&public_key_der)
            .map_err(|_| ExtensionTrustFeedError::InvalidPublicKey)?;

        let signature_bytes = BASE64
            .decode(self.signature.as_bytes())
            .map_err(|_| ExtensionTrustFeedError::InvalidEncoding)?;
        let signature = Signature::from_slice(&signature_bytes)
            .map_err(|_| ExtensionTrustFeedError::InvalidSignature)?;
        verifying_key
            .verify(self.payload.as_bytes(), &signature)
            .map_err(|_| ExtensionTrustFeedError::InvalidSignature)?;

        let payload: ExtensionTrustFeedPayload = serde_json::from_str(&self.payload)
            .map_err(|_| ExtensionTrustFeedError::InvalidPayload)?;
        validate_payload_shape(&payload)?;

        if payload.protocol != self.protocol
            || payload.issued_at_ms != self.issued_at_ms
            || payload.expires_at_ms != self.expires_at_ms
            || payload.signer.algorithm != self.algorithm
            || payload.signer.key_id != self.key_id
            || payload.signer.trust_epoch != self.trust_epoch
            || payload.signer.public_key_fingerprint != self.public_key_fingerprint
        {
            return Err(ExtensionTrustFeedError::EnvelopePayloadMismatch);
        }

        if self.expires_at_ms <= self.issued_at_ms || self.issued_at_ms < 0 {
            return Err(ExtensionTrustFeedError::InvalidValidityWindow);
        }
        if self.issued_at_ms > now_ms.saturating_add(EXTENSION_TRUST_MAX_FUTURE_SKEW_MS) {
            return Err(ExtensionTrustFeedError::FutureDated);
        }
        if now_ms > self.expires_at_ms {
            return Err(ExtensionTrustFeedError::Expired);
        }

        let signer = payload
            .keys
            .iter()
            .find(|key| key.key_id == self.key_id && key.trust_epoch == self.trust_epoch)
            .ok_or(ExtensionTrustFeedError::SignerNotActive)?;
        if signer.status != ExtensionTrustKeyStatus::Active
            || signer.algorithm != self.algorithm
            || signer.public_key != self.public_key
            || signer.public_key_fingerprint != self.public_key_fingerprint
        {
            return Err(ExtensionTrustFeedError::SignerNotActive);
        }

        Ok(VerifiedExtensionTrustFeed {
            envelope: self,
            payload,
        })
    }
}

impl VerifiedExtensionTrustFeed {
    pub fn key(&self, key_id: &str, trust_epoch: u64) -> Option<&ExtensionTrustKey> {
        self.payload
            .keys
            .iter()
            .find(|key| key.key_id == key_id && key.trust_epoch == trust_epoch)
    }

    pub fn authorize_release_signing_key(
        &self,
        release: ExtensionReleaseSigningKeyRef<'_>,
    ) -> Result<(), ExtensionTrustFeedError> {
        if release.algorithm != EXTENSION_TRUST_ALGORITHM {
            return Err(ExtensionTrustFeedError::UnsupportedAlgorithm);
        }
        if release.trust_epoch == 0 {
            return Err(ExtensionTrustFeedError::InvalidEpoch);
        }
        let key = self
            .key(release.key_id, release.trust_epoch)
            .ok_or(ExtensionTrustFeedError::SigningKeyAbsent)?;
        if key.status == ExtensionTrustKeyStatus::Revoked {
            return Err(ExtensionTrustFeedError::SigningKeyRevoked);
        }
        if key.algorithm != release.algorithm
            || key.public_key != release.public_key
            || key.public_key_fingerprint != release.public_key_fingerprint
        {
            return Err(ExtensionTrustFeedError::SigningKeyMismatch);
        }
        Ok(())
    }
}

fn validate_payload_shape(payload: &ExtensionTrustFeedPayload) -> Result<(), ExtensionTrustFeedError> {
    if payload.protocol != EXTENSION_TRUST_FEED_PROTOCOL {
        return Err(ExtensionTrustFeedError::UnknownProtocol);
    }
    if payload.signer.algorithm != EXTENSION_TRUST_ALGORITHM {
        return Err(ExtensionTrustFeedError::UnsupportedAlgorithm);
    }
    if payload.signer.trust_epoch == 0 {
        return Err(ExtensionTrustFeedError::InvalidEpoch);
    }
    if payload.expires_at_ms <= payload.issued_at_ms || payload.issued_at_ms < 0 {
        return Err(ExtensionTrustFeedError::InvalidValidityWindow);
    }
    for key in &payload.keys {
        if key.algorithm != EXTENSION_TRUST_ALGORITHM
            || key.trust_epoch == 0
            || key.created_at_ms < 0
            || (key.status == ExtensionTrustKeyStatus::Active && key.revoked_at_ms.is_some())
            || (key.status == ExtensionTrustKeyStatus::Revoked && key.revoked_at_ms.is_none())
        {
            return Err(ExtensionTrustFeedError::InvalidTrustKey);
        }
        validate_key_material(
            &key.algorithm,
            &key.public_key,
            &key.public_key_fingerprint,
            None,
        )
        .map_err(|_| ExtensionTrustFeedError::InvalidTrustKey)?;
    }
    Ok(())
}

fn validate_key_material(
    algorithm: &str,
    public_key: &str,
    expected_fingerprint: &str,
    decoded: Option<&[u8]>,
) -> Result<(), ExtensionTrustFeedError> {
    if algorithm != EXTENSION_TRUST_ALGORITHM {
        return Err(ExtensionTrustFeedError::UnsupportedAlgorithm);
    }
    let owned;
    let der = match decoded {
        Some(bytes) => bytes,
        None => {
            owned = BASE64
                .decode(public_key.as_bytes())
                .map_err(|_| ExtensionTrustFeedError::InvalidEncoding)?;
            &owned
        }
    };
    VerifyingKey::from_public_key_der(der)
        .map_err(|_| ExtensionTrustFeedError::InvalidPublicKey)?;
    let fingerprint = Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if fingerprint != expected_fingerprint {
        return Err(ExtensionTrustFeedError::InvalidFingerprint);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{pkcs8::EncodePublicKey, Signer, SigningKey};

    fn fixture(now_ms: i64) -> (ExtensionTrustFeedEnvelope, SigningKey, String, String) {
        let seed = [7_u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let der = signing_key
            .verifying_key()
            .to_public_key_der()
            .unwrap()
            .as_bytes()
            .to_vec();
        let public_key = BASE64.encode(&der);
        let fingerprint = Sha256::digest(&der)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let payload = ExtensionTrustFeedPayload {
            protocol: EXTENSION_TRUST_FEED_PROTOCOL.into(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms + 60_000,
            signer: ExtensionTrustFeedSigner {
                algorithm: EXTENSION_TRUST_ALGORITHM.into(),
                key_id: "marketplace-key-2".into(),
                trust_epoch: 2,
                public_key_fingerprint: fingerprint.clone(),
            },
            keys: vec![
                ExtensionTrustKey {
                    algorithm: EXTENSION_TRUST_ALGORITHM.into(),
                    key_id: "marketplace-key-1".into(),
                    trust_epoch: 1,
                    public_key: public_key.clone(),
                    public_key_fingerprint: fingerprint.clone(),
                    status: ExtensionTrustKeyStatus::Revoked,
                    created_at_ms: now_ms - 120_000,
                    revoked_at_ms: Some(now_ms - 1_000),
                    revocation_reason: "rotation after compromise".into(),
                },
                ExtensionTrustKey {
                    algorithm: EXTENSION_TRUST_ALGORITHM.into(),
                    key_id: "marketplace-key-2".into(),
                    trust_epoch: 2,
                    public_key: public_key.clone(),
                    public_key_fingerprint: fingerprint.clone(),
                    status: ExtensionTrustKeyStatus::Active,
                    created_at_ms: now_ms - 2_000,
                    revoked_at_ms: None,
                    revocation_reason: String::new(),
                },
            ],
        };
        let payload_json = serde_json::to_string(&payload).unwrap();
        let signature = signing_key.sign(payload_json.as_bytes());
        let envelope = ExtensionTrustFeedEnvelope {
            protocol: EXTENSION_TRUST_FEED_PROTOCOL.into(),
            algorithm: EXTENSION_TRUST_ALGORITHM.into(),
            key_id: "marketplace-key-2".into(),
            trust_epoch: 2,
            public_key: public_key.clone(),
            public_key_fingerprint: fingerprint.clone(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms + 60_000,
            payload: payload_json,
            signature: BASE64.encode(signature.to_bytes()),
        };
        (envelope, signing_key, public_key, fingerprint)
    }

    #[test]
    fn valid_backend_shaped_feed_verifies_and_authorizes_active_key() {
        let now_ms = 1_000_000;
        let (envelope, _signing_key, public_key, fingerprint) = fixture(now_ms);
        let verified = envelope.verify(now_ms).unwrap();
        assert_eq!(verified.payload.keys.len(), 2);
        verified
            .authorize_release_signing_key(ExtensionReleaseSigningKeyRef {
                algorithm: EXTENSION_TRUST_ALGORITHM,
                key_id: "marketplace-key-2",
                trust_epoch: 2,
                public_key: &public_key,
                public_key_fingerprint: &fingerprint,
            })
            .unwrap();
    }

    #[test]
    fn payload_or_signature_tampering_fails_closed() {
        let now_ms = 1_000_000;
        let (mut envelope, _, _, _) = fixture(now_ms);
        envelope.payload = envelope.payload.replace("marketplace-key-1", "marketplace-evil");
        assert_eq!(
            envelope.verify(now_ms),
            Err(ExtensionTrustFeedError::InvalidSignature)
        );

        let (mut envelope, _, _, _) = fixture(now_ms);
        envelope.signature = BASE64.encode([0_u8; 64]);
        assert_eq!(
            envelope.verify(now_ms),
            Err(ExtensionTrustFeedError::InvalidSignature)
        );
    }

    #[test]
    fn stale_or_future_feed_fails_closed() {
        let now_ms = 1_000_000;
        let (envelope, _, _, _) = fixture(now_ms);
        assert_eq!(
            envelope.clone().verify(now_ms + 60_001),
            Err(ExtensionTrustFeedError::Expired)
        );
        assert_eq!(
            envelope.verify(now_ms - EXTENSION_TRUST_MAX_FUTURE_SKEW_MS - 1),
            Err(ExtensionTrustFeedError::FutureDated)
        );
    }

    #[test]
    fn revoked_or_absent_release_signing_key_is_rejected() {
        let now_ms = 1_000_000;
        let (envelope, _, public_key, fingerprint) = fixture(now_ms);
        let verified = envelope.verify(now_ms).unwrap();
        assert_eq!(
            verified.authorize_release_signing_key(ExtensionReleaseSigningKeyRef {
                algorithm: EXTENSION_TRUST_ALGORITHM,
                key_id: "marketplace-key-1",
                trust_epoch: 1,
                public_key: &public_key,
                public_key_fingerprint: &fingerprint,
            }),
            Err(ExtensionTrustFeedError::SigningKeyRevoked)
        );
        assert_eq!(
            verified.authorize_release_signing_key(ExtensionReleaseSigningKeyRef {
                algorithm: EXTENSION_TRUST_ALGORITHM,
                key_id: "missing",
                trust_epoch: 99,
                public_key: &public_key,
                public_key_fingerprint: &fingerprint,
            }),
            Err(ExtensionTrustFeedError::SigningKeyAbsent)
        );
    }

    #[test]
    fn envelope_payload_mismatch_fails_even_with_valid_resigned_payload() {
        let now_ms = 1_000_000;
        let (mut envelope, signing_key, _, _) = fixture(now_ms);
        let mut payload: ExtensionTrustFeedPayload = serde_json::from_str(&envelope.payload).unwrap();
        payload.signer.trust_epoch = 3;
        envelope.payload = serde_json::to_string(&payload).unwrap();
        envelope.signature = BASE64.encode(signing_key.sign(envelope.payload.as_bytes()).to_bytes());
        assert_eq!(
            envelope.verify(now_ms),
            Err(ExtensionTrustFeedError::EnvelopePayloadMismatch)
        );
    }

    #[test]
    fn unknown_security_values_fail_deserialization() {
        let now_ms = 1_000_000;
        let (envelope, _, _, _) = fixture(now_ms);
        let unknown_status = envelope.payload.replace("\"revoked\"", "\"suspended\"");
        assert!(serde_json::from_str::<ExtensionTrustFeedPayload>(&unknown_status).is_err());

        let unknown_field = envelope.payload.replacen(
            "{\"protocol\"",
            "{\"futureSecurityField\":true,\"protocol\"",
            1,
        );
        assert!(serde_json::from_str::<ExtensionTrustFeedPayload>(&unknown_field).is_err());
    }

    #[test]
    fn key_material_and_fingerprint_are_validated_for_every_feed_key() {
        let now_ms = 1_000_000;
        let (mut envelope, signing_key, _, _) = fixture(now_ms);
        let mut payload: ExtensionTrustFeedPayload = serde_json::from_str(&envelope.payload).unwrap();
        payload.keys[0].public_key_fingerprint = "00".repeat(32);
        envelope.payload = serde_json::to_string(&payload).unwrap();
        envelope.signature = BASE64.encode(signing_key.sign(envelope.payload.as_bytes()).to_bytes());
        assert_eq!(
            envelope.verify(now_ms),
            Err(ExtensionTrustFeedError::InvalidTrustKey)
        );
    }
}
