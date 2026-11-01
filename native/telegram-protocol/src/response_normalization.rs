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

pub fn parse_http_date_unix_seconds(date: &str) -> Option<i64> {
    let chars: Vec<char> = date.chars().collect();
    for start in 0..chars.len() {
        let tail = &chars[start..];
        if tail.len() < 29 {
            break;
        }
        if let Some(value) = parse_http_date_at(tail) {
            return Some(value);
        }
    }
    None
}

fn parse_http_date_at(input: &[char]) -> Option<i64> {
    if input.len() < 29 {
        return None;
    }
    let input = &input[..29];
    if !input[..3].iter().all(|ch| is_regex_word_char(*ch))
        || input[3] != ','
        || input[4] != ' '
        || input[7] != ' '
        || input[11] != ' '
        || input[16] != ' '
        || input[19] != ':'
        || input[22] != ':'
        || input[25] != ' '
        || input[26] != 'G'
        || input[27] != 'M'
        || input[28] != 'T'
    {
        return None;
    }

    let day = parse_ascii_digits(&input[5..7])?;
    let month_text: String = input[8..11].iter().collect();
    let month = match month_text.as_str() {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = parse_ascii_digits(&input[12..16])?;
    let hour = parse_ascii_digits(&input[17..19])?;
    let minute = parse_ascii_digits(&input[20..22])?;
    let second = parse_ascii_digits(&input[23..25])?;

    if year == 0
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }

    let days = days_from_civil(year as i64, month as i64, day as i64);
    Some(
        days.saturating_mul(86_400)
            .saturating_add(i64::from(hour) * 3_600)
            .saturating_add(i64::from(minute) * 60)
            .saturating_add(i64::from(second)),
    )
}

fn parse_ascii_digits(input: &[char]) -> Option<u32> {
    input.iter().try_fold(0_u32, |value, ch| {
        if !ch.is_ascii_digit() {
            return None;
        }
        Some(value * 10 + u32::from((*ch as u8) - b'0'))
    })
}

fn is_regex_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
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


#[cfg(test)]
mod http_date_tests {
    use super::parse_http_date_unix_seconds;

    #[test]
    fn http_date_parser_matches_fixed_u_shape_without_clock_mutation() {
        assert_eq!(
            parse_http_date_unix_seconds("Wed, 10 Jul 2019 14:33:38 GMT"),
            Some(1_562_769_218)
        );
        assert_eq!(
            parse_http_date_unix_seconds("prefix Wed, 10 Jul 2019 14:33:38 GMT suffix"),
            Some(1_562_769_218)
        );
        assert_eq!(
            parse_http_date_unix_seconds("Thu, 29 Feb 2024 00:00:00 GMT"),
            Some(1_709_164_800)
        );
    }

    #[test]
    fn http_date_parser_rejects_invalid_month_date_and_time() {
        for invalid in [
            "Wed, 10 Jly 2019 14:33:38 GMT",
            "Wed, 31 Apr 2019 14:33:38 GMT",
            "Wed, 29 Feb 2019 14:33:38 GMT",
            "Wed, 10 Jul 2019 24:00:00 GMT",
            "Wed, 10 Jul 2019 23:60:00 GMT",
            "Wed, 10 Jul 2019 23:59:60 GMT",
            "Wed, 10 Jul 0000 14:33:38 GMT",
        ] {
            assert_eq!(parse_http_date_unix_seconds(invalid), None, "{invalid}");
        }
    }
}
