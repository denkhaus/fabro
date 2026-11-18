//! Conformance of the board-column vocabulary seam (work#1460; ADR-0045
//! P3 preparation).
//!
//! `fabro_api::board_column` is the single run-status-kind →
//! board-column mapping, and `RunStatusKind::board_rank`'s ranks are the
//! mapped column's position in the spec's `BoardColumn` enum. Every
//! expectation below is derived from parsing the OpenAPI spec in the
//! repository, so a rename, an addition, or a reorder in the spec reds
//! these tests instead of drifting silently.

use fabro_api::types::BoardColumn;
use fabro_api::{board_column, board_column_kinds};
use fabro_types::RunStatusKind;
use serde_json::Value;
use strum::VariantArray;

/// The `BoardColumn` enum tokens, in spec order, read from the OpenAPI
/// spec in the repository.
#[expect(
    clippy::disallowed_methods,
    reason = "a synchronous test reads the spec from the repository once"
)]
fn spec_board_columns() -> Vec<String> {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../docs/public/api-reference/fabro-api.yaml"
    ))
    .expect("the OpenAPI spec is in the repository");
    let yaml: serde_yaml::Value = serde_yaml::from_str(&text).expect("the spec parses");
    let spec: Value = serde_json::to_value(yaml).expect("the spec is JSON-compatible");
    let tokens = spec["components"]["schemas"]["BoardColumn"]["enum"]
        .as_array()
        .expect("BoardColumn is a string enum in the spec");
    tokens
        .iter()
        .map(|token| {
            token
                .as_str()
                .expect("every BoardColumn enum entry is a string")
                .to_string()
        })
        .collect()
}

/// Every status kind maps to a column the spec names. A spec rename of a
/// column token reds here: the mapping still names the old token.
#[test]
fn every_kind_maps_to_a_spec_board_column() {
    let columns = spec_board_columns();
    for kind in RunStatusKind::VARIANTS {
        let column = board_column(*kind);
        assert!(
            columns.iter().any(|name| *name == column.to_string()),
            "{kind} maps to board column `{column}`, which the spec's \
             BoardColumn enum does not name"
        );
    }
}

/// Every spec column except `archived` is the target of at least one
/// status kind. A renamed column (nothing maps to the new token) and a
/// newly added column red here.
#[test]
fn every_spec_column_except_archived_is_mapped() {
    let columns = spec_board_columns();
    for name in &columns {
        if name == "archived" {
            // The overlay column: orthogonal to the status buckets.
            continue;
        }
        let mapped = RunStatusKind::VARIANTS
            .iter()
            .any(|kind| board_column(*kind).to_string() == *name);
        assert!(
            mapped,
            "spec board column `{name}` is not the target of any status kind"
        );
    }
    assert!(
        columns.iter().any(|name| name == "archived"),
        "the spec's BoardColumn enum should keep the archived overlay column"
    );
}

/// A rank IS the mapped column's position in the spec enum order: the
/// single mapping and `board_rank`'s numbers share that one order, so a
/// mis-mapping, a spec reorder, or a rank drift reds here.
#[test]
fn board_rank_is_the_spec_column_position() {
    let columns = spec_board_columns();
    for kind in RunStatusKind::VARIANTS {
        let name = board_column(*kind).to_string();
        let position = columns
            .iter()
            .position(|column| column == &name)
            .unwrap_or_else(|| panic!("`{name}` should be in the spec's BoardColumn enum"));
        assert_eq!(
            kind.board_rank(),
            u8::try_from(position)
                .unwrap_or_else(|_| panic!("column position {position} should fit u8")),
            "{kind}'s board_rank must equal the position of its mapped \
             column `{name}` in the spec's BoardColumn enum"
        );
    }
    // `archived` is an overlay flag, not a status: no kind maps to it,
    // and its position stays unassigned so columns can grow around it.
    for kind in RunStatusKind::VARIANTS {
        assert_ne!(
            board_column(*kind),
            BoardColumn::Archived,
            "{kind} must not map to the archived overlay column"
        );
    }
}

/// The inversion is derived from the mapping and round-trips it;
/// `archived` is the one column with no kinds behind it.
#[test]
fn board_column_kinds_inverts_the_mapping() {
    for kind in RunStatusKind::VARIANTS {
        let column = board_column(*kind);
        let kinds = board_column_kinds(column)
            .unwrap_or_else(|| panic!("only archived lacks kinds, not `{column}`"));
        assert!(kinds.contains(kind), "`{column}` should board {kind}");
    }
    assert_eq!(board_column_kinds(BoardColumn::Archived), None);
}
