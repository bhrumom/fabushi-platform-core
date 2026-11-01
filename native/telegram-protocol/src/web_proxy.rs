use hmac::{Hmac, Mac};
use sha2::Sha256;
use thiserror::Error;

const ROOT_CONTEXT: &[u8] = b"tdesktop-web-proxy-bridge-v1\n";
const PATH_CONTEXT: &[u8] = b"tdesktop-web-proxy-bridge-v2\n";
const LINK_SECRET_MARKER: u8 = 0x70;
const MAX_BASE_PATH_BYTES: usize = 128;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WebProxyConfigError {
    #[error("WebProxy host is not canonical")]
    InvalidHost,
    #[error("WebProxy base path is invalid")]
    InvalidBasePath,
    #[error("WebProxy secret text is malformed")]
    InvalidSecret,
    #[error("WebProxy secret type is unsupported")]
    UnsupportedSecret,
    #[error("path-bearing WebProxy links require a marked secret")]
    UnmarkedPathSecret,
}

pub fn normalize_web_proxy_base_path(value: &str) -> Result<String, WebProxyConfigError> {
    let mut result = value.trim();
    if let Some(stripped) = result.strip_prefix('/') {
        result = stripped;
    }
    if let Some(stripped) = result.strip_suffix('/') {
        result = stripped;
    }
    if result.is_empty() {
        return Ok(String::new());
    }
    if result.len() > MAX_BASE_PATH_BYTES || !result.is_ascii() {
        return Err(WebProxyConfigError::InvalidBasePath);
    }

    let mut segment_start = true;
    for byte in result.bytes() {
        if byte == b'/' {
            if segment_start {
                return Err(WebProxyConfigError::InvalidBasePath);
            }
            segment_start = true;
            continue;
        }
        let alphanumeric = byte.is_ascii_alphanumeric();
        let extra = matches!(byte, b'-' | b'_');
        if !alphanumeric && !(extra && !segment_start) {
            return Err(WebProxyConfigError::InvalidBasePath);
        }
        segment_start = false;
    }
    if segment_start {
        return Err(WebProxyConfigError::InvalidBasePath);
    }
    Ok(result.to_owned())
}

pub fn validate_canonical_web_proxy_host(host: &str) -> Result<(), WebProxyConfigError> {
    if host.is_empty()
        || host.len() > 253
        || host.trim() != host
        || !host.is_ascii()
        || host.bytes().any(|byte| byte.is_ascii_uppercase())
        || host.contains([':', '/', '?', '#', '@'])
        || host.ends_with('.')
        || !host.contains('.')
    {
        return Err(WebProxyConfigError::InvalidHost);
    }

    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(WebProxyConfigError::InvalidHost);
        }
    }

    let last = host.rsplit('.').next().expect("host contains a label");
    let numeric = if let Some(hex) = last.strip_prefix("0x") {
        !hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    } else {
        last.bytes().all(|byte| byte.is_ascii_digit())
    };
    if numeric {
        return Err(WebProxyConfigError::InvalidHost);
    }
    Ok(())
}

pub fn parse_web_proxy_secret_text(value: &str) -> Result<Vec<u8>, WebProxyConfigError> {
    let decoded = if value.len() >= 32
        && value.len().is_multiple_of(2)
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        hex::decode(value).map_err(|_| WebProxyConfigError::InvalidSecret)?
    } else {
        decode_base64url(value)?
    };

    match decoded.as_slice() {
        bytes if bytes.len() == 16 => Ok(decoded),
        bytes if bytes.len() == 17 && bytes[0] == 0xdd => Ok(decoded),
        bytes if bytes.len() >= 21 && bytes[0] == 0xee => {
            Err(WebProxyConfigError::UnsupportedSecret)
        }
        _ => Err(WebProxyConfigError::InvalidSecret),
    }
}

pub fn web_proxy_bridge_capability(
    host: &str,
    base_path: &str,
    secret_text: &str,
) -> Result<String, WebProxyConfigError> {
    validate_canonical_web_proxy_host(host)?;
    let path = normalize_web_proxy_base_path(base_path)?;
    let key = parse_web_proxy_secret_text(secret_text)?;

    let mut context = Vec::with_capacity(
        if path.is_empty() {
            ROOT_CONTEXT.len() + host.len()
        } else {
            PATH_CONTEXT.len() + host.len() + 1 + path.len()
        },
    );
    if path.is_empty() {
        context.extend_from_slice(ROOT_CONTEXT);
        context.extend_from_slice(host.as_bytes());
    } else {
        context.extend_from_slice(PATH_CONTEXT);
        context.extend_from_slice(host.as_bytes());
        context.push(b'\n');
        context.extend_from_slice(path.as_bytes());
    }

    let mut mac =
        HmacSha256::new_from_slice(&key).map_err(|_| WebProxyConfigError::InvalidSecret)?;
    mac.update(&context);
    Ok(encode_base64url_no_pad(&mac.finalize().into_bytes()))
}

