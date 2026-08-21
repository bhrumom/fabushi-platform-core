use crate::actor::ActorId;
use crate::conversation::ConversationId;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fmt;
use thiserror::Error;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use zeroize::Zeroize;

const SECRET_CHAT_VERSION: u16 = 1;
const SECRET_CHAT_ALGORITHM: &str = "X25519+HKDF-SHA256+XChaCha20Poly1305";
const REPLAY_WINDOW_BITS: u64 = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretPublicKey {
    pub algorithm: String,
    pub key_base64: String,
}

impl SecretPublicKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self {
            algorithm: "X25519".into(),
            key_base64: URL_SAFE_NO_PAD.encode(bytes),
        }
    }

    pub fn to_bytes(&self) -> Result<[u8; 32], SecretChatError> {
        if self.algorithm != "X25519" {
            return Err(SecretChatError::UnsupportedAlgorithm(
                self.algorithm.clone(),
            ));
        }
        decode_array::<32>(&self.key_base64, "public key")
    }
}

pub struct SecretPrivateKey([u8; 32]);

impl SecretPrivateKey {
    pub fn generate() -> Result<Self, SecretChatError> {
        let mut bytes = [0u8; 32];
        getrandom::getrandom(&mut bytes)
            .map_err(|error| SecretChatError::Random(error.to_string()))?;
        Ok(Self(bytes))
    }

    pub fn public_key(&self) -> SecretPublicKey {
        let secret = StaticSecret::from(self.0);
        let public = X25519PublicKey::from(&secret);
        SecretPublicKey::from_bytes(public.to_bytes())
    }
}

impl fmt::Debug for SecretPrivateKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretPrivateKey([REDACTED])")
    }
}

