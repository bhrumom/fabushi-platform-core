use serde_json::Map;
use serde_json::Value;
use sha2::Digest;
use sha2::Sha256;

/// Serialize JSON with recursively sorted object keys so content digests are
/// identical across native clients and Cloudflare's WebAssembly target.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&canonical_json_value(value))
}

pub fn canonical_json_sha256(value: &Value) -> Result<String, serde_json::Error> {
    canonical_json_bytes(value).map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}

fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json_value).collect()),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut canonical = Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_json_value(&values[key]));
            }
            Value::Object(canonical)
        }
        scalar => scalar.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::canonical_json_bytes;
    use super::canonical_json_sha256;
    use serde_json::json;

    #[test]
    fn recursively_normalizes_object_order_for_cross_target_digests() {
        let left = json!({"z": 1, "nested": {"b": 2, "a": 1}});
        let right: serde_json::Value =
            serde_json::from_str(r#"{"nested":{"a":1,"b":2},"z":1}"#).unwrap();

        assert_eq!(
            canonical_json_bytes(&left).unwrap(),
            canonical_json_bytes(&right).unwrap()
        );
        assert_eq!(
            canonical_json_sha256(&left).unwrap(),
            canonical_json_sha256(&right).unwrap()
        );
    }
}
