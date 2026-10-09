//! Provider-connection policy and authenticated delivery encryption, independent of login grants.
use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::{Aead, Payload}};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};

pub(crate) fn connector(id: &str) -> Option<(&'static str, &'static str)> {
    Some(match id {
        "fabushi-official-github" => ("github", "repo read:user user:email"),
        "fabushi-official-google-gmail" => ("google", "openid email profile https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose"),
        "fabushi-official-google-drive" => ("google", "openid email profile https://www.googleapis.com/auth/drive.readonly https://www.googleapis.com/auth/drive.file"),
        "fabushi-official-google-docs" => ("google", "openid email profile https://www.googleapis.com/auth/drive.readonly https://www.googleapis.com/auth/drive.file https://www.googleapis.com/auth/documents.readonly https://www.googleapis.com/auth/documents"),
        "fabushi-official-google-sheets" => ("google", "openid email profile https://www.googleapis.com/auth/drive.readonly https://www.googleapis.com/auth/drive.file https://www.googleapis.com/auth/spreadsheets.readonly https://www.googleapis.com/auth/spreadsheets"),
        "fabushi-official-google-slides" => ("google", "openid email profile https://www.googleapis.com/auth/drive.readonly https://www.googleapis.com/auth/drive.file https://www.googleapis.com/auth/presentations.readonly https://www.googleapis.com/auth/presentations"),
        "fabushi-official-google-calendar" => ("google", "openid email profile https://www.googleapis.com/auth/calendar.calendarlist.readonly https://www.googleapis.com/auth/calendar.events.freebusy https://www.googleapis.com/auth/calendar.events.readonly"),
        "fabushi-official-google-chat" => ("google", "openid email profile https://www.googleapis.com/auth/chat.spaces.readonly https://www.googleapis.com/auth/chat.memberships.readonly https://www.googleapis.com/auth/chat.messages.readonly https://www.googleapis.com/auth/chat.messages.create https://www.googleapis.com/auth/chat.users.readstate"),
        "fabushi-official-google-people" => ("google", "openid email profile https://www.googleapis.com/auth/directory.readonly https://www.googleapis.com/auth/contacts.readonly"),
        _ => return None,
    })
}

pub(crate) fn digest(value: &str) -> String { format!("{:x}", Sha256::digest(value.as_bytes())) }

fn cipher(server_key: &str) -> Aes256Gcm {
    let key = Sha256::digest(format!("fabushi-mcp-delivery-key:v1\0{server_key}").as_bytes());
    Aes256Gcm::new_from_slice(&key).expect("SHA256 has the AES256 key size")
}

pub(crate) fn seal(server_key: &str, attempt: &str, plaintext: &[u8]) -> Result<String, &'static str> {
    let mut nonce = [0u8; 12];
    getrandom::getrandom(&mut nonce).map_err(|_| "secure nonce unavailable")?;
    let encrypted = cipher(server_key).encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: attempt.as_bytes() })
        .map_err(|_| "credential encryption failed")?;
    let mut payload = nonce.to_vec(); payload.extend(encrypted);
    Ok(URL_SAFE_NO_PAD.encode(payload))
}

pub(crate) fn open(server_key: &str, attempt: &str, ciphertext: &str) -> Result<Vec<u8>, &'static str> {
    let payload = URL_SAFE_NO_PAD.decode(ciphertext).map_err(|_| "invalid credential ciphertext")?;
    if payload.len() < 28 || payload.len() > 131_072 { return Err("invalid credential ciphertext"); }
    cipher(server_key).decrypt(Nonce::from_slice(&payload[..12]), Payload { msg: &payload[12..], aad: attempt.as_bytes() })
        .map_err(|_| "credential delivery authentication failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delivery_is_confidential_and_bound_to_attempt_and_key() {
        let a = seal("private-signing-key", "attempt-a", b"provider-refresh-token").unwrap();
        let b = seal("private-signing-key", "attempt-a", b"provider-refresh-token").unwrap();
        assert_ne!(a, b);
        assert!(!a.contains("provider-refresh-token"));
        assert_eq!(open("private-signing-key", "attempt-a", &a).unwrap(), b"provider-refresh-token");
        assert!(open("private-signing-key", "attempt-b", &a).is_err());
        assert!(open("rotated-key", "attempt-a", &a).is_err());
        assert!(open("private-signing-key", "attempt-a", "malformed").is_err());
    }
    #[test]
    fn grants_are_connector_specific_and_never_login_only() {
        let (_, gmail) = connector("fabushi-official-google-gmail").unwrap();
        assert!(gmail.contains("gmail.readonly")); assert!(!gmail.contains("drive"));
        assert!(connector("fabushi-official-github").unwrap().1.contains("repo"));
        assert!(connector("https://attacker.example/mcp").is_none());
    }
}
