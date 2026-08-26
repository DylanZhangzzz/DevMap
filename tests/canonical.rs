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