impl Drop for SecretPrivateKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedSecretMessage {
    pub version: u16,
    pub algorithm: String,
    pub conversation_id: ConversationId,
    pub epoch: u64,
    pub sender_id: ActorId,
    pub recipient_id: ActorId,
    pub counter: u64,
    pub nonce_base64: String,
    pub ciphertext_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptedSecretBackup {
    pub version: u16,
    pub algorithm: String,
    pub conversation_id: ConversationId,
    pub nonce_base64: String,
    pub ciphertext_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecretBackupPayload {
    conversation_id: ConversationId,
    local_actor_id: ActorId,
    peer_actor_id: ActorId,
    epoch: u64,
    root_key_base64: String,
    send_counter: u64,
    replay_highest: u64,
    replay_bitmap_base64: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ReplayWindow {
    highest: u64,
    bitmap: u128,
}

impl ReplayWindow {
    fn check(&self, counter: u64) -> Result<(), SecretChatError> {
        if counter == 0 {
            return Err(SecretChatError::InvalidCounter);
        }
        if self.highest == 0 || counter > self.highest {
            return Ok(());
        }
        let distance = self.highest - counter;
        if distance >= REPLAY_WINDOW_BITS {
            return Err(SecretChatError::StaleMessage);
        }
        if self.bitmap & (1u128 << distance) != 0 {
            return Err(SecretChatError::ReplayDetected);
        }
        Ok(())
    }

    fn mark(&mut self, counter: u64) {
        if self.highest == 0 {
            self.highest = counter;
            self.bitmap = 1;
            return;
        }
        if counter > self.highest {
            let shift = counter - self.highest;
            self.bitmap = if shift >= REPLAY_WINDOW_BITS {
                1
            } else {
                (self.bitmap << shift) | 1
            };
            self.highest = counter;
            return;
        }
        let distance = self.highest - counter;
        if distance < REPLAY_WINDOW_BITS {
            self.bitmap |= 1u128 << distance;
        }
    }
}

pub struct SecretChatSession {
    conversation_id: ConversationId,
    local_actor_id: ActorId,
    peer_actor_id: ActorId,
    epoch: u64,
    root_key: [u8; 32],
    send_counter: u64,
    replay: ReplayWindow,
}

impl fmt::Debug for SecretChatSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretChatSession")
            .field("conversation_id", &self.conversation_id)
            .field("local_actor_id", &self.local_actor_id)
            .field("peer_actor_id", &self.peer_actor_id)
            .field("epoch", &self.epoch)
            .field("send_counter", &self.send_counter)
            .field("replay_highest", &self.replay.highest)
            .finish_non_exhaustive()
    }
}

impl SecretChatSession {
    pub fn establish(
        conversation_id: ConversationId,
        local_actor_id: ActorId,
        peer_actor_id: ActorId,
        local_private_key: &SecretPrivateKey,
        peer_public_key: &SecretPublicKey,
        epoch: u64,
    ) -> Result<Self, SecretChatError> {
        validate_identity(&conversation_id, &local_actor_id, &peer_actor_id, epoch)?;
        let root_key = derive_root_key(
            &conversation_id,
            &local_actor_id,
            &peer_actor_id,
            local_private_key,
            peer_public_key,
            epoch,
        )?;
        Ok(Self {
            conversation_id,
            local_actor_id,
            peer_actor_id,
            epoch,
            root_key,
            send_counter: 0,
            replay: ReplayWindow::default(),
        })
    }

    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn rotate(
        &mut self,
        local_private_key: &SecretPrivateKey,
        peer_public_key: &SecretPublicKey,
        next_epoch: u64,
    ) -> Result<(), SecretChatError> {
        if next_epoch <= self.epoch {
            return Err(SecretChatError::NonMonotonicEpoch {
                current: self.epoch,
                requested: next_epoch,
            });
        }
        let next = derive_root_key(
            &self.conversation_id,
            &self.local_actor_id,
            &self.peer_actor_id,
            local_private_key,
            peer_public_key,
            next_epoch,
        )?;
        self.root_key.zeroize();
        self.root_key = next;
        self.epoch = next_epoch;
        self.send_counter = 0;
        self.replay = ReplayWindow::default();
        Ok(())
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<EncryptedSecretMessage, SecretChatError> {
        if plaintext.is_empty() {
            return Err(SecretChatError::EmptyPlaintext);
        }
        self.send_counter = self
            .send_counter
            .checked_add(1)
            .ok_or(SecretChatError::CounterOverflow)?;
        let mut nonce = [0u8; 24];
        getrandom::getrandom(&mut nonce)
            .map_err(|error| SecretChatError::Random(error.to_string()))?;
        let aad = message_aad(
            &self.conversation_id,
            self.epoch,
            &self.local_actor_id,
            &self.peer_actor_id,
            self.send_counter,
        );
        let cipher = XChaCha20Poly1305::new_from_slice(&self.root_key)
            .map_err(|_| SecretChatError::InvalidKey)?;
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| SecretChatError::EncryptionFailed)?;
        Ok(EncryptedSecretMessage {
            version: SECRET_CHAT_VERSION,
            algorithm: SECRET_CHAT_ALGORITHM.into(),
            conversation_id: self.conversation_id.clone(),
            epoch: self.epoch,
            sender_id: self.local_actor_id.clone(),
            recipient_id: self.peer_actor_id.clone(),
            counter: self.send_counter,
            nonce_base64: URL_SAFE_NO_PAD.encode(nonce),
            ciphertext_base64: URL_SAFE_NO_PAD.encode(ciphertext),
        })
    }

    pub fn decrypt(
        &mut self,
        envelope: &EncryptedSecretMessage,
    ) -> Result<Vec<u8>, SecretChatError> {
        if envelope.version != SECRET_CHAT_VERSION {
            return Err(SecretChatError::UnsupportedVersion(envelope.version));
        }
        if envelope.algorithm != SECRET_CHAT_ALGORITHM {
            return Err(SecretChatError::UnsupportedAlgorithm(
                envelope.algorithm.clone(),
            ));
        }
        if envelope.conversation_id != self.conversation_id
            || envelope.sender_id != self.peer_actor_id
            || envelope.recipient_id != self.local_actor_id
        {
            return Err(SecretChatError::IdentityMismatch);
        }
        if envelope.epoch != self.epoch {
            return Err(SecretChatError::EpochMismatch {
                expected: self.epoch,
                actual: envelope.epoch,
            });
        }
        self.replay.check(envelope.counter)?;
        let nonce = decode_array::<24>(&envelope.nonce_base64, "message nonce")?;
        let ciphertext = URL_SAFE_NO_PAD
            .decode(envelope.ciphertext_base64.as_bytes())
            .map_err(|_| SecretChatError::InvalidEncoding("ciphertext"))?;
        let aad = message_aad(
            &self.conversation_id,
            self.epoch,
            &envelope.sender_id,
            &envelope.recipient_id,
            envelope.counter,
        );
        let cipher = XChaCha20Poly1305::new_from_slice(&self.root_key)
            .map_err(|_| SecretChatError::InvalidKey)?;
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| SecretChatError::AuthenticationFailed)?;
        self.replay.mark(envelope.counter);
        Ok(plaintext)
    }

