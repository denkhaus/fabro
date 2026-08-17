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
fn blob_hash_accepts_any_case_and_emits_lowercase() {
    for input in [
        BLOB_HASH.to_string(),
        BLOB_HASH.to_uppercase(),
        alternating_hex_case(BLOB_HASH),
    ] {
        let hash: ApiBlobHash = serde_json::from_value(json!(input)).unwrap();
        assert_eq!(serde_json::to_value(hash).unwrap(), json!(BLOB_HASH));
    }
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

fn alternating_hex_case(value: &str) -> String {
    value
        .chars()
        .enumerate()
        .map(|(index, character)| {
            if index % 2 == 0 {
                character.to_ascii_uppercase()
            } else {
                character
            }
        })
        .collect()
}
