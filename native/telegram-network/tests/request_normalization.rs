use fabushi_telegram_network::{
    generate_dns_random_padding_with_entropy, normalize_domain_resolver_request,
    normalize_special_config_doh_request, normalize_special_config_firestore_request,
    RequestNormalizationError, RequestProvider, DNS_DOMAINS, DNS_JSON_ACCEPT, DNS_USER_AGENT,
};

#[test]
fn doh_request_normalization_preserves_fixed_u_constants_and_components() {
    assert_eq!(
        DNS_DOMAINS,
        ["google.com", "www.google.com", "google.ru", "www.google.ru"]
    );
    assert_eq!(
        DNS_USER_AGENT,
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36"
    );

    let google = normalize_domain_resolver_request(
        RequestProvider::Google,
        "google.com",
        Some("dns"),
        "example.com",
        false,
        "Ab9",
    );
    assert_eq!(google.scheme, "https");
    assert_eq!(google.url_host, "google.com");
    assert_eq!(google.path, "/resolve");
    assert_eq!(
        google.query.as_deref(),
        Some("name=example.com&type=1&random_padding=Ab9")
    );
    assert_eq!(google.host_override.as_deref(), Some("dns.google.com"));
    assert_eq!(google.accept, None);
    assert_eq!(google.user_agent, DNS_USER_AGENT);

    let mozilla = normalize_domain_resolver_request(
        RequestProvider::Mozilla,
        "mozilla.cloudflare-dns.com",
        None,
        "example.com",
        true,
        "Ab9",
    );
    assert_eq!(mozilla.path, "/dns-query");
    assert_eq!(
        mozilla.query.as_deref(),
        Some("name=example.com&type=28&random_padding=Ab9")
    );
    assert_eq!(mozilla.host_override, None);
    assert_eq!(mozilla.accept, Some(DNS_JSON_ACCEPT));
}

#[test]
fn special_config_request_normalization_preserves_doh_and_firestore_shapes() {
    let google = normalize_special_config_doh_request(
        RequestProvider::Google,
        "dns.google.com",
        None,
        "apv3.stel.com",
        "pad",
    );
    assert_eq!(
        google.query.as_deref(),
        Some("name=apv3.stel.com&type=ANY&random_padding=pad")
    );

    let mozilla = normalize_special_config_doh_request(
        RequestProvider::Mozilla,
        "mozilla.cloudflare-dns.com",
        None,
        "apv3.stel.com",
        "pad",
    );
    assert_eq!(
        mozilla.query.as_deref(),
        Some("name=apv3.stel.com&type=16&random_padding=pad")
    );
    assert_eq!(mozilla.accept, Some(DNS_JSON_ACCEPT));

    let direct = normalize_special_config_firestore_request("firestore", None);
    assert_eq!(direct.url_host, "firestore.googleapis.com");
    assert_eq!(
        direct.path,
        "/v1/projects/reserve-5a846/databases/(default)/documents/ipconfig/v3"
    );
    assert_eq!(direct.host_override, None);

    let camouflage =
        normalize_special_config_firestore_request("www.google.ru", Some("firestore"));
    assert_eq!(camouflage.url_host, "www.google.ru");
    assert_eq!(
        camouflage.host_override.as_deref(),
        Some("firestore.googleapis.com")
    );
}

#[test]
fn dns_padding_contract_preserves_rejection_range_and_modulo_alphabet() {
    let mut source = vec![0_u8, 23, 0, 25, 61, 62, 63, 124];
    source.extend(std::iter::repeat(0_u8).take(16));
    let mut source = source.into_iter();

    let padding = generate_dns_random_padding_with_entropy(|| {
        source.next().ok_or(RequestNormalizationError::EntropyUnavailable)
    })
    .unwrap();
    assert_eq!(padding.len(), 13);
    assert_eq!(&padding[..6], "az9abA");
    assert!(padding.chars().all(|ch| ch.is_ascii_alphanumeric()));

    let mut first = true;
    let maximum = generate_dns_random_padding_with_entropy(|| {
        if first {
            first = false;
            Ok(u8::MAX)
        } else {
            Ok(61)
        }
    })
    .unwrap();
    assert_eq!(maximum.len(), 128);
    assert!(maximum.chars().all(|ch| ch == '9'));
}