    pub fn backup(
        &self,
        recovery_key: &[u8; 32],
    ) -> Result<EncryptedSecretBackup, SecretChatError> {
        let mut replay_bytes = self.replay.bitmap.to_be_bytes();
        let payload = SecretBackupPayload {
            conversation_id: self.conversation_id.clone(),
            local_actor_id: self.local_actor_id.clone(),
            peer_actor_id: self.peer_actor_id.clone(),
            epoch: self.epoch,
            root_key_base64: URL_SAFE_NO_PAD.encode(self.root_key),
            send_counter: self.send_counter,
            replay_highest: self.replay.highest,
            replay_bitmap_base64: URL_SAFE_NO_PAD.encode(replay_bytes),
        };
        replay_bytes.zeroize();
        let mut encoded = serde_json::to_vec(&payload)?;
        let mut nonce = [0u8; 24];
        getrandom::getrandom(&mut nonce)
            .map_err(|error| SecretChatError::Random(error.to_string()))?;
        let cipher = XChaCha20Poly1305::new_from_slice(recovery_key)
            .map_err(|_| SecretChatError::InvalidRecoveryKey)?;
        let aad = backup_aad(&self.conversation_id);
        let encrypted = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &encoded,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| SecretChatError::EncryptionFailed)?;
        encoded.zeroize();
        Ok(EncryptedSecretBackup {
            version: SECRET_CHAT_VERSION,
            algorithm: "XChaCha20Poly1305".into(),
            conversation_id: self.conversation_id.clone(),
            nonce_base64: URL_SAFE_NO_PAD.encode(nonce),
            ciphertext_base64: URL_SAFE_NO_PAD.encode(encrypted),
        })
    }

    pub fn restore(
        backup: &EncryptedSecretBackup,
        recovery_key: &[u8; 32],
    ) -> Result<Self, SecretChatError> {
        if backup.version != SECRET_CHAT_VERSION {
            return Err(SecretChatError::UnsupportedVersion(backup.version));
        }
        if backup.algorithm != "XChaCha20Poly1305" {
            return Err(SecretChatError::UnsupportedAlgorithm(
                backup.algorithm.clone(),
            ));
        }
        let nonce = decode_array::<24>(&backup.nonce_base64, "backup nonce")?;
        let encrypted = URL_SAFE_NO_PAD
            .decode(backup.ciphertext_base64.as_bytes())
            .map_err(|_| SecretChatError::InvalidEncoding("backup ciphertext"))?;
        let cipher = XChaCha20Poly1305::new_from_slice(recovery_key)
            .map_err(|_| SecretChatError::InvalidRecoveryKey)?;
        let aad = backup_aad(&backup.conversation_id);
        let mut plaintext = cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &encrypted,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| SecretChatError::AuthenticationFailed)?;
        let payload: SecretBackupPayload = serde_json::from_slice(&plaintext)?;
        plaintext.zeroize();
        if payload.conversation_id != backup.conversation_id {
            return Err(SecretChatError::IdentityMismatch);
        }
        validate_identity(
            &payload.conversation_id,
            &payload.local_actor_id,
            &payload.peer_actor_id,
            payload.epoch,
        )?;
        let root_key = decode_array::<32>(&payload.root_key_base64, "root key")?;
        let replay_bitmap = u128::from_be_bytes(decode_array::<16>(
            &payload.replay_bitmap_base64,
            "replay bitmap",
        )?);
        Ok(Self {
            conversation_id: payload.conversation_id,
            local_actor_id: payload.local_actor_id,
            peer_actor_id: payload.peer_actor_id,
            epoch: payload.epoch,
            root_key,
            send_counter: payload.send_counter,
            replay: ReplayWindow {
                highest: payload.replay_highest,
                bitmap: replay_bitmap,
            },
        })
    }
}

