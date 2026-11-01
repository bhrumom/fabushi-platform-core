use crate::api::{parse_help_config_simple, ApiRequestError, HelpConfigSimple, IpPort};
use aes::{
    cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit},
    Aes256,
};
use num_bigint_dig::BigUint;
use sha2::{Digest, Sha256};
use std::net::Ipv4Addr;
use subtle::ConstantTimeEq;
use thiserror::Error;

const SPECIAL_CONFIG_BASE64_LENGTH: usize = 344;
const SPECIAL_CONFIG_RSA_LENGTH: usize = 256;
const SPECIAL_CONFIG_AES_KEY_LENGTH: usize = 32;
const SPECIAL_CONFIG_AES_BLOCK_LENGTH: usize = 16;
const SPECIAL_CONFIG_ENCRYPTED_LENGTH: usize = 224;
const SPECIAL_CONFIG_DATA_LENGTH: usize = 208;
const SPECIAL_CONFIG_DIGEST_LENGTH: usize = 16;
const SPECIAL_CONFIG_RSA_EXPONENT: u32 = 65_537;
const SPECIAL_CONFIG_RSA_MODULUS_BASE64: &str = concat!(
    "yr+18Rex2ohtVy8sroGPBwXD3DOoKCSpjDqYoXgCqB7ioln4eDCF",
    "fOBUlfXUEvM/fnKCpF46VkAftlb4VuPDeQSS/ZxZYEGqHaywlroVnXHIjgqoxiAd",
    "192xRGreuXIaUKmkwlM9JID9WS2jUsTpzQ91L8MEPLJ/4zrBwZua8W5fECwCCh2c",
    "9G5IzzBm+otMS/YKwmR1olzRCyEkyAEjXWqBI9Ftv5eG8m0VkBzOG655WIYdyV0H",
    "fDK/NWcvGqa0w/nriMD6mDjKOryamw0OP9QuYgMN0C9xMW9y8SmP4h92OAWodTYg",
    "Y1hZCxdv6cs5UnW9+PWvS+WIbkh+GaWYxw==",
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialConfigEndpoint {
    pub dc_id: i32,
    pub ip_address: Ipv4Addr,
    pub port: i32,
    pub secret: Vec<u8>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SpecialConfigError {
    #[error("special config base64 length is {actual}; expected {expected}")]
    InvalidEncodedLength { actual: usize, expected: usize },
    #[error("special config base64 is malformed")]
    InvalidBase64,
    #[error("special config decoded length is {actual}; expected {expected}")]
    InvalidDecodedLength { actual: usize, expected: usize },
    #[error("special config pinned RSA key is invalid")]
    InvalidPinnedRsaKey,
    #[error("special config RSA input is outside the pinned modulus")]
    RsaInputOutOfRange,
    #[error("special config RSA result is invalid")]
    InvalidRsaResult,
    #[error("special config encrypted payload layout is invalid")]
    InvalidEncryptedLayout,
    #[error("special config digest does not match")]
    DigestMismatch,
    #[error("special config declared TL length is invalid: {0}")]
    InvalidDeclaredLength(i32),
    #[error("special config TL payload is invalid: {0}")]
    Protocol(#[from] ApiRequestError),
    #[error("special config expired at {expires}; current Unix time is {now_unix}")]
    Expired { now_unix: i64, expires: i32 },
    #[error("special config contains no access-point rules")]
    EmptyRules,
}

pub fn decode_special_config_payload(
    encoded: &[u8],
) -> Result<HelpConfigSimple, SpecialConfigError> {
    let normalized: Vec<u8> = encoded
        .iter()
        .copied()
        .filter(|byte| is_base64_text_byte(*byte))
        .collect();
    if normalized.len() != SPECIAL_CONFIG_BASE64_LENGTH {
        return Err(SpecialConfigError::InvalidEncodedLength {
            actual: normalized.len(),
            expected: SPECIAL_CONFIG_BASE64_LENGTH,
        });
    }

    let encrypted = decode_standard_base64(&normalized)?;
    if encrypted.len() != SPECIAL_CONFIG_RSA_LENGTH {
        return Err(SpecialConfigError::InvalidDecodedLength {
            actual: encrypted.len(),
            expected: SPECIAL_CONFIG_RSA_LENGTH,
        });
    }
    let recovered = raw_rsa_public_recover(&encrypted)?;
    decode_recovered_special_config(&recovered)
}

pub fn select_special_config_endpoints(
    config: &HelpConfigSimple,
    phone: &str,
    now_unix: i64,
) -> Result<Vec<SpecialConfigEndpoint>, SpecialConfigError> {
    if now_unix > i64::from(config.expires) {
        return Err(SpecialConfigError::Expired {
            now_unix,
            expires: config.expires,
        });
    }
    if config.rules.is_empty() {
        return Err(SpecialConfigError::EmptyRules);
    }

    let cleaned_phone: String = phone
        .bytes()
        .filter(|byte| byte.is_ascii_digit())
        .map(char::from)
        .collect();
    let mut endpoints = Vec::new();
    for rule in &config.rules {
        if !phone_matches_prefix_rules(&cleaned_phone, &rule.phone_prefix_rules) {
            continue;
        }
        for address in &rule.ips {
            match address {
                IpPort::Plain { ipv4, port } => endpoints.push(SpecialConfigEndpoint {
                    dc_id: rule.dc_id,
                    ip_address: ipv4_from_tl_bits(*ipv4),
                    port: *port,
                    secret: Vec::new(),
                }),
                IpPort::Secret {
                    ipv4,
                    port,
                    secret,
                } => endpoints.push(SpecialConfigEndpoint {
                    dc_id: rule.dc_id,
                    ip_address: ipv4_from_tl_bits(*ipv4),
                    port: *port,
                    secret: secret.clone(),
                }),
            }
        }
    }
    Ok(endpoints)
}

pub fn decode_and_select_special_config(
    encoded: &[u8],
    phone: &str,
    now_unix: i64,
) -> Result<Vec<SpecialConfigEndpoint>, SpecialConfigError> {
    let config = decode_special_config_payload(encoded)?;
    select_special_config_endpoints(&config, phone, now_unix)
}

fn decode_recovered_special_config(
    recovered: &[u8],
) -> Result<HelpConfigSimple, SpecialConfigError> {
    if recovered.len() != SPECIAL_CONFIG_RSA_LENGTH {
        return Err(SpecialConfigError::InvalidDecodedLength {
            actual: recovered.len(),
            expected: SPECIAL_CONFIG_RSA_LENGTH,
        });
    }
    let key: [u8; SPECIAL_CONFIG_AES_KEY_LENGTH] = recovered[..SPECIAL_CONFIG_AES_KEY_LENGTH]
        .try_into()
        .map_err(|_| SpecialConfigError::InvalidEncryptedLayout)?;
    let iv: [u8; SPECIAL_CONFIG_AES_BLOCK_LENGTH] = recovered
        [SPECIAL_CONFIG_AES_KEY_LENGTH - SPECIAL_CONFIG_AES_BLOCK_LENGTH
            ..SPECIAL_CONFIG_AES_KEY_LENGTH]
        .try_into()
        .map_err(|_| SpecialConfigError::InvalidEncryptedLayout)?;
    let ciphertext = &recovered[SPECIAL_CONFIG_AES_KEY_LENGTH..];
    if ciphertext.len() != SPECIAL_CONFIG_ENCRYPTED_LENGTH {
        return Err(SpecialConfigError::InvalidEncryptedLayout);
    }
    let decrypted = aes256_cbc_decrypt(ciphertext, &key, &iv)?;
    if decrypted.len() != SPECIAL_CONFIG_ENCRYPTED_LENGTH {
        return Err(SpecialConfigError::InvalidEncryptedLayout);
    }

    let (data, expected_digest) = decrypted.split_at(SPECIAL_CONFIG_DATA_LENGTH);
    if expected_digest.len() != SPECIAL_CONFIG_DIGEST_LENGTH {
        return Err(SpecialConfigError::InvalidEncryptedLayout);
    }
    let actual_digest = Sha256::digest(data);
    if !bool::from(expected_digest.ct_eq(&actual_digest[..SPECIAL_CONFIG_DIGEST_LENGTH])) {
        return Err(SpecialConfigError::DigestMismatch);
    }

    let real_length = i32::from_le_bytes(
        data[..4]
            .try_into()
            .map_err(|_| SpecialConfigError::InvalidEncryptedLayout)?,
    );
    if real_length <= 0
        || real_length as usize > SPECIAL_CONFIG_DATA_LENGTH
        || real_length & 0x03 != 0
    {
        return Err(SpecialConfigError::InvalidDeclaredLength(real_length));
    }
    parse_help_config_simple(&data[4..real_length as usize]).map_err(Into::into)
}

fn raw_rsa_public_recover(input: &[u8]) -> Result<Vec<u8>, SpecialConfigError> {
    if input.len() != SPECIAL_CONFIG_RSA_LENGTH {
        return Err(SpecialConfigError::InvalidDecodedLength {
            actual: input.len(),
            expected: SPECIAL_CONFIG_RSA_LENGTH,
        });
    }
    let modulus_bytes =
        decode_standard_base64(SPECIAL_CONFIG_RSA_MODULUS_BASE64.as_bytes())
            .map_err(|_| SpecialConfigError::InvalidPinnedRsaKey)?;
    if modulus_bytes.len() != SPECIAL_CONFIG_RSA_LENGTH {
        return Err(SpecialConfigError::InvalidPinnedRsaKey);
    }

    let modulus = BigUint::from_bytes_be(&modulus_bytes);
    let value = BigUint::from_bytes_be(input);
    if value >= modulus {
        return Err(SpecialConfigError::RsaInputOutOfRange);
    }
    let exponent = BigUint::from(SPECIAL_CONFIG_RSA_EXPONENT);
    let recovered = value.modpow(&exponent, &modulus).to_bytes_be();
    if recovered.len() > SPECIAL_CONFIG_RSA_LENGTH {
        return Err(SpecialConfigError::InvalidRsaResult);
    }
    let mut output = vec![0_u8; SPECIAL_CONFIG_RSA_LENGTH - recovered.len()];
    output.extend_from_slice(&recovered);
    Ok(output)
}

fn aes256_cbc_decrypt(
    ciphertext: &[u8],
    key: &[u8; SPECIAL_CONFIG_AES_KEY_LENGTH],
    iv: &[u8; SPECIAL_CONFIG_AES_BLOCK_LENGTH],
) -> Result<Vec<u8>, SpecialConfigError> {
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(SPECIAL_CONFIG_AES_BLOCK_LENGTH) {
        return Err(SpecialConfigError::InvalidEncryptedLayout);
    }
    let cipher = Aes256::new(GenericArray::from_slice(key));
    let mut previous = *iv;
    let mut output = Vec::with_capacity(ciphertext.len());
    for chunk in ciphertext.chunks_exact(SPECIAL_CONFIG_AES_BLOCK_LENGTH) {
        let current: [u8; SPECIAL_CONFIG_AES_BLOCK_LENGTH] = chunk
            .try_into()
            .map_err(|_| SpecialConfigError::InvalidEncryptedLayout)?;
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        for (byte, prior) in block.iter_mut().zip(previous) {
            *byte ^= prior;
        }
        output.extend_from_slice(&block);
        previous = current;
    }
    Ok(output)
}

fn phone_matches_prefix_rules(cleaned_phone: &str, rules: &str) -> bool {
    let mut matched = false;
    for prefix in rules.split(',') {
        if prefix.is_empty() {
            matched = true;
        } else if let Some(required) = prefix.strip_prefix('+') {
            if cleaned_phone.starts_with(required) {
                matched = true;
            }
        } else if let Some(rejected) = prefix.strip_prefix('-') {
            if cleaned_phone.starts_with(rejected) {
                return false;
            }
        }
    }
    matched
}

fn ipv4_from_tl_bits(ipv4: i32) -> Ipv4Addr {
    Ipv4Addr::from(ipv4 as u32)
}

fn is_base64_text_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
}

fn decode_standard_base64(input: &[u8]) -> Result<Vec<u8>, SpecialConfigError> {
    if input.is_empty() || input.len() % 4 != 0 {
        return Err(SpecialConfigError::InvalidBase64);
    }
    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    let chunks = input.len() / 4;
    for (index, chunk) in input.chunks_exact(4).enumerate() {
        let last = index + 1 == chunks;
        if chunk[0] == b'=' || chunk[1] == b'=' {
            return Err(SpecialConfigError::InvalidBase64);
        }
        let a = base64_value(chunk[0]).ok_or(SpecialConfigError::InvalidBase64)?;
        let b = base64_value(chunk[1]).ok_or(SpecialConfigError::InvalidBase64)?;
        output.push((a << 2) | (b >> 4));

        if chunk[2] == b'=' {
            if !last || chunk[3] != b'=' {
                return Err(SpecialConfigError::InvalidBase64);
            }
            continue;
        }
        let c = base64_value(chunk[2]).ok_or(SpecialConfigError::InvalidBase64)?;
        output.push((b << 4) | (c >> 2));

        if chunk[3] == b'=' {
            if !last {
                return Err(SpecialConfigError::InvalidBase64);
            }
            continue;
        }
        let d = base64_value(chunk[3]).ok_or(SpecialConfigError::InvalidBase64)?;
        output.push((c << 6) | d);
    }
    Ok(output)
}

fn base64_value(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{
        AccessPointRule, ACCESS_POINT_RULE_CONSTRUCTOR, HELP_CONFIG_SIMPLE_CONSTRUCTOR,
        IP_PORT_SECRET_CONSTRUCTOR,
    };
    use crate::tl::TlWriter;
    use aes::cipher::BlockEncrypt;

    fn decode_hex(value: &str) -> Vec<u8> {
        hex::decode(value).expect("fixed test hex")
    }

    fn aes256_cbc_encrypt_for_fixture(
        plaintext: &[u8],
        key: &[u8; SPECIAL_CONFIG_AES_KEY_LENGTH],
        iv: &[u8; SPECIAL_CONFIG_AES_BLOCK_LENGTH],
    ) -> Vec<u8> {
        assert!(plaintext.len().is_multiple_of(SPECIAL_CONFIG_AES_BLOCK_LENGTH));
        let cipher = Aes256::new(GenericArray::from_slice(key));
        let mut previous = *iv;
        let mut output = Vec::with_capacity(plaintext.len());
        for chunk in plaintext.chunks_exact(SPECIAL_CONFIG_AES_BLOCK_LENGTH) {
            let mut block = [0_u8; SPECIAL_CONFIG_AES_BLOCK_LENGTH];
            for ((target, source), prior) in block.iter_mut().zip(chunk).zip(previous) {
                *target = *source ^ prior;
            }
            let mut encrypted = GenericArray::clone_from_slice(&block);
            cipher.encrypt_block(&mut encrypted);
            output.extend_from_slice(&encrypted);
            previous.copy_from_slice(&encrypted);
        }
        output
    }

    #[test]
    fn aes256_cbc_matches_nist_vector() {
        let key: [u8; 32] = decode_hex(
            "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4",
        )
        .try_into()
        .expect("key");
        let iv: [u8; 16] = decode_hex("000102030405060708090a0b0c0d0e0f")
            .try_into()
            .expect("iv");
        let ciphertext = decode_hex(concat!(
            "f58c4c04d6e5f1ba779eabfb5f7bfbd6",
            "9cfc4e967edb808d679f777bc6702c7d",
            "39f23369a9d9bacfa530e26304231461",
            "b2eb05e2c39be9fcda6c19078c6a9d1b",
        ));
        let plaintext = decode_hex(concat!(
            "6bc1bee22e409f96e93d7e117393172a",
            "ae2d8a571e03ac9c9eb76fac45af8e51",
            "30c81c46a35ce411e5fbc1191a0a52ef",
            "f69f2445df4f9b17ad2b417be66c3710",
        ));
        assert_eq!(
            aes256_cbc_decrypt(&ciphertext, &key, &iv).expect("decrypt"),
            plaintext
        );
    }

    #[test]
    fn recovered_payload_uses_declared_length_digest_and_existing_tl_parser() {
        let mut tl = TlWriter::new();
        tl.write_u32(HELP_CONFIG_SIMPLE_CONSTRUCTOR);
        tl.write_i32(10);
        tl.write_i32(20);
        tl.write_vector_length(1).expect("rules");
        tl.write_u32(ACCESS_POINT_RULE_CONSTRUCTOR);
        tl.write_string("+1").expect("phone rules");
        tl.write_i32(2);
        tl.write_vector_length(1).expect("ips");
        tl.write_u32(IP_PORT_SECRET_CONSTRUCTOR);
        tl.write_i32(0x0102_0304);
        tl.write_i32(443);
        tl.write_bytes(&[0xdd; 17]).expect("secret");
        let tl = tl.into_bytes();

        let real_length = 4 + tl.len();
        assert!(real_length <= SPECIAL_CONFIG_DATA_LENGTH);
        assert!(real_length.is_multiple_of(4));
        let mut data = [0_u8; SPECIAL_CONFIG_DATA_LENGTH];
        data[..4].copy_from_slice(&(real_length as i32).to_le_bytes());
        data[4..real_length].copy_from_slice(&tl);
        let digest = Sha256::digest(data);
        let mut plaintext = Vec::from(data);
        plaintext.extend_from_slice(&digest[..SPECIAL_CONFIG_DIGEST_LENGTH]);

        let key = [0x42_u8; SPECIAL_CONFIG_AES_KEY_LENGTH];
        let iv: [u8; SPECIAL_CONFIG_AES_BLOCK_LENGTH] = key
            [SPECIAL_CONFIG_AES_KEY_LENGTH - SPECIAL_CONFIG_AES_BLOCK_LENGTH..]
            .try_into()
            .expect("iv");
        let ciphertext = aes256_cbc_encrypt_for_fixture(&plaintext, &key, &iv);
        let mut recovered = Vec::from(key);
        recovered.extend_from_slice(&ciphertext);

        let config = decode_recovered_special_config(&recovered).expect("special config");
        assert_eq!(config.date, 10);
        assert_eq!(config.expires, 20);
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].phone_prefix_rules, "+1");
    }

    #[test]
    fn selection_matches_source_prefix_order_and_preserves_endpoint_bits() {
        let config = HelpConfigSimple {
            date: 100,
            expires: 200,
            rules: vec![
                AccessPointRule {
                    phone_prefix_rules: "+1,-1555".into(),
                    dc_id: 1,
                    ips: vec![IpPort::Plain {
                        ipv4: 0x0102_0304,
                        port: -1,
                    }],
                },
                AccessPointRule {
                    phone_prefix_rules: ",-44".into(),
                    dc_id: 2,
                    ips: vec![IpPort::Secret {
                        ipv4: 0x7f00_0001,
                        port: 443,
                        secret: vec![0xdd; 17],
                    }],
                },
            ],
        };
        let selected =
            select_special_config_endpoints(&config, "+1 (212) 555-0100", 50).expect("select");
        assert_eq!(
            selected,
            vec![SpecialConfigEndpoint {
                dc_id: 2,
                ip_address: Ipv4Addr::new(127, 0, 0, 1),
                port: 443,
                secret: vec![0xdd; 17],
            }]
        );
    }

    #[test]
    fn selection_rejects_only_expired_or_empty_rule_sets() {
        let not_yet_started = HelpConfigSimple {
            date: 200,
            expires: 300,
            rules: vec![AccessPointRule {
                phone_prefix_rules: "+1".into(),
                dc_id: 1,
                ips: vec![IpPort::Plain {
                    ipv4: 0x0102_0304,
                    port: 443,
                }],
            }],
        };
        assert_eq!(
            select_special_config_endpoints(&not_yet_started, "+1", 100)
                .expect("source does not reject now < date")[0]
                .ip_address,
            Ipv4Addr::new(1, 2, 3, 4)
        );
        assert!(matches!(
            select_special_config_endpoints(&not_yet_started, "+1", 301),
            Err(SpecialConfigError::Expired { .. })
        ));

        let empty = HelpConfigSimple {
            date: 0,
            expires: i32::MAX,
            rules: Vec::new(),
        };
        assert_eq!(
            select_special_config_endpoints(&empty, "+1", 0),
            Err(SpecialConfigError::EmptyRules)
        );
    }

    #[test]
    fn selection_with_no_matching_rule_is_successfully_empty() {
        let config = HelpConfigSimple {
            date: 0,
            expires: i32::MAX,
            rules: vec![AccessPointRule {
                phone_prefix_rules: "+44".into(),
                dc_id: 1,
                ips: vec![IpPort::Plain {
                    ipv4: 0x0102_0304,
                    port: 443,
                }],
            }],
        };
        assert!(
            select_special_config_endpoints(&config, "+1 212 555 0100", 0)
                .expect("no match is not an error")
                .is_empty()
        );
    }

    #[test]
    fn normalization_and_rsa_domain_fail_closed() {
        assert!(matches!(
            decode_special_config_payload(b"not enough payload"),
            Err(SpecialConfigError::InvalidEncodedLength { .. })
        ));
        let modulus =
            decode_standard_base64(SPECIAL_CONFIG_RSA_MODULUS_BASE64.as_bytes()).expect("modulus");
        assert_eq!(modulus.len(), SPECIAL_CONFIG_RSA_LENGTH);
        assert_eq!(
            raw_rsa_public_recover(&modulus),
            Err(SpecialConfigError::RsaInputOutOfRange)
        );
    }
}
