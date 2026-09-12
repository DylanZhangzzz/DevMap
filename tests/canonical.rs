use devmap::canonical::{canonical_json, content_id, sha256_hex};
use serde_json::json;

#[test]
fn canonical_json_sorts_nested_object_keys_and_preserves_arrays() {
    let first = json!({
        "z": "中文",
        "nested": {"b": true, "a": null},
        "items": [3, 1, 2],
        "a": "first"
    });
    let second = json!({
        "items": [3, 1, 2],
        "a": "first",
        "z": "中文",
        "nested": {"a": null, "b": true}
    });

    let first_bytes = canonical_json(&first).expect("canonicalize first object");
    let second_bytes = canonical_json(&second).expect("canonicalize second object");

    assert_eq!(first_bytes, second_bytes);
    assert_eq!(
        String::from_utf8(first_bytes).unwrap(),
        r#"{"a":"first","items":[3,1,2],"nested":{"a":null,"b":true},"z":"中文"}"#
    );
}

#[test]
fn canonical_json_rejects_floating_point_values() {
    let error = canonical_json(&json!({"confidence": 0.9})).unwrap_err();
    assert!(error.to_string().contains("floating point"));
}

#[test]
fn sha256_and_content_ids_are_stable() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        content_id("common-ground", b"abc"),
        "common-ground:sha256-ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn canonical_bytes_match_original_recursive_normalizer() {
    use serde_json::{Map, Value};
    // Frozen pre-optimization algorithm: persisted journal hashes depend on
    // these bytes, including Unicode keys, integer extremes and array order.
    fn original(value: Value) -> Value {
        match value {
            Value::Array(values) => Value::Array(values.into_iter().map(original).collect()),
            Value::Object(values) => {
                let mut entries: Vec<_> = values.into_iter().collect();
                entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
                let mut result = Map::new();
                for (key, value) in entries {
                    result.insert(key, original(value));
                }
                Value::Object(result)
            }
            scalar => scalar,
        }
    }
    let mut cases = vec![
        json!(null),
        json!(true),
        json!(false),
        json!([]),
        json!({}),
        json!(i64::MIN),
        json!(u64::MAX),
        json!("\"\\\n\r\t\u{0}中文😀"),
    ];
    for n in 0..128 {
        cases.push(json!({
            "z": [{"β": [n, null, false], "a": {"😀": u64::MAX, "": i64::MIN}}],
            "a": [3, 1, 2, {"\n": "\\", "中文": n.to_string()}],
            "é": {"z": {}, "a": []},
            "e\u{301}": n,
        }));
    }
    for value in cases {
        let expected = serde_json::to_vec(&original(value.clone())).unwrap();
        assert_eq!(canonical_json(&value).unwrap(), expected);
    }
}

#[test]
fn canonical_float_rejection_reaches_every_nested_position() {
    for value in [
        json!(0.0),
        json!(-0.0),
        json!([{"x": [1, 0.5]}]),
        json!({"a": {"b": 1.0}}),
    ] {
        assert!(matches!(
            canonical_json(&value),
            Err(devmap::error::DevMapError::FloatingPointNotCanonical)
        ));
    }
}