impl Drop for SecretChatSession {
    fn drop(&mut self) {
        self.root_key.zeroize();
    }
}

fn derive_root_key(
    conversation_id: &ConversationId,
    local_actor_id: &ActorId,
    peer_actor_id: &ActorId,
    local_private_key: &SecretPrivateKey,
    peer_public_key: &SecretPublicKey,
    epoch: u64,
) -> Result<[u8; 32], SecretChatError> {
    let peer_public = X25519PublicKey::from(peer_public_key.to_bytes()?);
    let local_secret = StaticSecret::from(local_private_key.0);
    let shared = local_secret.diffie_hellman(&peer_public);
    if shared.as_bytes().iter().all(|byte| *byte == 0) {
        return Err(SecretChatError::InvalidPeerKey);
    }
    let (first_actor, second_actor) = if local_actor_id.0 <= peer_actor_id.0 {
        (&local_actor_id.0, &peer_actor_id.0)
    } else {
        (&peer_actor_id.0, &local_actor_id.0)
    };
    let info = format!(
        "fabushi-secret-chat-v1|{}|{}|{}|{}",
        conversation_id.0, first_actor, second_actor, epoch
    );
    let hkdf = Hkdf::<Sha256>::new(Some(conversation_id.0.as_bytes()), shared.as_bytes());
    let mut root = [0u8; 32];
    hkdf.expand(info.as_bytes(), &mut root)
        .map_err(|_| SecretChatError::KdfFailed)?;
    Ok(root)
}

fn validate_identity(
    conversation_id: &ConversationId,
    local_actor_id: &ActorId,
    peer_actor_id: &ActorId,
    epoch: u64,
) -> Result<(), SecretChatError> {
    if !conversation_id.is_valid()
        || !local_actor_id.is_valid()
        || !peer_actor_id.is_valid()
        || local_actor_id == peer_actor_id
        || epoch == 0
    {
        return Err(SecretChatError::InvalidIdentity);
    }
    Ok(())
}

fn message_aad(
    conversation_id: &ConversationId,
    epoch: u64,
    sender_id: &ActorId,
    recipient_id: &ActorId,
    counter: u64,
) -> String {
    format!(
        "fabushi-secret-chat-v1|{}|{}|{}|{}|{}",
        conversation_id.0, epoch, sender_id.0, recipient_id.0, counter
    )
}

fn backup_aad(conversation_id: &ConversationId) -> String {
    format!("fabushi-secret-backup-v1|{}", conversation_id.0)
}

fn decode_array<const N: usize>(
    value: &str,
    label: &'static str,
) -> Result<[u8; N], SecretChatError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value.as_bytes())
        .map_err(|_| SecretChatError::InvalidEncoding(label))?;
    bytes
        .try_into()
        .map_err(|_| SecretChatError::InvalidEncoding(label))
}

#[derive(Debug, Error)]
pub enum SecretChatError {
    #[error("secret chat identity or epoch is invalid")]
    InvalidIdentity,
    #[error("secret chat plaintext must not be empty")]
    EmptyPlaintext,
    #[error("secret chat random source failed: {0}")]
    Random(String),
    #[error("secret chat public key is invalid")]
    InvalidPeerKey,
    #[error("secret chat key is invalid")]
    InvalidKey,
    #[error("secret chat recovery key is invalid")]
    InvalidRecoveryKey,
    #[error("secret chat key derivation failed")]
    KdfFailed,
    #[error("secret chat encryption failed")]
    EncryptionFailed,
    #[error("secret chat authentication failed")]
    AuthenticationFailed,
    #[error("secret chat message identity does not match the current session")]
    IdentityMismatch,
    #[error("secret chat counter is invalid")]
    InvalidCounter,
    #[error("secret chat send counter overflowed")]
    CounterOverflow,
    #[error("secret chat replay was detected")]
    ReplayDetected,
    #[error("secret chat message is outside the replay window")]
    StaleMessage,
    #[error("secret chat epoch mismatch: expected {expected}, got {actual}")]
    EpochMismatch { expected: u64, actual: u64 },
    #[error("secret chat rotation epoch must increase from {current} to above {requested}")]
    NonMonotonicEpoch { current: u64, requested: u64 },
    #[error("secret chat protocol version {0} is unsupported")]
    UnsupportedVersion(u16),
    #[error("secret chat algorithm is unsupported: {0}")]
    UnsupportedAlgorithm(String),
    #[error("secret chat {0} encoding is invalid")]
    InvalidEncoding(&'static str),
    #[error("secret chat backup serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(
        epoch: u64,
    ) -> (
        SecretChatSession,
        SecretChatSession,
        SecretPrivateKey,
        SecretPrivateKey,
    ) {
        let alice_private = SecretPrivateKey::generate().unwrap();
        let bob_private = SecretPrivateKey::generate().unwrap();
        let alice_public = alice_private.public_key();
        let bob_public = bob_private.public_key();
        let alice = SecretChatSession::establish(
            ConversationId::new("secret:1"),
            ActorId::new("human:alice"),
            ActorId::new("human:bob"),
            &alice_private,
            &bob_public,
            epoch,
        )
        .unwrap();
        let bob = SecretChatSession::establish(
            ConversationId::new("secret:1"),
            ActorId::new("human:bob"),
            ActorId::new("human:alice"),
            &bob_private,
            &alice_public,
            epoch,
        )
        .unwrap();
        (alice, bob, alice_private, bob_private)
    }

