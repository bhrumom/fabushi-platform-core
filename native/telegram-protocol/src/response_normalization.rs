use serde_json::Value;

const DNS_MIN_TTL_MILLIS: i64 = 10_000;
const DNS_MAX_TTL_MILLIS: i64 = 300_000;
const DNS_TXT_TYPE: i32 = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsEntry {
    pub data: String,
    pub ttl_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDnsResponse {
    pub data: Vec<String>,
    pub ttl_millis: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialConfigResponseKind {
    DnsTxt,
    Firestore,
}

pub fn parse_dns_response(bytes: &[u8], type_restriction: Option<i32>) -> Vec<DnsEntry> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let Ok(document) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let Some(response) = document.as_object() else {
        return Vec::new();
    };
    let Some(answer) = response.get("Answer").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for element in answer {
        let Some(object) = element.as_object() else {
            continue;
        };
        if let Some(expected) = type_restriction {
            let Some(value) = object.get("type").and_then(Value::as_f64) else {
                continue;
            };
            if safe_round(value) != i64::from(expected) {
                continue;
            }
        }
        let Some(data) = object.get("data").and_then(Value::as_str) else {
            continue;
        };
        let ttl_seconds = object
            .get("TTL")
            .and_then(Value::as_f64)
            .map(safe_round)
            .unwrap_or(0);
        result.push(DnsEntry {
            data: data.to_owned(),
            ttl_seconds,
        });
    }
    result
}

pub fn normalize_dns_response(
    bytes: &[u8],
    type_restriction: Option<i32>,
) -> Option<NormalizedDnsResponse> {
    let entries = parse_dns_response(bytes, type_restriction);
    if entries.is_empty() {
        return None;
    }
    let mut ttl_millis = DNS_MAX_TTL_MILLIS;
    let mut data = Vec::with_capacity(entries.len());
    for entry in entries {
        data.push(entry.data);
        let bounded = entry
            .ttl_seconds
            .saturating_mul(1000)
            .max(DNS_MIN_TTL_MILLIS);
        ttl_millis = ttl_millis.min(bounded);
    }
    Some(NormalizedDnsResponse { data, ttl_millis })
}

pub fn normalize_special_config_response(
    kind: SpecialConfigResponseKind,
    bytes: &[u8],
) -> Vec<u8> {
    match kind {
        SpecialConfigResponseKind::DnsTxt => concatenate_dns_txt_fields(parse_dns_response(
            bytes,
            Some(DNS_TXT_TYPE),
        )),
        SpecialConfigResponseKind::Firestore => parse_firestore_response(bytes),
    }
}

fn concatenate_dns_txt_fields(entries: Vec<DnsEntry>) -> Vec<u8> {
    let mut ordered: Vec<(usize, usize, String)> = entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| (utf16_len(&entry.data), index, entry.data))
        .collect();
    ordered.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
    });
    let mut joined = String::new();
    for (_, _, data) in ordered {
        joined.push_str(&data);
    }
    qstring_to_latin1(&joined)
}

fn parse_firestore_response(bytes: &[u8]) -> Vec<u8> {
    let Ok(document) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let Some(root) = document.as_object() else {
        return Vec::new();
    };
    let Some(value) = root
        .get("fields")
        .and_then(Value::as_object)
        .and_then(|fields| fields.get("data"))
        .and_then(Value::as_object)
        .and_then(|data| data.get("stringValue"))
        .and_then(Value::as_str)
    else {
        return Vec::new();
    };
    qstring_to_latin1(value)
}

fn qstring_to_latin1(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .map(|unit| u8::try_from(unit).unwrap_or(b'?'))
        .collect()
}

fn utf16_len(value: &str) -> usize {
    value.encode_utf16().count()
}

fn safe_round(value: f64) -> i64 {
    if value.is_nan() {
        0
    } else if value >= i64::MAX as f64 {
        i64::MAX
    } else if value <= i64::MIN as f64 {
        i64::MIN
    } else {
        value.round() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doh_parser_filters_entries_and_preserves_source_order() {
        let response = br#"{
            "Answer": [
                {"type": 1, "data": "1.1.1.1", "TTL": 600},
                {"type": 16, "data": "short", "TTL": 0},
                {"type": "16", "data": "wrong-type", "TTL": 20},
                {"type": 16, "data": "longer", "TTL": 20.6},
                {"type": 16, "TTL": 30},
                null
            ]
        }"#;
        assert_eq!(
            parse_dns_response(response, Some(16)),
            vec![
                DnsEntry {
                    data: "short".into(),
                    ttl_seconds: 0,
                },
                DnsEntry {
                    data: "longer".into(),
                    ttl_seconds: 21,
                },
            ]
        );
        assert_eq!(
            normalize_dns_response(response, Some(16)),
            Some(NormalizedDnsResponse {
                data: vec!["short".into(), "longer".into()],
                ttl_millis: 10_000,
            })
        );
    }

    #[test]
    fn doh_structural_failures_are_empty_and_ttl_is_source_bounded() {
        for malformed in [
            b"".as_slice(),
            b"not-json".as_slice(),
            br#"[]"#.as_slice(),
            br#"{}"#.as_slice(),
            br#"{"Answer":{}}"#.as_slice(),
        ] {
            assert!(parse_dns_response(malformed, None).is_empty());
            assert_eq!(normalize_dns_response(malformed, None), None);
        }
        let response =
            br#"{"Answer":[{"data":"a","TTL":9999},{"data":"b","TTL":20}]}"#;
        assert_eq!(
            normalize_dns_response(response, None),
            Some(NormalizedDnsResponse {
                data: vec!["a".into(), "b".into()],
                ttl_millis: 20_000,
            })
        );
    }

    #[test]
    fn special_config_txt_matches_qmultimap_order_and_latin1_projection() {
        let response = br#"{"Answer":[
            {"type":16,"data":"aa"},
            {"type":16,"data":"bb"},
            {"type":1,"data":"ignored"},
            {"type":16,"data":"cccc"},
            {"type":16,"data":"é😀"}
        ]}"#;
        assert_eq!(
            normalize_special_config_response(SpecialConfigResponseKind::DnsTxt, response),
            b"cccc???bbaa"
        );
    }

    #[test]
    fn firestore_adapter_extracts_nested_string_and_fails_closed() {
        assert_eq!(
            normalize_special_config_response(
                SpecialConfigResponseKind::Firestore,
                br#"{"fields":{"data":{"stringValue":"abé😀"}}}"#,
            ),
            b"abé??"
        );
        for malformed in [
            b"not-json".as_slice(),
            br#"[]"#.as_slice(),
            br#"{}"#.as_slice(),
            br#"{"fields":{"data":{"stringValue":17}}}"#.as_slice(),
        ] {
            assert!(
                normalize_special_config_response(SpecialConfigResponseKind::Firestore, malformed)
                    .is_empty()
            );
        }
    }
}
