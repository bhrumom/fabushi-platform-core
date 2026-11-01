use crate::request_normalization::{
    generate_dns_random_padding, normalize_special_config_doh_request,
    normalize_special_config_firestore_request, NormalizedServiceRequest,
    RequestNormalizationError, RequestProvider, DNS_DOMAINS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialConfigAttemptKind {
    Google,
    Mozilla,
    Firestore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialConfigAttempt {
    pub kind: SpecialConfigAttemptKind,
    pub data: String,
    pub host: Option<String>,
}

impl SpecialConfigAttempt {
    pub fn normalized_request(
        &self,
        domain: &str,
    ) -> Result<NormalizedServiceRequest, RequestNormalizationError> {
        match self.kind {
            SpecialConfigAttemptKind::Google => Ok(normalize_special_config_doh_request(
                RequestProvider::Google,
                &self.data,
                self.host.as_deref(),
                domain,
                &generate_dns_random_padding()?,
            )),
            SpecialConfigAttemptKind::Mozilla => Ok(normalize_special_config_doh_request(
                RequestProvider::Mozilla,
                &self.data,
                self.host.as_deref(),
                domain,
                &generate_dns_random_padding()?,
            )),
            SpecialConfigAttemptKind::Firestore => Ok(
                normalize_special_config_firestore_request(&self.data, self.host.as_deref()),
            ),
        }
    }

    pub fn is_dns_txt(&self) -> bool {
        matches!(
            self.kind,
            SpecialConfigAttemptKind::Google | SpecialConfigAttemptKind::Mozilla
        )
    }
}

pub fn seed_special_config_attempts(
    test_mode: bool,
) -> Result<Vec<SpecialConfigAttempt>, RequestNormalizationError> {
    seed_special_config_attempts_with_entropy(test_mode, secure_random_u64)
}

pub fn seed_special_config_attempts_with_entropy(
    test_mode: bool,
    mut next_u64: impl FnMut() -> Result<u64, RequestNormalizationError>,
) -> Result<Vec<SpecialConfigAttempt>, RequestNormalizationError> {
    let mut attempts = vec![
        SpecialConfigAttempt {
            kind: SpecialConfigAttemptKind::Google,
            data: "dns.google.com".to_string(),
            host: None,
        },
        SpecialConfigAttempt {
            kind: SpecialConfigAttemptKind::Mozilla,
            data: "mozilla.cloudflare-dns.com".to_string(),
            host: None,
        },
    ];
    shuffle_range(&mut attempts, 0, 2, &mut next_u64)?;

    if !test_mode {
        let firestore_start = attempts.len();
        attempts.push(SpecialConfigAttempt {
            kind: SpecialConfigAttemptKind::Firestore,
            data: "firestore".to_string(),
            host: None,
        });
        for domain in DNS_DOMAINS {
            attempts.push(SpecialConfigAttempt {
                kind: SpecialConfigAttemptKind::Firestore,
                data: domain.to_string(),
                host: Some("firestore".to_string()),
            });
        }
        let firestore_end = attempts.len();
        shuffle_range(
            &mut attempts,
            firestore_start,
            firestore_end,
            &mut next_u64,
        )?;
    }

    Ok(attempts)
}

pub fn secure_random_index(upper_exclusive: usize) -> Result<usize, RequestNormalizationError> {
    if upper_exclusive <= 1 {
        return Ok(0);
    }
    let value = secure_random_u64()?;
    Ok((value % upper_exclusive as u64) as usize)
}

fn secure_random_u64() -> Result<u64, RequestNormalizationError> {
    let mut bytes = [0_u8; 8];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| RequestNormalizationError::EntropyUnavailable)?;
    Ok(u64::from_le_bytes(bytes))
}

fn shuffle_range(
    values: &mut [SpecialConfigAttempt],
    from: usize,
    till: usize,
    next_u64: &mut impl FnMut() -> Result<u64, RequestNormalizationError>,
) -> Result<(), RequestNormalizationError> {
    if till <= from + 1 {
        return Ok(());
    }
    for index in (from + 1..till).rev() {
        let width = index - from + 1;
        let offset = (next_u64()? % width as u64) as usize;
        values.swap(index, from + offset);
    }
    Ok(())
}

#[cfg(test)]
mod contracts {
    use super::*;

    #[test]
    fn test_mode_seeds_only_doh_and_keeps_the_group_randomizable() {
        let attempts =
            seed_special_config_attempts_with_entropy(true, || Ok(0)).unwrap();
        assert_eq!(attempts.len(), 2);
        assert!(attempts.iter().all(SpecialConfigAttempt::is_dns_txt));
        assert_eq!(attempts[0].kind, SpecialConfigAttemptKind::Mozilla);
        assert_eq!(attempts[1].kind, SpecialConfigAttemptKind::Google);
    }

    #[test]
    fn production_mode_keeps_doh_group_before_firestore_group() {
        let mut entropy = [1_u64, 3, 2, 1, 0].into_iter();
        let attempts = seed_special_config_attempts_with_entropy(false, || {
            Ok(entropy.next().unwrap_or(0))
        })
        .unwrap();

        assert_eq!(attempts.len(), 7);
        assert!(attempts[..2].iter().all(SpecialConfigAttempt::is_dns_txt));
        assert!(attempts[2..]
            .iter()
            .all(|attempt| attempt.kind == SpecialConfigAttemptKind::Firestore));
        assert_eq!(
            attempts[2..]
                .iter()
                .filter(|attempt| attempt.host.as_deref() == Some("firestore"))
                .count(),
            DNS_DOMAINS.len()
        );
        assert_eq!(
            attempts[2..]
                .iter()
                .filter(|attempt| attempt.host.is_none())
                .count(),
            1
        );
    }

    #[test]
    fn request_normalization_is_selected_by_attempt_kind() {
        let google = SpecialConfigAttempt {
            kind: SpecialConfigAttemptKind::Google,
            data: "dns.google.com".into(),
            host: None,
        };
        let firestore = SpecialConfigAttempt {
            kind: SpecialConfigAttemptKind::Firestore,
            data: "google.com".into(),
            host: Some("firestore".into()),
        };
        assert!(google.is_dns_txt());
        assert!(!firestore.is_dns_txt());
        let request = normalize_special_config_firestore_request(
            &firestore.data,
            firestore.host.as_deref(),
        );
        assert_eq!(request.url_host, "google.com");
        assert_eq!(
            request.host_override.as_deref(),
            Some("firestore.googleapis.com")
        );
    }

    #[test]
    fn secure_random_index_has_a_stable_zero_case() {
        assert_eq!(secure_random_index(0).unwrap(), 0);
        assert_eq!(secure_random_index(1).unwrap(), 0);
    }
}
