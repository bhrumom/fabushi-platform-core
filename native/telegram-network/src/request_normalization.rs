use thiserror::Error;

pub const DNS_DOMAINS: [&str; 4] = [
    "google.com",
    "www.google.com",
    "google.ru",
    "www.google.ru",
];

pub const DNS_USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) ",
    "AppleWebKit/537.36 (KHTML, like Gecko) ",
    "Chrome/151.0.0.0 Safari/537.36"
);

pub const DNS_JSON_ACCEPT: &str = "application/dns-json";

const DNS_PADDING_ALPHABET: &[u8; 62] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const DNS_PADDING_MIN: usize = 13;

const FIRESTORE_PROJECT: &str = "reserve-5a846";
const FIRESTORE_CONFIG_KEY: &str = "ipconfig";
const FIRESTORE_CONFIG_SUBKEY: &str = "v3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestProvider {
    Google,
    Mozilla,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedServiceRequest {
    pub scheme: &'static str,
    pub url_host: String,
    pub path: String,
    pub query: Option<String>,
    pub host_override: Option<String>,
    pub accept: Option<&'static str>,
    pub user_agent: &'static str,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RequestNormalizationError {
    #[error("entropy source failed")]
    EntropyUnavailable,
}

pub fn generate_dns_random_padding_with_entropy(
    mut next_byte: impl FnMut() -> Result<u8, RequestNormalizationError>,
) -> Result<String, RequestNormalizationError> {
    let count = loop {
        let candidate = 1 + usize::from(next_byte()?) / 2;
        if candidate >= DNS_PADDING_MIN {
            break candidate;
        }
    };

    let mut result = String::with_capacity(count);
    for _ in 0..count {
        let byte = next_byte()?;
        result.push(DNS_PADDING_ALPHABET[usize::from(byte) % DNS_PADDING_ALPHABET.len()] as char);
    }
    Ok(result)
}

pub fn generate_dns_random_padding() -> Result<String, RequestNormalizationError> {
    generate_dns_random_padding_with_entropy(|| {
        let mut byte = [0_u8; 1];
        getrandom::getrandom(&mut byte).map_err(|_| RequestNormalizationError::EntropyUnavailable)?;
        Ok(byte[0])
    })
}

pub fn normalize_domain_resolver_request(
    provider: RequestProvider,
    endpoint_host: &str,
    host_alias: Option<&str>,
    domain: &str,
    ipv6: bool,
    padding: &str,
) -> NormalizedServiceRequest {
    let dns_type = if ipv6 { 28 } else { 1 };
    match provider {
        RequestProvider::Google => NormalizedServiceRequest {
            scheme: "https",
            url_host: endpoint_host.to_owned(),
            path: "/resolve".to_owned(),
            query: Some(format!(
                "name={domain}&type={dns_type}&random_padding={padding}"
            )),
            host_override: host_alias.map(|alias| format!("{alias}.google.com")),
            accept: None,
            user_agent: DNS_USER_AGENT,
        },
        RequestProvider::Mozilla => NormalizedServiceRequest {
            scheme: "https",
            url_host: endpoint_host.to_owned(),
            path: "/dns-query".to_owned(),
            query: Some(format!(
                "name={domain}&type={dns_type}&random_padding={padding}"
            )),
            host_override: None,
            accept: Some(DNS_JSON_ACCEPT),
            user_agent: DNS_USER_AGENT,
        },
    }
}

pub fn normalize_special_config_doh_request(
    provider: RequestProvider,
    endpoint_host: &str,
    host_alias: Option<&str>,
    domain: &str,
    padding: &str,
) -> NormalizedServiceRequest {
    match provider {
        RequestProvider::Google => NormalizedServiceRequest {
            scheme: "https",
            url_host: endpoint_host.to_owned(),
            path: "/resolve".to_owned(),
            query: Some(format!(
                "name={domain}&type=ANY&random_padding={padding}"
            )),
            host_override: host_alias.map(|alias| format!("{alias}.google.com")),
            accept: None,
            user_agent: DNS_USER_AGENT,
        },
        RequestProvider::Mozilla => NormalizedServiceRequest {
            scheme: "https",
            url_host: endpoint_host.to_owned(),
            path: "/dns-query".to_owned(),
            query: Some(format!(
                "name={domain}&type=16&random_padding={padding}"
            )),
            host_override: None,
            accept: Some(DNS_JSON_ACCEPT),
            user_agent: DNS_USER_AGENT,
        },
    }
}

