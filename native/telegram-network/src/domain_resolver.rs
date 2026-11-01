use crate::{
    generate_dns_random_padding, normalize_domain_resolver_request, DirectHttpsRace, NetworkError,
    NormalizedServiceRequest, RequestProvider, StaggerSettlementPolicy, DNS_DOMAINS,
};
use fabushi_telegram_protocol::normalize_dns_response;
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

const RESOLVER_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Clone, Default)]
pub struct ProxyResolutionCache {
    host: Option<String>,
    ips: Vec<IpAddr>,
    expires_at: Option<Instant>,
}

impl ProxyResolutionCache {
    pub fn clear(&mut self) {
        self.host = None;
        self.ips.clear();
        self.expires_at = None;
    }

    pub fn candidates(
        &mut self,
        host: &str,
        prefer_ipv6: bool,
    ) -> Result<Vec<IpAddr>, NetworkError> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![ip]);
        }
        let fresh = self.host.as_deref() == Some(host)
            && self.expires_at.is_some_and(|expires| expires > Instant::now())
            && !self.ips.is_empty();
        if !fresh {
            let resolved = resolve_proxy_host(host)?;
            self.host = Some(host.to_owned());
            self.ips = resolved.ips;
            self.expires_at = Some(Instant::now() + resolved.ttl);
        }
        let mut ips = self.ips.clone();
        ips.sort_by_key(|ip| ip.is_ipv6() != prefer_ipv6);
        Ok(ips)
    }

    pub fn promote(&mut self, host: &str, ip: IpAddr) {
        if self.host.as_deref() != Some(host) {
            return;
        }
        if let Some(index) = self.ips.iter().position(|value| *value == ip) {
            self.ips[..=index].rotate_right(1);
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> &[IpAddr] {
        &self.ips
    }
}

struct ResolvedProxyHost {
    ips: Vec<IpAddr>,
    ttl: Duration,
}

fn resolve_proxy_host(host: &str) -> Result<ResolvedProxyHost, NetworkError> {
    if host.trim() != host || host.is_empty() {
        return Err(NetworkError::InvalidProxyEndpoint);
    }
    let v4 = resolve_family(host, false)?;
    if v4.0.is_empty() {
        return Err(NetworkError::ProxyResolutionFailed);
    }
    let v6 = resolve_family(host, true).unwrap_or_default();
    let mut ips = v4.0;
    ips.extend(v6.0);
    let ttl = match (v4.1, v6.1) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        _ => Duration::from_secs(10),
    };
    Ok(ResolvedProxyHost { ips, ttl })
}

fn resolve_family(
    host: &str,
    ipv6: bool,
) -> Result<(Vec<IpAddr>, Option<Duration>), NetworkError> {
    let requests = resolver_requests(host, ipv6)?;
    let mut race = DirectHttpsRace::start(
        StaggerSettlementPolicy::DomainResolver,
        requests,
        RESOLVER_ATTEMPT_TIMEOUT,
    );
    while let Ok(reply) = race.recv() {
        let Ok(response) = reply.result else {
            race.finalize_reply(reply.attempt_index);
            continue;
        };
        let family = if ipv6 { 28 } else { 1 };
        let Some(normalized) = normalize_dns_response(&response.body, Some(family)) else {
            race.finalize_reply(reply.attempt_index);
            continue;
        };
        let ips: Vec<IpAddr> = normalized
            .data
            .into_iter()
            .filter_map(|value| value.parse::<IpAddr>().ok())
            .filter(|ip| ip.is_ipv6() == ipv6)
            .collect();
        if ips.is_empty() {
            race.finalize_reply(reply.attempt_index);
            continue;
        }
        race.accept_reply(reply.attempt_index);
        return Ok((
            ips,
            Some(Duration::from_millis(
                u64::try_from(normalized.ttl_millis).unwrap_or(10_000),
            )),
        ));
    }
    Ok((Vec::new(), None))
}

fn resolver_requests(
    host: &str,
    ipv6: bool,
) -> Result<Vec<NormalizedServiceRequest>, NetworkError> {
    let mut domains = DNS_DOMAINS.to_vec();
    let camouflage = domains.remove(0);
    let mut attempts = vec![
        normalize_domain_resolver_request(
            RequestProvider::Google,
            "dns.google.com",
            None,
            host,
            ipv6,
            &generate_dns_random_padding()
                .map_err(|_| NetworkError::ProxyResolutionFailed)?,
        ),
        normalize_domain_resolver_request(
            RequestProvider::Google,
            camouflage,
            Some("dns"),
            host,
            ipv6,
            &generate_dns_random_padding()
                .map_err(|_| NetworkError::ProxyResolutionFailed)?,
        ),
        normalize_domain_resolver_request(
            RequestProvider::Mozilla,
            "mozilla.cloudflare-dns.com",
            None,
            host,
            ipv6,
            &generate_dns_random_padding()
                .map_err(|_| NetworkError::ProxyResolutionFailed)?,
        ),
    ];
    for domain in domains {
        attempts.push(normalize_domain_resolver_request(
            RequestProvider::Google,
            domain,
            Some("dns"),
            host,
            ipv6,
            &generate_dns_random_padding()
                .map_err(|_| NetworkError::ProxyResolutionFailed)?,
        ));
    }
    Ok(attempts)
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn successful_proxy_ip_promotion_is_transient_and_stable() {
        let mut cache = ProxyResolutionCache {
            host: Some("proxy.example".into()),
            ips: vec![
                "192.0.2.1".parse().unwrap(),
                "192.0.2.2".parse().unwrap(),
                "2001:db8::1".parse().unwrap(),
            ],
            expires_at: Some(Instant::now() + Duration::from_secs(60)),
        };
        cache.promote("proxy.example", "192.0.2.2".parse().unwrap());
        assert_eq!(
            cache.snapshot(),
            &[
                "192.0.2.2".parse().unwrap(),
                "192.0.2.1".parse().unwrap(),
                "2001:db8::1".parse().unwrap(),
            ]
        );
    }

    #[test]
    fn resolver_request_shape_uses_no_proxy_https_normalization() {
        let requests = resolver_requests("proxy.example", false).unwrap();
        assert_eq!(requests.len(), 6);
        assert_eq!(requests[0].url_host, "dns.google.com");
        assert_eq!(requests[0].path, "/resolve");
        assert_eq!(requests[2].url_host, "mozilla.cloudflare-dns.com");
        assert_eq!(requests[2].accept, Some("application/dns-json"));
    }
}