    #[test]
    fn x25519_sessions_encrypt_and_decrypt_bidirectionally() {
        let (mut alice, mut bob, _, _) = pair(1);
        let envelope = alice.encrypt("南无阿弥陀佛".as_bytes()).unwrap();
        assert_eq!(bob.decrypt(&envelope).unwrap(), "南无阿弥陀佛".as_bytes());
        let reply = bob.encrypt(b"received").unwrap();
        assert_eq!(alice.decrypt(&reply).unwrap(), b"received");
    }

    #[test]
    fn tamper_and_replay_are_rejected() {
        let (mut alice, mut bob, _, _) = pair(1);
        let envelope = alice.encrypt(b"authenticated").unwrap();
        let mut tampered = envelope.clone();
        tampered.ciphertext_base64.push('A');
        assert!(matches!(
            bob.decrypt(&tampered),
            Err(SecretChatError::AuthenticationFailed | SecretChatError::InvalidEncoding(_))
        ));
        assert_eq!(bob.decrypt(&envelope).unwrap(), b"authenticated");
        assert!(matches!(
            bob.decrypt(&envelope),
            Err(SecretChatError::ReplayDetected)
        ));
    }

    #[test]
    fn rotation_uses_fresh_x25519_keys_and_monotonic_epochs() {
        let (mut alice, mut bob, _, _) = pair(1);
        let new_alice_private = SecretPrivateKey::generate().unwrap();
        let new_bob_private = SecretPrivateKey::generate().unwrap();
        let new_alice_public = new_alice_private.public_key();
        let new_bob_public = new_bob_private.public_key();
        alice
            .rotate(&new_alice_private, &new_bob_public, 2)
            .unwrap();
        bob.rotate(&new_bob_private, &new_alice_public, 2).unwrap();
        let envelope = alice.encrypt(b"epoch two").unwrap();
        assert_eq!(bob.decrypt(&envelope).unwrap(), b"epoch two");
        assert!(matches!(
            alice.rotate(&new_alice_private, &new_bob_public, 2),
            Err(SecretChatError::NonMonotonicEpoch { .. })
        ));
    }

    #[test]
    fn encrypted_backup_restores_session_without_plaintext_key_storage() {
        let (mut alice, mut bob, _, _) = pair(7);
        let first = alice.encrypt(b"before backup").unwrap();
        assert_eq!(bob.decrypt(&first).unwrap(), b"before backup");
        let recovery_key = [9u8; 32];
        let backup = alice.backup(&recovery_key).unwrap();
        assert!(!backup.ciphertext_base64.contains("before backup"));
        let mut restored = SecretChatSession::restore(&backup, &recovery_key).unwrap();
        let after = restored.encrypt(b"after restore").unwrap();
        assert_eq!(bob.decrypt(&after).unwrap(), b"after restore");
        let wrong_key = [8u8; 32];
        assert!(matches!(
            SecretChatSession::restore(&backup, &wrong_key),
            Err(SecretChatError::AuthenticationFailed)
        ));
    }
}
