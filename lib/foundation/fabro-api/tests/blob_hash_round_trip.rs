use std::any::{TypeId, type_name};

use fabro_api::types::{BlobHash as ApiBlobHash, WriteBlobResponse};
use fabro_types::BlobHash;
use serde_json::json;

const BLOB_HASH: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

#[test]
fn blob_hash_schema_reuses_domain_type() {
    assert_same_type::<ApiBlobHash, BlobHash>();
}

#[test]
fn write_blob_response_round_trips_exact_wire_shape() {
    let value = json!({ "hash": BLOB_HASH });

    let response: WriteBlobResponse = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&response).unwrap(), value);
}

#[test]
fn blob_hash_emits_the_documented_lowercase_pattern() {
    // Serialization must match the OpenAPI schema pattern `^[0-9a-f]{64}$`.
    let hash: ApiBlobHash = serde_json::from_value(json!(BLOB_HASH)).unwrap();
    let emitted = serde_json::to_value(hash).unwrap();
    assert_eq!(emitted, json!(BLOB_HASH));

    let text = emitted.as_str().unwrap();
    assert_eq!(text.len(), 64);
    assert!(
        text.bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    );
}

#[test]
fn blob_hash_rejects_non_hex_values() {
    assert!(serde_json::from_value::<ApiBlobHash>(json!("not-a-blob-hash")).is_err());
}

fn assert_same_type<Api: 'static, Domain: 'static>() {
    assert_eq!(
        TypeId::of::<Api>(),
        TypeId::of::<Domain>(),
        "{} must be the domain type {}",
        type_name::<Api>(),
        type_name::<Domain>()
    );
}
