use std::any::{TypeId, type_name};

use fabro_api::types::{
    PetriAdmission as ApiPetriAdmission, PetriGraphRef as ApiPetriGraphRef,
    RunEngine as ApiRunEngine,
};
use fabro_types::{PetriAdmission, PetriGraphRef, RunEngine};
use serde_json::json;

#[test]
fn run_engine_reuses_canonical_types() {
    assert_same_type::<ApiRunEngine, RunEngine>();
    assert_same_type::<ApiPetriAdmission, PetriAdmission>();
    assert_same_type::<ApiPetriGraphRef, PetriGraphRef>();
}

#[test]
fn the_legacy_engine_round_trips_as_its_kind_alone() {
    let value = json!({ "kind": "legacy" });
    let engine: RunEngine = serde_json::from_value(value.clone()).unwrap();
    assert!(engine.is_legacy());
    assert_eq!(serde_json::to_value(&engine).unwrap(), value);
}

#[test]
fn the_petri_engine_round_trips_with_its_admission_flattened() {
    let value = json!({
        "kind": "petri",
        "graph": {
            "blob": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
            "digest": "sha256:root"
        },
        "children": [
            {
                "blob": "3cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
                "digest": "sha256:child"
            }
        ]
    });
    let engine: RunEngine = serde_json::from_value(value.clone()).unwrap();
    let admission = engine
        .petri()
        .expect("a Petri engine carries its admission");
    assert_eq!(admission.graph.digest, "sha256:root");
    assert_eq!(admission.children.len(), 1);
    assert_eq!(serde_json::to_value(&engine).unwrap(), value);
}

fn assert_same_type<T: 'static, U: 'static>() {
    assert_eq!(
        TypeId::of::<T>(),
        TypeId::of::<U>(),
        "{} should be the same type as {}",
        type_name::<T>(),
        type_name::<U>()
    );
}