pub fn web_proxy_bridge_path(base_path: &str) -> Result<String, WebProxyConfigError> {
    let path = normalize_web_proxy_base_path(base_path)?;
    Ok(if path.is_empty() {
        "/".to_owned()
    } else {
        format!("/{path}/")
    })
}

pub fn web_proxy_bridge_url(
    host: &str,
    base_path: &str,
    secret_text: &str,
) -> Result<String, WebProxyConfigError> {
    validate_canonical_web_proxy_host(host)?;
    let path = normalize_web_proxy_base_path(base_path)?;
    let capability = web_proxy_bridge_capability(host, &path, secret_text)?;
    let bridge_path = if path.is_empty() {
        "/".to_owned()
    } else {
        format!("/{path}/")
    };
    Ok(format!("https://{host}{bridge_path}?bridge={capability}"))
}

pub fn encode_web_proxy_link_secret(
    base_path: &str,
    secret_text: &str,
) -> Result<String, WebProxyConfigError> {
    let path = normalize_web_proxy_base_path(base_path)?;
    if path.is_empty() {
        return Ok(secret_text.to_owned());
    }
    let secret = parse_web_proxy_secret_text(secret_text)?;
    let mut marked = Vec::with_capacity(1 + secret.len());
    marked.push(LINK_SECRET_MARKER);
    marked.extend_from_slice(&secret);
    Ok(encode_base64url_no_pad(&marked))
}

pub fn decode_web_proxy_link_secret(
    value: &str,
    has_base_path: bool,
) -> Result<String, WebProxyConfigError> {
    if let Ok(decoded) = decode_base64url(value) {
        if decoded.len() >= 17 && decoded[0] == LINK_SECRET_MARKER {
            return Ok(hex::encode(&decoded[1..]));
        }
    }
    if has_base_path {
        Err(WebProxyConfigError::UnmarkedPathSecret)
    } else {
        Ok(value.to_owned())
    }
}

fn encode_base64url_no_pad(value: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut output = String::with_capacity((value.len() * 4 + 2) / 3);
    for chunk in value.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied();
        let c = chunk.get(2).copied();
        output.push(ALPHABET[(a >> 2) as usize] as char);
        output.push(ALPHABET[(((a & 0x03) << 4) | b.unwrap_or(0) >> 4) as usize] as char);
        if let Some(b) = b {
            output.push(ALPHABET[(((b & 0x0f) << 2) | c.unwrap_or(0) >> 6) as usize] as char);
        }
        if let Some(c) = c {
            output.push(ALPHABET[(c & 0x3f) as usize] as char);
        }
    }
    output
}

fn decode_base64url(value: &str) -> Result<Vec<u8>, WebProxyConfigError> {
    if value.is_empty() || !value.is_ascii() {
        return Err(WebProxyConfigError::InvalidSecret);
    }
    let bytes = value.as_bytes();
    let padding = bytes.iter().rev().take_while(|&&byte| byte == b'=').count();
    if padding > 2
        || bytes[..bytes.len() - padding].contains(&b'=')
        || bytes.len() - padding < 2
    {
        return Err(WebProxyConfigError::InvalidSecret);
    }
    let inner = &bytes[..bytes.len() - padding];
    if inner.len() % 4 == 1 {
        return Err(WebProxyConfigError::InvalidSecret);
    }

    let mut output = Vec::with_capacity(inner.len() * 3 / 4);
    let mut accumulator = 0_u32;
    let mut bits = 0_u8;
    for &byte in inner {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err(WebProxyConfigError::InvalidSecret),
        };
        accumulator = (accumulator << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push(((accumulator >> bits) & 0xff) as u8);
        }
    }
    if bits != 0 && (accumulator & ((1_u32 << bits) - 1)) != 0 {
        return Err(WebProxyConfigError::InvalidSecret);
    }
    Ok(output)
}