pub fn normalize_special_config_firestore_request(
    attempt_data: &str,
    attempt_host: Option<&str>,
) -> NormalizedServiceRequest {
    let url_host = match attempt_host {
        Some(_) => attempt_data.to_owned(),
        None => format!("{attempt_data}.googleapis.com"),
    };
    let host_override = attempt_host.map(|service| format!("{service}.googleapis.com"));
    NormalizedServiceRequest {
        scheme: "https",
        url_host,
        path: format!(
            "/v1/projects/{FIRESTORE_PROJECT}/databases/(default)/documents/{FIRESTORE_CONFIG_KEY}/{FIRESTORE_CONFIG_SUBKEY}"
        ),
        query: None,
        host_override,
        accept: None,
        user_agent: DNS_USER_AGENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_rejects_short_counts_and_preserves_source_alphabet_mapping() {
        let mut bytes = [0_u8, 23, 25, 0, 25, 61, 62, 63, 26]
            .into_iter()
            .chain(std::iter::repeat(0));
        let padding = generate_dns_random_padding_with_entropy(|| {
            bytes.next().ok_or(RequestNormalizationError::EntropyUnavailable)
        })
        .unwrap();

        assert_eq!(padding.len(), 13);
        assert_eq!(&padding[..6], "az9abA");
        assert!(padding.chars().all(|ch| ch.is_ascii_alphanumeric()));
    }

    #[test]
    fn padding_accepts_source_maximum_count() {
        let mut first = true;
        let padding = generate_dns_random_padding_with_entropy(|| {
            if first {
                first = false;
                Ok(u8::MAX)
            } else {
                Ok(61)
            }
        })
        .unwrap();
        assert_eq!(padding.len(), 128);
        assert!(padding.chars().all(|ch| ch == '9'));
    }

    #[test]
    fn google_and_mozilla_request_shapes_are_scheduler_free() {
        let google = normalize_domain_resolver_request(
            RequestProvider::Google,
            "google.com",
            Some("dns"),
            "example.com",
            false,
            "pad",
        );
        assert_eq!(google.scheme, "https");
        assert_eq!(google.path, "/resolve");
        assert_eq!(google.query.as_deref(), Some("name=example.com&type=1&random_padding=pad"));
        assert_eq!(google.host_override.as_deref(), Some("dns.google.com"));
        assert_eq!(google.accept, None);

        let mozilla = normalize_domain_resolver_request(
            RequestProvider::Mozilla,
            "mozilla.cloudflare-dns.com",
            None,
            "example.com",
            true,
            "pad",
        );
        assert_eq!(mozilla.path, "/dns-query");
        assert_eq!(mozilla.query.as_deref(), Some("name=example.com&type=28&random_padding=pad"));
        assert_eq!(mozilla.host_override, None);
        assert_eq!(mozilla.accept, Some(DNS_JSON_ACCEPT));
    }

    #[test]
    fn special_config_doh_and_firestore_shapes_match_fixed_u_components() {
        let google = normalize_special_config_doh_request(
            RequestProvider::Google,
            "dns.google.com",
            None,
            "apv3.stel.com",
            "xyz",
        );
        assert_eq!(
            google.query.as_deref(),
            Some("name=apv3.stel.com&type=ANY&random_padding=xyz")
        );

        let mozilla = normalize_special_config_doh_request(
            RequestProvider::Mozilla,
            "mozilla.cloudflare-dns.com",
            None,
            "apv3.stel.com",
            "xyz",
        );
        assert_eq!(
            mozilla.query.as_deref(),
            Some("name=apv3.stel.com&type=16&random_padding=xyz")
        );
        assert_eq!(mozilla.accept, Some(DNS_JSON_ACCEPT));

        let direct = normalize_special_config_firestore_request("firestore", None);
        assert_eq!(direct.url_host, "firestore.googleapis.com");
        assert_eq!(direct.host_override, None);

        let camouflage =
            normalize_special_config_firestore_request("google.com", Some("firestore"));
        assert_eq!(camouflage.url_host, "google.com");
        assert_eq!(
            camouflage.host_override.as_deref(),
            Some("firestore.googleapis.com")
        );
    }
}
