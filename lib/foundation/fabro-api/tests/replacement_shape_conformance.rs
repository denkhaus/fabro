//! Conformance net over the `with_replacement` registry (work#1685).
//!
//! The sprint-101 class (work#1678): a replacement type's field goes
//! `Option` (null on the wire) while the spec schema still says
//! `type: string`. Spec/router conformance stays green (it never looks
//! at field shapes), round-trip tests stay green (they exercise the
//! replacement type's own JSON), and only the two-axis human review
//! caught it. This net walks EVERY `with_replacement(...)` mapping —
//! read straight out of `build.rs`, so the table cannot be mirrored or
//! shadowed — and asserts each spec schema's field shapes are
//! wire-compatible with the replacement type's serde shape:
//!
//! - **nullability**: `Option<T>` fields must be nullable in the schema (`type:
//!   [T, "null"]`, `oneOf: [<T>, {type: null}]`, or `nullable: true`);
//!   non-`Option` fields must not be.
//! - **required membership**: serde-mandatory fields (no `Option`, no
//!   `#[serde(default)]`, not `skip_deserializing`) must be listed in the
//!   schema's `required`.
//! - **field name set**: every serde field (honoring `rename`, `rename_all`,
//!   `flatten`, `skip`) must appear in the schema's `properties`, and every
//!   schema property must exist on the type.
//! - **enum variant sets**: unit enums vs `enum:` schemas (both directions);
//!   internally/adjacently tagged enums vs `discriminator` mappings (tag
//!   property name + variant keys).
//! - **primitive kinds**: `String`/`DateTime`/ULID vs `type: string`, integers
//!   vs `type: integer`, `Vec<T>` vs `type: array`, maps vs `type: object` —
//!   through newtype transparency, `ulid_id!`/`string_id!` macro expansion, and
//!   the MANUAL_WIRE_KINDS table for hand-written `impl Serialize` types.
//!
//! The Rust-side serde shape is extracted from the mapped crates'
//! sources (`syn`), located through `cargo metadata` — including the
//! vendored forks and the lithos-llm git checkout, whose pinned
//! revision is what the workspace actually compiles.
//!
//! The net's known boundaries, deliberately: per-variant object schemas
//! of tagged enums (e.g. `RunStatusPending`) are not themselves
//! replacement mappings and are not walked; untagged enums have no
//! stable field surface and are skipped; anonymous inline schemas that
//! typify names are checked through their container schema's fields.

#![allow(
    clippy::absolute_paths,
    clippy::default_trait_access,
    clippy::manual_let_else,
    clippy::too_many_lines,
    reason = "This net mirrors the direct assertion style of the OpenAPI conformance tests."
)]
#![expect(
    clippy::disallowed_methods,
    reason = "The net reads the spec and the mapped crates' sources and runs `cargo metadata` \
              synchronously on purpose: it is a plain test with no runtime to block."
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use quote::ToTokens;
use serde_yaml::Value;
use syn::{Expr, Item, Pat, Stmt, UseTree};

/// Mapped schema names that have no `components.schemas` entry and no
/// inline schema anywhere in the spec — typify never produces these
/// names, so the `with_replacement` entries are vestigial. The net
/// reports every OTHER unresolvable name as drift; entries land here
/// only after investigation, with the reason recorded.
const VESTIGIAL_MAPPINGS: &[(&str, &str)] = &[];

/// Mappings that legitimately yield zero comparable assertions because
/// both sides are shapeless from the net's vantage point. Anything NEW
/// landing here must come with its own reason.
const EXPECTED_NO_COMPARISON: &[(&str, &str)] = &[
    // `NetworkPolicy` is an externally tagged enum (unit variants ride a
    // bare string, data variants ride their renamed variant key) and the
    // spec models exactly that as a mixed `oneOf` — no single axis of
    // the net can compare it today.
    ("SandboxNetworkPolicy", "sandbox_driver::NetworkPolicy"),
    // The spec models the two optional-presence combos of
    // `ReasoningOutput` as `oneOf` arms; the Rust type is one struct
    // with optional members. Structurally compatible, not comparable
    // field-by-field.
    ("ReasoningOutput", "lithos_llm::types::ReasoningOutput"),
    // The spec's `type` discriminator property carries no value list
    // (the variants are enumerated in prose only), so there is nothing
    // to compare the Rust variant names against.
    ("CompletionContentPart", "lithos_llm::types::ContentPart"),
];

/// Spec properties the Rust replacement type genuinely does not carry,
/// each investigated and accepted for now. These are REAL wire-contract
/// gaps — the fix needs a maintainer decision (add the field to the
/// type, or re-shape the schema) and should be filed as work items; the
/// entries keep the net green without hiding the debt.
const KNOWN_DRIFT: &[(&str, &str, &str)] = &[
    (
        "GitRunTarget",
        "kind",
        "the API's git discriminator: clients send `kind: git` and the Rust type ignores the          unknown field, but the replacement never emits it, so responses carrying the type          violate the schema's `required`",
    ),
    (
        "PrincipalUser",
        "kind",
        "the principal discriminator rides the `Principal` enum's tag on the wire;          `UserPrincipal` is the inner payload and carries no `kind` field",
    ),
    (
        "WorkflowSettings",
        "environments",
        "the spec models a resolved per-environment settings map the Rust snapshot type does          not carry — wire payloads never include it",
    ),
];

/// Replacement types whose serde is HAND-WRITTEN, so their declared
/// fields are not their wire surface. The table records what the wire
/// actually sees; a mapped type with a manual `impl Serialize` that is
/// missing here makes the net fail loudly instead of guessing.
/// Computed members a manual object impl emits beyond the declared
/// fields, with their wire kind.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ManualFieldKind {
    Str,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ManualWire {
    /// Serializes to a plain string (`serializer.collect_str`).
    Str,
    /// Object-shaped: the declared fields ride the wire (through a
    /// wire-projection struct or `serialize_struct`), with documented
    /// deviations from the derived surface.
    Object {
        /// `Option` members are omitted (never null) when absent.
        omit_absent_options: bool,
        /// Members the impl emits beyond the declared fields.
        extra_fields:        &'static [(&'static str, ManualFieldKind)],
    },
}

const MANUAL_WIRE_KINDS: &[(&str, ManualWire, &str)] = &[
    (
        "fabro_types::stage_id::StageId",
        ManualWire::Str,
        "manual impl serializes `node_id#visit` via collect_str",
    ),
    (
        "fabro_types::stage_id::ParallelBranchId",
        ManualWire::Str,
        "manual impl serializes `group#index` via collect_str",
    ),
    (
        "fabro_types::blob_hash::BlobHash",
        ManualWire::Str,
        "manual impl serializes the lowercase hex SHA-256",
    ),
    (
        "fabro_types::outcome::StageOutcome",
        ManualWire::Str,
        "manual impl serializes the snake_case outcome name",
    ),
    (
        "lithos_llm::types::ErrorKind",
        ManualWire::Str,
        "manual impl serializes the stable category spelling (unknown kinds pass through)",
    ),
    (
        "fabro_types::pull_request::PullRequestLink",
        ManualWire::Object {
            omit_absent_options: true,
            extra_fields:        &[("html_url", ManualFieldKind::Str)],
        },
        "manual impl computes `html_url` and emits `state` only when present          (denkhaus/work#1173), never as null",
    ),
    (
        "fabro_types::run_sandbox::RunSandbox",
        ManualWire::Object {
            omit_absent_options: false,
            extra_fields:        &[],
        },
        "manual impl serializes the same field set through `RunSandboxWireRef`",
    ),
];

fn manual_wire_kind(crate_name: &str, name: &str) -> Option<ManualWire> {
    MANUAL_WIRE_KINDS
        .iter()
        .filter(|(path, _, _)| {
            let segments: Vec<&str> = path.split("::").collect();
            segments.first() == Some(&crate_name) && segments.last() == Some(&name)
        })
        .map(|(_, kind, _)| *kind)
        .next()
        .or_else(|| {
            // Exact-path spelling is the canonical form; a root-path
            // resolution that matches a table entry's tail also matches.
            MANUAL_WIRE_KINDS
                .iter()
                .find(|(path, _, _)| path.rsplit("::").next() == Some(name))
                .map(|(_, kind, _)| *kind)
        })
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[test]
fn every_replacement_mapping_matches_its_rust_serde_shape() {
    let mappings = replacement_table();
    assert!(
        !mappings.is_empty(),
        "the with_replacement table extraction found nothing — \
         update `replacement_table` to the new build.rs shape"
    );

    let mut findings: Vec<String> = Vec::new();
    let mut comparable = 0usize;

    for (schema_name, rust_path) in &mappings {
        let before = findings.len();
        let comparisons = check_mapping(schema_name, rust_path, &mut findings);
        comparable += comparisons;
        if comparisons == 0 && findings.len() == before {
            if EXPECTED_NO_COMPARISON.contains(&(schema_name.as_str(), rust_path.as_str())) {
                continue;
            }
            findings.push(format!(
                "schema `{schema_name}` → {rust_path}: the net compared nothing for this mapping — \
                 either the schema shape or the Rust type is outside the net's rules; extend the \
                 net or record the mapping in EXPECTED_NO_COMPARISON with a reason (work#1685)"
            ));
        }
    }

    assert!(
        findings.is_empty(),
        "spec ↔ replacement-type shape drift (work#1685), {} finding(s):\n\n{}\n",
        findings.len(),
        findings.join("\n\n")
    );
    assert!(
        comparable > 0,
        "the net compared nothing — every mapping resolved to zero assertions"
    );
}

// ---------------------------------------------------------------------------
// build.rs table extraction — the registry's single source of truth
// ---------------------------------------------------------------------------

fn replacement_table() -> Vec<(String, String)> {
    static TABLE: OnceLock<Vec<(String, String)>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            let build_rs = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build.rs");
            let source = std::fs::read_to_string(&build_rs)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", build_rs.display()));
            let file: syn::File = syn::parse_file(&source)
                .unwrap_or_else(|err| panic!("failed to parse build.rs: {err}"));

            let mut table = Vec::new();
            for item in &file.items {
                let Item::Fn(func) = item else { continue };
                if func.sig.ident != "main" {
                    continue;
                }
                for stmt in &func.block.stmts {
                    let Stmt::Local(local) = stmt else { continue };
                    // Typed bindings (`let replacements: &[…] = …`) wrap
                    // the identifier in `Pat::Type`.
                    let binding = match &local.pat {
                        Pat::Ident(pat_ident) => Some(pat_ident.ident.to_string()),
                        Pat::Type(pat_type) => match &*pat_type.pat {
                            Pat::Ident(pat_ident) => Some(pat_ident.ident.to_string()),
                            _ => None,
                        },
                        _ => None,
                    };
                    let Some(binding) = binding else { continue };
                    if binding != "replacements" {
                        continue;
                    }
                    let Some(init) = &local.init else { continue };
                    extract_table_expr(&init.expr, &mut table);
                }
            }
            assert!(
                !table.is_empty(),
                "could not find the `replacements` table in build.rs — update the \
                 extraction in this test to the table's new shape (work#1685)"
            );
            table
        })
        .clone()
}

fn extract_table_expr(expr: &Expr, table: &mut Vec<(String, String)>) {
    // `= &[…]` — descend through the reference (and grouping) wrappers.
    let mut current = expr;
    loop {
        match current {
            Expr::Reference(reference) => current = &reference.expr,
            Expr::Group(group) => current = &group.expr,
            Expr::Paren(paren) => current = &paren.expr,
            _ => break,
        }
    }
    let Expr::Array(array) = current else {
        panic!(
            "build.rs `replacements` is no longer an array literal — \
             update the extraction in this test (work#1685)"
        );
    };
    for element in &array.elems {
        let Expr::Tuple(tuple) = element else {
            panic!(
                "build.rs `replacements` entry is no longer a tuple literal — \
                 update the extraction in this test (work#1685)"
            );
        };
        let elements: Vec<&Expr> = tuple.elems.iter().collect();
        let [schema, rust_path, _impls] = elements.as_slice() else {
            panic!(
                "build.rs `replacements` entry no longer has three elements — \
                 update the extraction in this test (work#1685)"
            );
        };
        let schema_name = literal_str(schema)
            .unwrap_or_else(|| panic!("build.rs replacement schema name is not a string literal"));
        let type_path = literal_str(rust_path)
            .unwrap_or_else(|| panic!("build.rs replacement rust type is not a string literal"));
        table.push((schema_name, type_path));
    }
}

fn literal_str(expr: &Expr) -> Option<String> {
    let Expr::Lit(lit) = expr else { return None };
    let syn::Lit::Str(text) = &lit.lit else {
        return None;
    };
    Some(text.value())
}

// ---------------------------------------------------------------------------
// Spec access
// ---------------------------------------------------------------------------

fn spec() -> &'static Value {
    static SPEC: OnceLock<Value> = OnceLock::new();
    SPEC.get_or_init(|| {
        let path = repo_root().join("docs/public/api-reference/fabro-api.yaml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        serde_yaml::from_str(&text).unwrap_or_else(|err| panic!("failed to parse the spec: {err}"))
    })
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("fabro-api manifest dir should be nested under <repo>/lib/foundation/fabro-api")
        .to_path_buf()
}

fn spec_get<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_mapping()?.get(Value::String(key.into()))
}

fn components_schemas() -> &'static serde_yaml::Mapping {
    static ONCE: OnceLock<()> = OnceLock::new();
    let _ = ONCE.set(());
    let components = spec_get(spec(), "components").expect("spec is missing `components`");
    let schemas = spec_get(components, "schemas").expect("spec is missing `components.schemas`");
    schemas
        .as_mapping()
        .expect("components.schemas should be a mapping")
}

fn schema_by_name(name: &str) -> Option<&'static Value> {
    components_schemas().get(Value::String(name.into()))
}

/// A property schema reduced to what the comparison engine needs.
#[derive(Debug, Clone, PartialEq)]
enum SpecShape {
    /// `type: string | integer | number | boolean`
    Prim(String),
    /// `type: array` with directly comparable items (if any)
    Array(Option<Box<SpecShape>>),
    /// `type: object`
    Object,
    /// `$ref: '#/components/schemas/<name>'` (unresolved — the engine
    /// follows it)
    Ref(String),
    /// Direct `enum: [...]` values
    Enum(Vec<String>),
    /// `oneOf`/`allOf` with several non-null arms, or unconstrained —
    /// not directly comparable
    Complex,
}

/// Render a compact spec-side shape for failure messages.
fn render_schema_shape(value: &Value) -> String {
    if let Some(name) = value.get("$ref").and_then(Value::as_str) {
        return format!("$ref: {name}");
    }
    if let Some(variants) = value.get("oneOf").and_then(Value::as_sequence) {
        return format!("oneOf: [{} variants]", variants.len());
    }
    if let Some(variants) = value.get("allOf").and_then(Value::as_sequence) {
        return format!("allOf: [{} variants]", variants.len());
    }
    match value.get("type") {
        Some(Value::Sequence(types)) => {
            let rendered: Vec<String> = types
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            format!("type: [{}]", rendered.join(", "))
        }
        Some(Value::String(single)) => {
            if let Some(values) = value.get("enum").and_then(Value::as_sequence) {
                let rendered: Vec<String> = values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                format!("type: {single}, enum: [{}]", rendered.join(", "))
            } else {
                format!("type: {single}")
            }
        }
        _ => "untyped schema".to_string(),
    }
}

fn is_pure_null(value: &Value) -> bool {
    value
        .get("type")
        .is_some_and(|t| t == &Value::String("null".into()))
}

/// True when the schema admits null in any of the spec's styles.
fn schema_is_nullable(value: &Value, depth: u8) -> bool {
    if depth > 6 {
        return false;
    }
    if value
        .get("nullable")
        .is_some_and(|v| v == &Value::Bool(true))
    {
        return true;
    }
    if let Some(types) = value.get("type").and_then(Value::as_sequence) {
        if types.iter().any(|t| t == &Value::String("null".into())) {
            return true;
        }
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(variants) = value.get(key).and_then(Value::as_sequence) {
            if variants.iter().any(is_pure_null) {
                return true;
            }
        }
    }
    if value.get("$ref").is_some() {
        return match ref_target(value) {
            Some(target) => schema_is_nullable(target, depth + 1),
            None => false,
        };
    }
    false
}

/// The `$ref` target schema, if the value is a bare reference.
fn ref_target(value: &Value) -> Option<&'static Value> {
    let reference = value.get("$ref")?.as_str()?;
    let name = reference.rsplit('/').next()?;
    schema_by_name(name)
}

/// Reduce a (nullability-stripped) schema to a comparable shape.
fn spec_shape(value: &Value, depth: u8) -> SpecShape {
    if depth > 6 {
        return SpecShape::Complex;
    }
    if value.get("$ref").is_some() {
        return match ref_target(value) {
            Some(target) => spec_shape(target, depth + 1),
            None => {
                let name = value
                    .get("$ref")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string();
                SpecShape::Ref(name)
            }
        };
    }
    if let Some(type_value) = value.get("type") {
        if let Some(types) = type_value.as_sequence() {
            let non_null: Vec<&str> = types
                .iter()
                .filter_map(Value::as_str)
                .filter(|t| *t != "null")
                .collect();
            return shape_of_type_list(value, &non_null, depth);
        }
        if let Some(single) = type_value.as_str() {
            if single != "null" {
                return shape_of_type_list(value, &[single], depth);
            }
        }
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(variants) = value.get(key).and_then(Value::as_sequence) {
            let non_null: Vec<&Value> = variants.iter().filter(|v| !is_pure_null(v)).collect();
            if non_null.len() == 1 {
                return spec_shape(non_null[0], depth + 1);
            }
            return SpecShape::Complex;
        }
    }
    SpecShape::Complex
}

fn shape_of_type_list(value: &Value, types: &[&str], depth: u8) -> SpecShape {
    match types {
        ["string"] => match value.get("enum").and_then(Value::as_sequence) {
            Some(values) => SpecShape::Enum(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            ),
            None => SpecShape::Prim("string".into()),
        },
        ["integer"] => SpecShape::Prim("integer".into()),
        ["number"] => SpecShape::Prim("number".into()),
        ["boolean"] => SpecShape::Prim("boolean".into()),
        ["array"] => {
            let items = value
                .get("items")
                .map(|items| Box::new(spec_shape(items, depth + 1)));
            SpecShape::Array(items)
        }
        ["object"] => SpecShape::Object,
        _ => SpecShape::Complex,
    }
}

// ---------------------------------------------------------------------------
// Rust type expressions — the net's own tree, stripped of `syn` so it
// can live in a `static` index (syn types carry spans and are neither
// `Debug` nor `Send`/`Sync` by default)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct TypeSeg {
    ident: String,
    /// Generic type arguments of this segment (lifetimes and consts
    /// dropped — irrelevant to wire shape).
    args:  Vec<TypeExpr>,
}

#[derive(Clone, Debug, PartialEq)]
enum TypeExpr {
    Path(Vec<TypeSeg>),
    Reference(Box<TypeExpr>),
    Slice(Box<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Other(String),
}

impl TypeExpr {
    fn render(&self) -> String {
        match self {
            TypeExpr::Path(segments) => {
                let rendered: Vec<String> = segments.iter().map(render_seg).collect();
                rendered.join("::")
            }
            TypeExpr::Reference(inner) => format!("&{}", inner.render()),
            TypeExpr::Slice(inner) => format!("[{}]", inner.render()),
            TypeExpr::Tuple(elements) => {
                let rendered: Vec<String> = elements.iter().map(TypeExpr::render).collect();
                format!("({})", rendered.join(", "))
            }
            TypeExpr::Other(text) => text.clone(),
        }
    }

    fn path_segments(&self) -> Option<&[TypeSeg]> {
        match self {
            TypeExpr::Path(segments) => Some(segments),
            _ => None,
        }
    }

    /// `Option<T>` → `Some(T)` for the outermost wrapper.
    fn option_inner(&self) -> Option<&TypeExpr> {
        let segments = self.path_segments()?;
        let last = segments.last()?;
        if last.ident != "Option" {
            return None;
        }
        last.args.first()
    }

    /// Type arguments of the final path segment.
    fn final_args(&self) -> &[TypeExpr] {
        match self.path_segments().map(|segments| segments.last()) {
            Some(Some(last)) => &last.args,
            _ => &[],
        }
    }
}

fn render_seg(segment: &TypeSeg) -> String {
    if segment.args.is_empty() {
        return segment.ident.clone();
    }
    let rendered: Vec<String> = segment.args.iter().map(TypeExpr::render).collect();
    format!("{}<{}>", segment.ident, rendered.join(", "))
}

fn type_expr(ty: &syn::Type) -> TypeExpr {
    match ty {
        syn::Type::Path(path) => TypeExpr::Path(
            path.path
                .segments
                .iter()
                .map(|segment| TypeSeg {
                    ident: segment.ident.to_string(),
                    args:  path_segment_args(&segment.arguments),
                })
                .collect(),
        ),
        syn::Type::Reference(reference) => {
            TypeExpr::Reference(Box::new(type_expr(&reference.elem)))
        }
        syn::Type::Slice(slice) => TypeExpr::Slice(Box::new(type_expr(&slice.elem))),
        syn::Type::Tuple(tuple) => TypeExpr::Tuple(tuple.elems.iter().map(type_expr).collect()),
        syn::Type::Group(group) => type_expr(&group.elem),
        syn::Type::Paren(paren) => type_expr(&paren.elem),
        other => TypeExpr::Other(other.to_token_stream().to_string()),
    }
}

fn path_segment_args(arguments: &syn::PathArguments) -> Vec<TypeExpr> {
    let syn::PathArguments::AngleBracketed(args) = arguments else {
        return Vec::new();
    };
    args.args
        .iter()
        .filter_map(|arg| match arg {
            syn::GenericArgument::Type(inner) => Some(type_expr(inner)),
            _ => None,
        })
        .collect()
}

fn parse_type_expr(text: &str) -> TypeExpr {
    let parsed = syn::parse_str::<syn::Type>(text)
        .unwrap_or_else(|err| panic!("hard-coded type `{text}` should parse: {err}"));
    type_expr(&parsed)
}

// ---------------------------------------------------------------------------
// Crate source index
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct ResolvedType {
    crate_name:   String,
    module:       String,
    name:         String,
    file:         PathBuf,
    def:          TypeDef,
    /// The type has a hand-written `impl Serialize` — its field surface
    /// is not its wire surface; [`MANUAL_WIRE_KINDS`] must say what is.
    manual_serde: bool,
}

impl ResolvedType {
    fn full_path(&self) -> String {
        if self.module.is_empty() {
            format!("{}::{}", self.crate_name, self.name)
        } else {
            format!("{}::{}::{}", self.crate_name, self.module, self.name)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum TypeDef {
    /// Newtype over a single unnamed field (transparent on the wire).
    Newtype(TypeExpr, String),
    Struct(StructDef),
    Enum(EnumDef),
}

#[derive(Clone, Debug, PartialEq)]
struct StructDef {
    fields: Vec<SourceField>,
}

#[derive(Clone, Debug, PartialEq)]
struct SourceField {
    ident:       String,
    ty:          TypeExpr,
    optional:    bool,
    default:     bool,
    skip_ser:    bool,
    /// `skip_serializing_if = "Option::is_none"` — null is never emitted.
    skip_ser_if: bool,
    skip_deser:  bool,
    flatten:     bool,
    wire_name:   String,
}

/// Where a (possibly flatten-merged) field was declared — the context
/// reference resolution needs for its types.
#[derive(Clone, Debug, PartialEq)]
struct FieldContext {
    origin:     String,
    crate_name: String,
    module:     String,
    file:       PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
struct EnumDef {
    /// `tag = "..."` — internally or adjacently tagged.
    tag:       Option<String>,
    /// `content = "..."` — the adjacent content key.
    content:   Option<String>,
    untagged:  bool,
    /// Every variant is a unit variant (serializes to bare names).
    unit_only: bool,
    /// Serialized variant names in declaration order.
    variants:  Vec<String>,
    /// A `#[serde(other)]` catch-all exists (deserialization accepts
    /// any string).
    catch_all: bool,
}

#[derive(Default)]
struct CrateIndex {
    /// "(module_path, TypeName)" → definition
    by_module:       BTreeMap<(String, String), TypeDef>,
    /// "(module_path, TypeName)" → defining file
    module_files:    BTreeMap<(String, String), PathBuf>,
    /// bare type name → module paths declaring it
    by_name:         BTreeMap<String, Vec<String>>,
    /// file path → import map (visible last segment → full path segments)
    imports:         BTreeMap<PathBuf, BTreeMap<String, Vec<String>>>,
    /// the crate root's import map — how re-exported public paths
    /// (`fabro_types::ContextWindowSnapshot` living in pebble) resolve
    root_imports:    BTreeMap<String, Vec<String>>,
    /// module path → defining file (for consulting a module's re-exports)
    files_by_module: BTreeMap<String, PathBuf>,
    /// types with a hand-written `impl Serialize` in their file
    manual_serde:    BTreeMap<(String, String), ()>,
}

struct SourceIndexes {
    crates: BTreeMap<String, CrateIndex>,
}

impl SourceIndexes {
    fn resolve(&self, rust_path: &str) -> Option<ResolvedType> {
        let segments: Vec<&str> = rust_path.split("::").collect();
        let [crate_name, rest @ ..] = segments.as_slice() else {
            return None;
        };
        let type_name = (*rest.last()?).to_string();
        let module = rest[..rest.len() - 1].join("::");
        if let Some(resolved) = self.resolve_qualified(crate_name, &module, &type_name) {
            return Some(resolved);
        }
        // Re-exported through the crate root (`pub use other::Type` or
        // `pub use self::module::Type`).
        let imported = self
            .crates
            .get(*crate_name)?
            .root_imports
            .get(&type_name)?
            .clone();
        let imported_type = imported.last()?.clone();
        if self.crates.contains_key(&imported[0]) {
            let imported_module = imported[1..imported.len() - 1].join("::");
            return self.resolve_qualified(&imported[0], &imported_module, &imported_type);
        }
        // Internal module path (`pub use usage::ModelRef`).
        let imported_module = imported[..imported.len() - 1].join("::");
        self.resolve_qualified(crate_name, &imported_module, &imported_type)
    }

    fn resolve_qualified(
        &self,
        crate_name: &str,
        module: &str,
        type_name: &str,
    ) -> Option<ResolvedType> {
        let index = self.crates.get(crate_name)?;
        let key = (module.to_string(), type_name.to_string());
        if let Some(def) = index.by_module.get(&key) {
            return Some(ResolvedType {
                crate_name:   crate_name.to_string(),
                module:       module.to_string(),
                name:         type_name.to_string(),
                file:         index.module_files.get(&key)?.clone(),
                def:          def.clone(),
                manual_serde: index.manual_serde.contains_key(&key),
            });
        }
        // Re-exported public paths (e.g. `fabro_types::SessionSummary`
        // is declared inside `session`): fall back to a unique bare-name
        // match within the crate.
        if let Some(modules) = index.by_name.get(type_name) {
            if modules.len() == 1 {
                let module = modules[0].clone();
                let key = (module.clone(), type_name.to_string());
                if let Some(def) = index.by_module.get(&key) {
                    return Some(ResolvedType {
                        crate_name: crate_name.to_string(),
                        module,
                        name: type_name.to_string(),
                        file: index.module_files.get(&key)?.clone(),
                        def: def.clone(),
                        manual_serde: index.manual_serde.contains_key(&key),
                    });
                }
            }
        }
        // The module itself may re-export the type
        // (`command_output::CommandTermination` living in pebble).
        let module_file = index.files_by_module.get(module)?;
        let imported = index.imports.get(module_file)?.get(type_name)?.clone();
        if imported.len() < 2 {
            return None;
        }
        if self.crates.contains_key(&imported[0]) {
            let type_name = imported.last()?.clone();
            let module = imported[1..imported.len() - 1].join("::");
            return self.resolve_qualified(&imported[0], &module, &type_name);
        }
        None
    }

    /// Resolve a type reference appearing inside `in_crate`, using the
    /// defining file's imports when the name is unqualified.
    fn resolve_reference(
        &self,
        segments: &[TypeSeg],
        in_crate: &str,
        in_module: &str,
        file: &Path,
    ) -> Option<ResolvedType> {
        let head = segments[0].ident.as_str();
        match head {
            "crate" => {
                let module = segments[1..segments.len() - 1]
                    .iter()
                    .map(|segment| segment.ident.clone())
                    .collect::<Vec<_>>()
                    .join("::");
                let type_name = &segments.last()?.ident;
                self.resolve_qualified(in_crate, &module, type_name)
            }
            "super" | "self" => {
                let base = match head {
                    "super" => in_module.rsplit_once("::").map_or("", |(prefix, _)| prefix),
                    _ => in_module,
                };
                let mut full: Vec<String> = base
                    .split("::")
                    .filter(|part| !part.is_empty())
                    .map(str::to_string)
                    .collect();
                full.extend(segments[1..].iter().map(|segment| segment.ident.clone()));
                let type_name = full.pop()?;
                let module = full.join("::");
                self.resolve_qualified(in_crate, &module, &type_name)
            }
            _ if self.crates.contains_key(head) => {
                let module = segments[1..segments.len() - 1]
                    .iter()
                    .map(|segment| segment.ident.clone())
                    .collect::<Vec<_>>()
                    .join("::");
                let type_name = &segments.last()?.ident;
                self.resolve_qualified(head, &module, type_name)
            }
            // Unqualified: the defining file's imports first, then a
            // unique bare-name match in the defining crate, then across
            // all indexed crates (same-file types have no import).
            _ => {
                let index = self.crates.get(in_crate)?;
                if let Some(imported) = index.imports.get(file).and_then(|m| m.get(head)) {
                    let imported = imported.clone();
                    let crate_name = match imported[0].as_str() {
                        "crate" => in_crate.to_string(),
                        "super" | "self" => return None, // rare inside imports; skip
                        crate_name => crate_name.to_string(),
                    };
                    let type_name = imported.last()?.clone();
                    let module = imported[1..imported.len() - 1].join("::");
                    return self.resolve_qualified(&crate_name, &module, &type_name);
                }
                if let Some(modules) = index.by_name.get(head) {
                    if modules.len() == 1 {
                        return self.resolve_qualified(in_crate, &modules[0], head);
                    }
                }
                let mut global: Vec<(&str, &String)> = Vec::new();
                for (crate_name, index) in &self.crates {
                    if let Some(modules) = index.by_name.get(head) {
                        for module in modules {
                            global.push((crate_name.as_str(), module));
                        }
                    }
                }
                if global.len() == 1 {
                    let (crate_name, module) = global[0];
                    return self.resolve_qualified(crate_name, module, head);
                }
                None
            }
        }
    }
}

fn indexes() -> &'static SourceIndexes {
    static INDEXES: OnceLock<SourceIndexes> = OnceLock::new();
    INDEXES.get_or_init(build_indexes)
}

fn build_indexes() -> SourceIndexes {
    let manifest_paths = dependency_manifest_paths();
    let needed: BTreeSet<String> = replacement_table()
        .iter()
        .filter_map(|(_, path)| path.split("::").next().map(str::to_string))
        .collect();
    let mut crates = BTreeMap::new();
    for crate_name in needed {
        let Some(manifest) = manifest_paths.get(&crate_name) else {
            panic!(
                "cargo metadata does not know crate `{crate_name}` referenced by the \
                 with_replacement table — is the table's crate path spelled correctly?"
            );
        };
        let src_root = manifest
            .parent()
            .expect("manifest has a parent")
            .join("src");
        let index = parse_crate(&crate_name, &src_root);
        crates.insert(crate_name, index);
    }
    SourceIndexes { crates }
}

/// lib name → manifest path, resolved through cargo metadata so the net
/// also reaches the vendored forks and the lithos-llm git checkout at
/// the exact revisions the workspace compiles.
fn dependency_manifest_paths() -> BTreeMap<String, PathBuf> {
    static PATHS: OnceLock<BTreeMap<String, PathBuf>> = OnceLock::new();
    PATHS
        .get_or_init(|| {
            let manifest = repo_root().join("Cargo.toml");
            let output = std::process::Command::new("cargo")
                .args(["metadata", "--format-version", "1", "--offline"])
                .arg("--manifest-path")
                .arg(&manifest)
                .output()
                .expect("failed to run `cargo metadata` — is the workspace buildable?");
            assert!(
                output.status.success(),
                "`cargo metadata --offline` failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let parsed: Value = serde_yaml::from_slice(&output.stdout)
                .unwrap_or_else(|err| panic!("cargo metadata produced invalid JSON: {err}"));
            let packages = spec_get(&parsed, "packages").expect("metadata is missing packages");
            let mut paths = BTreeMap::new();
            let Value::Sequence(list) = packages else {
                panic!("cargo metadata `packages` should be a list");
            };
            for package in list {
                let Some(_name) = package.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let Some(targets) = package.get("targets").and_then(Value::as_sequence) else {
                    continue;
                };
                for target in targets {
                    let Some(kinds) = target.get("kind").and_then(Value::as_sequence) else {
                        continue;
                    };
                    let is_lib = kinds.iter().any(|k| k == &Value::String("lib".into()));
                    let Some(lib_name) = target.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    if is_lib {
                        if let Some(manifest_path) =
                            package.get("manifest_path").and_then(Value::as_str)
                        {
                            paths.insert(lib_name.to_string(), PathBuf::from(manifest_path));
                        }
                    }
                }
            }
            paths
        })
        .clone()
}

fn parse_crate(crate_name: &str, src_root: &Path) -> CrateIndex {
    let mut index = CrateIndex::default();
    let entry = if src_root.join("lib.rs").exists() {
        src_root.join("lib.rs")
    } else {
        src_root.join("main.rs")
    };
    let mut queue: Vec<(String, PathBuf)> = vec![(String::new(), entry)];
    let mut visited = BTreeSet::new();
    while let Some((module_path, file_path)) = queue.pop() {
        if !visited.insert(file_path.clone()) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&file_path) else {
            continue;
        };
        let Ok(file) = syn::parse_file(&source) else {
            panic!(
                "failed to parse {} of crate {crate_name}",
                file_path.display()
            );
        };
        let file_imports = collect_imports(&file);
        if module_path.is_empty() {
            index.root_imports = file_imports.clone();
        }
        index
            .files_by_module
            .insert(module_path.to_string(), file_path.clone());
        index.imports.insert(file_path.clone(), file_imports);
        let mut manual = BTreeSet::new();
        collect_manual_serialize(&file.items, &mut manual);
        for name in manual {
            index
                .manual_serde
                .insert((module_path.to_string(), name), ());
        }
        collect_items(
            &file.items,
            &module_path,
            &file_path,
            &mut index,
            &mut queue,
        );
    }
    index
}

/// Names with a hand-written `impl … Serialize for <Name>` in this file.
fn collect_manual_serialize(items: &[Item], out: &mut BTreeSet<String>) {
    for item in items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(trait_name) = item_impl.trait_.as_ref().and_then(|(_, path, _)| {
            path.segments
                .last()
                .map(|segment| segment.ident.to_string())
        }) else {
            continue;
        };
        if trait_name != "Serialize" {
            continue;
        }
        let syn::Type::Path(self_path) = &*item_impl.self_ty else {
            continue;
        };
        if let Some(last) = self_path.path.segments.last() {
            out.insert(last.ident.to_string());
        }
    }
}

fn collect_items(
    items: &[Item],
    module_path: &str,
    file_path: &Path,
    index: &mut CrateIndex,
    queue: &mut Vec<(String, PathBuf)>,
) {
    for item in items {
        match item {
            Item::Struct(item_struct) => {
                let name = item_struct.ident.to_string();
                let def = type_def_for_struct(item_struct);
                index_type(index, module_path, &name, file_path, def);
            }
            Item::Enum(item_enum) => {
                let name = item_enum.ident.to_string();
                let def = enum_def(&item_enum.variants, &item_enum.attrs);
                index_type(index, module_path, &name, file_path, TypeDef::Enum(def));
            }
            Item::Macro(item_macro) => {
                // Macro-defined ID newtypes that serialize as strings:
                // `ulid_id!(SessionId)` (fabro-types/src/id.rs) and
                // `string_id!(ProviderId)` (lithos catalog).
                let macro_name = item_macro
                    .mac
                    .path
                    .segments
                    .last()
                    .map(|segment| segment.ident.to_string());
                if !matches!(macro_name.as_deref(), Some("ulid_id") | Some("string_id")) {
                    continue;
                }
                let Ok(ident) = syn::parse2::<syn::Ident>(item_macro.mac.tokens.clone()) else {
                    continue;
                };
                index_type(
                    index,
                    module_path,
                    &ident.to_string(),
                    file_path,
                    TypeDef::Newtype(
                        parse_type_expr("String"),
                        "macro-defined ID newtype (serializes as a string)".to_string(),
                    ),
                );
            }
            Item::Mod(item_mod) => {
                let child = join_module(module_path, &item_mod.ident.to_string());
                match &item_mod.content {
                    Some((_, content)) => {
                        collect_items(content, &child, file_path, index, queue);
                    }
                    None => {
                        // Children of `lib.rs`/`mod.rs` live beside the
                        // file; children of `foo.rs` live in `foo/`.
                        let parent = file_path.parent().unwrap_or(Path::new("."));
                        let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                        let base_dir = if stem == "lib" || stem == "main" || stem == "mod" {
                            parent.to_path_buf()
                        } else {
                            parent.join(stem)
                        };
                        let candidates = [
                            base_dir.join(format!("{}.rs", item_mod.ident)),
                            base_dir.join(item_mod.ident.to_string()).join("mod.rs"),
                        ];
                        for candidate in candidates {
                            if candidate.exists() {
                                queue.push((child.clone(), candidate));
                                break;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn type_def_for_struct(item_struct: &syn::ItemStruct) -> TypeDef {
    let container = serde_attrs(&item_struct.attrs);
    let syn::Fields::Named(named) = &item_struct.fields else {
        // Tuple structs are newtypes on the wire; unit structs have no
        // field surface at all (represented as an empty struct).
        if let syn::Fields::Unnamed(unnamed) = &item_struct.fields {
            if unnamed.unnamed.len() == 1 {
                let inner = type_expr(&unnamed.unnamed[0].ty);
                let rendered = inner.render();
                return TypeDef::Newtype(inner, rendered);
            }
        }
        return TypeDef::Struct(StructDef { fields: Vec::new() });
    };
    let mut fields = Vec::new();
    for field in &named.named {
        let attrs = serde_attrs(&field.attrs);
        let ident = field
            .ident
            .as_ref()
            .expect("named field has an ident")
            .to_string();
        let wire_name = attrs
            .rename
            .clone()
            .unwrap_or_else(|| match &container.rename_all {
                Some(rule) => apply_rename_all(rule, &ident),
                None => ident.clone(),
            });
        fields.push(SourceField {
            ident,
            ty: type_expr(&field.ty),
            optional: type_expr(&field.ty).option_inner().is_some(),
            default: attrs.default,
            skip_ser: attrs.skip || attrs.skip_ser,
            skip_ser_if: attrs.skip_ser_if,
            skip_deser: attrs.skip || attrs.skip_deser,
            flatten: attrs.flatten,
            wire_name,
        });
    }
    TypeDef::Struct(StructDef { fields })
}

fn index_type(
    index: &mut CrateIndex,
    module_path: &str,
    name: &str,
    file_path: &Path,
    def: TypeDef,
) {
    index
        .by_module
        .insert((module_path.to_string(), name.to_string()), def);
    index.module_files.insert(
        (module_path.to_string(), name.to_string()),
        file_path.to_path_buf(),
    );
    index
        .by_name
        .entry(name.to_string())
        .or_default()
        .push(module_path.to_string());
}

fn join_module(module_path: &str, child: &str) -> String {
    if module_path.is_empty() {
        child.to_string()
    } else {
        format!("{module_path}::{child}")
    }
}

/// Visible last segments → full path segments, from a file's `use` items.
fn collect_imports(file: &syn::File) -> BTreeMap<String, Vec<String>> {
    let mut imports = BTreeMap::new();
    for item in &file.items {
        let Item::Use(item_use) = item else { continue };
        collect_use_tree(&item_use.tree, &mut Vec::new(), &mut imports);
    }
    imports
}

fn collect_use_tree(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    imports: &mut BTreeMap<String, Vec<String>>,
) {
    match tree {
        UseTree::Path(use_path) => {
            prefix.push(use_path.ident.to_string());
            collect_use_tree(&use_path.tree, prefix, imports);
            prefix.pop();
        }
        UseTree::Name(use_name) => {
            let mut full = prefix.clone();
            full.push(use_name.ident.to_string());
            imports.insert(use_name.ident.to_string(), full);
        }
        UseTree::Rename(use_rename) => {
            let mut full = prefix.clone();
            full.push(use_rename.ident.to_string());
            imports.insert(use_rename.rename.to_string(), full);
        }
        UseTree::Glob(_) => {}
        UseTree::Group(use_group) => {
            for inner in &use_group.items {
                collect_use_tree(inner, prefix, imports);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Serde attribute extraction
// ---------------------------------------------------------------------------

#[derive(Default)]
struct SerdeAttrs {
    rename:      Option<String>,
    rename_all:  Option<String>,
    default:     bool,
    skip:        bool,
    skip_ser:    bool,
    /// `skip_serializing_if = …` — with `Option::is_none`, null is never
    /// emitted (the key is omitted instead).
    skip_ser_if: bool,
    skip_deser:  bool,
    flatten:     bool,
    tag:         Option<String>,
    content:     Option<String>,
    untagged:    bool,
    other:       bool,
}

fn serde_attrs(attrs: &[syn::Attribute]) -> SerdeAttrs {
    let mut parsed = SerdeAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let ok = attr.parse_nested_meta(|meta| {
            let name = meta
                .path
                .get_ident()
                .map(|ident| ident.to_string())
                .unwrap_or_default();
            let value = |meta: &syn::meta::ParseNestedMeta| -> Option<String> {
                meta.value()
                    .ok()?
                    .parse::<syn::LitStr>()
                    .ok()
                    .map(|lit| lit.value())
            };
            match name.as_str() {
                "rename" => parsed.rename = value(&meta),
                "rename_all" => parsed.rename_all = value(&meta),
                "tag" => parsed.tag = value(&meta),
                "content" => parsed.content = value(&meta),
                "default" => {
                    parsed.default = true;
                    // `default = "function"` — consume the value tokens.
                    if let Ok(value) = meta.value() {
                        let _ = value.parse::<syn::Expr>();
                    }
                }
                "skip" => parsed.skip = true,
                "skip_serializing" => parsed.skip_ser = true,
                "skip_serializing_if" => {
                    parsed.skip_ser_if = true;
                    // `skip_serializing_if = "predicate"` — consume the value.
                    if let Ok(value) = meta.value() {
                        let _ = value.parse::<syn::Expr>();
                    }
                }
                "skip_deserializing" => parsed.skip_deser = true,
                "flatten" => parsed.flatten = true,
                "untagged" => parsed.untagged = true,
                "other" => parsed.other = true,
                // Unknown items (skip_serializing_if, with, borrow, …)
                // must still consume their value tokens or
                // parse_nested_meta errors on the leftover input.
                _ => {
                    if let Ok(value) = meta.value() {
                        let _ = value.parse::<syn::Expr>();
                    }
                }
            }
            Ok(())
        });
        if ok.is_err() {
            // A serde attribute we cannot parse would silently weaken the
            // net; fail loudly with the attribute text so it can be added.
            panic!(
                "unparseable #[serde(...)] attribute: `{:?}` — extend serde_attrs in this test",
                attr.to_token_stream().to_string()
            );
        }
    }
    parsed
}

fn enum_def(
    variants: &syn::punctuated::Punctuated<syn::Variant, syn::token::Comma>,
    attrs: &[syn::Attribute],
) -> EnumDef {
    let container = serde_attrs(attrs);
    let mut names = Vec::new();
    let mut unit_only = true;
    let mut catch_all = false;
    for variant in variants {
        let variant_attrs = serde_attrs(&variant.attrs);
        if variant_attrs.other {
            // A catch-all accepts any string and is never emitted; it is
            // not part of the demanded enum set.
            catch_all = true;
            continue;
        }
        if variant_attrs.untagged {
            // An untagged variant serializes to its payload's own shape,
            // producing no tag value and no fixed name.
            unit_only = false;
            continue;
        }
        let name = variant_attrs
            .rename
            .clone()
            .unwrap_or_else(|| match &container.rename_all {
                Some(rule) => apply_rename_all(rule, &variant.ident.to_string()),
                None => variant.ident.to_string(),
            });
        names.push(name);
        if !matches!(variant.fields, syn::Fields::Unit) {
            unit_only = false;
        }
    }
    EnumDef {
        tag: container.tag,
        content: container.content,
        untagged: container.untagged,
        unit_only,
        variants: names,
        catch_all,
    }
}

/// Apply serde `rename_all` casing to a field or variant identifier.
fn apply_rename_all(rename_all: &str, ident: &str) -> String {
    let words = split_ident_words(ident);
    match rename_all {
        "camelCase" => words
            .iter()
            .enumerate()
            .map(|(index, word)| {
                if index == 0 {
                    word.to_lowercase()
                } else {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(first) => {
                            first.to_uppercase().collect::<String>()
                                + &chars.as_str().to_lowercase()
                        }
                        None => String::new(),
                    }
                }
            })
            .collect(),
        "PascalCase" => words
            .iter()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => {
                        first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                    }
                    None => String::new(),
                }
            })
            .collect(),
        "snake_case" => words
            .iter()
            .map(|w| w.to_lowercase())
            .collect::<Vec<_>>()
            .join("_"),
        "kebab-case" => words
            .iter()
            .map(|w| w.to_lowercase())
            .collect::<Vec<_>>()
            .join("-"),
        "lowercase" => words.join("").to_lowercase(),
        "UPPERCASE" => words.join("").to_uppercase(),
        "SCREAMING_SNAKE_CASE" => words.join("_").to_uppercase(),
        _ => ident.to_string(),
    }
}

fn split_ident_words(ident: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_uppercase = false;
    for ch in ident.chars() {
        if ch == '_' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_uppercase = false;
        } else if ch.is_uppercase() && !previous_uppercase && !current.is_empty() {
            words.push(std::mem::take(&mut current));
            current.push(ch);
            previous_uppercase = true;
        } else {
            current.push(ch);
            previous_uppercase = ch.is_uppercase();
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    if words.is_empty() {
        words.push(ident.to_string());
    }
    words
}

// ---------------------------------------------------------------------------
// Wire-kind analysis
// ---------------------------------------------------------------------------

/// What a declared type means on the wire, after transparency strips
/// (`Option`, `Box`, `Cow`, newtypes, `ulid_id!` expansion).
#[derive(Debug, Clone, PartialEq)]
enum RustKind {
    Str,
    Bool,
    Int,
    Float,
    Array(Box<RustKind>),
    Map,
    Any,
    Named(ResolvedType),
    Unknown,
}

fn rust_kind(ty: &TypeExpr, in_crate: &str, in_module: &str, file: &Path, depth: u8) -> RustKind {
    if depth > 8 {
        return RustKind::Unknown;
    }
    if let Some(inner) = ty.option_inner() {
        return rust_kind(inner, in_crate, in_module, file, depth + 1);
    }
    let Some(segments) = ty.path_segments() else {
        return RustKind::Unknown;
    };
    let last = segments
        .last()
        .map(|segment| segment.ident.as_str())
        .unwrap_or_default();
    match last {
        "String" | "str" | "char" => return RustKind::Str,
        "bool" => return RustKind::Bool,
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
        | "isize" | "NonZeroU8" | "NonZeroU16" | "NonZeroU32" | "NonZeroU64" | "NonZeroI8"
        | "NonZeroI16" | "NonZeroI32" | "NonZeroI64" => return RustKind::Int,
        "f32" | "f64" => return RustKind::Float,
        "Value" => return RustKind::Any,
        "Box" | "Arc" | "Rc" | "Cow" => {
            if let Some(inner) = ty.final_args().first() {
                return rust_kind(inner, in_crate, in_module, file, depth + 1);
            }
        }
        "Vec" | "VecDeque" => {
            if let Some(inner) = ty.final_args().first() {
                let element = rust_kind(inner, in_crate, in_module, file, depth + 1);
                return RustKind::Array(Box::new(element));
            }
            return RustKind::Unknown;
        }
        "HashMap" | "BTreeMap" => return RustKind::Map,
        "DateTime" | "NaiveDateTime" | "NaiveDate" | "Uuid" | "Ulid" | "PathBuf" => {
            return RustKind::Str;
        }
        "Duration" => return RustKind::Unknown,
        _ => {}
    }
    match indexes().resolve_reference(segments, in_crate, in_module, file) {
        Some(resolved) if resolved.manual_serde => {
            match manual_wire_kind(&resolved.crate_name, &resolved.name) {
                Some(ManualWire::Str) => RustKind::Str,
                _ => RustKind::Named(resolved),
            }
        }
        Some(resolved) => match &resolved.def {
            // Newtype transparency: compare the wrapped kind, but keep
            // the named type for $ref name-consistency upstream.
            TypeDef::Newtype(inner, _) => {
                let wrapped = rust_kind(
                    inner,
                    &resolved.crate_name,
                    &resolved.module,
                    &resolved.file,
                    depth + 1,
                );
                match wrapped {
                    RustKind::Unknown => RustKind::Named(resolved),
                    other => other,
                }
            }
            _ => RustKind::Named(resolved),
        },
        None => RustKind::Unknown,
    }
}

// ---------------------------------------------------------------------------
// The comparison engine
// ---------------------------------------------------------------------------

fn check_mapping(schema_name: &str, rust_path: &str, findings: &mut Vec<String>) -> usize {
    let mut comparisons = 0usize;
    let prefix = format!("schema `{schema_name}` → {rust_path}");

    let resolved = match indexes().resolve(rust_path) {
        Some(resolved) => resolved,
        None => {
            findings.push(format!(
                "{prefix}: the Rust type does not resolve — is the table's path spelled \
                 correctly, or does the type come from a macro this net must learn about?"
            ));
            return 0;
        }
    };

    let schema = match schema_by_name(schema_name) {
        Some(schema) => schema,
        None => {
            if VESTIGIAL_MAPPINGS.contains(&(schema_name, rust_path)) {
                return 0;
            }
            findings.push(format!(
                "{prefix}: no schema named `{schema_name}` exists anywhere in the spec — \
                 the with_replacement entry is stale and shadows nothing; delete it"
            ));
            return 0;
        }
    };

    // Follow schema-level $ref aliases.
    let mut schema_value = schema;
    for _ in 0..8 {
        match ref_target(schema_value) {
            Some(target) => schema_value = target,
            None => break,
        }
    }

    let manual = resolved
        .manual_serde
        .then(|| manual_wire_kind(&resolved.crate_name, &resolved.name))
        .flatten();
    match manual {
        Some(ManualWire::Str) => {
            // The whole type rides the wire as a string.
            let shape = spec_shape(schema_value, 0);
            if matches!(shape, SpecShape::Complex) {
                return 0;
            }
            let mut mismatches: Vec<String> = Vec::new();
            if kind_matches(&RustKind::Str, &shape, 0, &mut |mismatch| {
                mismatches.push(mismatch.to_string())
            }) {
                if !mismatches.is_empty() {
                    findings.push(format!(
                        "{prefix}: the spec says `{}` but the manual serde impl emits a string \
                         — {}",
                        render_schema_shape(schema_value),
                        mismatches.join("; ")
                    ));
                }
                return 1;
            }
            return 0;
        }
        None if resolved.manual_serde => {
            findings.push(format!(
                "{prefix}: the type has a hand-written `impl Serialize`, so its fields are not \
                 its wire surface — teach this net the shape by adding the type to \
                 MANUAL_WIRE_KINDS with the wire shape and a reason (work#1685)"
            ));
            return 0;
        }
        Some(ManualWire::Object { .. }) | None => {}
    }

    match &resolved.def {
        TypeDef::Struct(_) => {
            comparisons +=
                check_struct_fields(schema_name, rust_path, &resolved, schema_value, findings);
        }
        TypeDef::Enum(enum_def) => {
            comparisons += check_enum(schema_name, rust_path, enum_def, schema_value, findings);
        }
        TypeDef::Newtype(inner, rendered) => {
            comparisons += check_newtype_schema(
                schema_name,
                rust_path,
                schema_value,
                &resolved,
                inner,
                rendered,
                findings,
            );
        }
    }
    comparisons
}

/// Collect a struct's serde field surface, merging `#[serde(flatten)]`
/// targets (flatten is transparent on the wire).
fn container_fields(resolved: &ResolvedType, depth: u8) -> Vec<(SourceField, FieldContext)> {
    let TypeDef::Struct(struct_def) = &resolved.def else {
        return Vec::new();
    };
    let manual = resolved
        .manual_serde
        .then(|| manual_wire_kind(&resolved.crate_name, &resolved.name))
        .flatten();
    let omit_absent_options = match manual {
        Some(ManualWire::Object {
            omit_absent_options,
            ..
        }) => omit_absent_options,
        _ => false,
    };
    let mut out = Vec::new();
    for source_field in &struct_def.fields {
        let mut field = source_field.clone();
        if omit_absent_options && field.optional {
            field.skip_ser_if = true;
        }
        let context = FieldContext {
            origin:     format!("{}.{}", resolved.full_path(), field.ident),
            crate_name: resolved.crate_name.clone(),
            module:     resolved.module.clone(),
            file:       resolved.file.clone(),
        };
        if field.flatten {
            if depth > 4 {
                continue;
            }
            if let Some(target) = flattened_type(resolved, source_field) {
                // Flattened TAGGED enums contribute their tag/content
                // keys to the wire surface; untagged ones contribute
                // open-ended keys the net cannot enumerate.
                if let TypeDef::Enum(enum_def) = &target.def {
                    if let (Some(tag), Some(content)) = (&enum_def.tag, enum_def.content.as_ref()) {
                        for key in [tag, content] {
                            out.push((
                                SourceField {
                                    ident:       key.clone(),
                                    ty:          parse_type_expr("serde_json::Value"),
                                    optional:    false,
                                    default:     false,
                                    skip_ser:    false,
                                    skip_ser_if: false,
                                    skip_deser:  false,
                                    flatten:     false,
                                    wire_name:   key.clone(),
                                },
                                FieldContext {
                                    origin:     format!(
                                        "{} flattens enum tag `{}` key `{key}`",
                                        context.origin, target.name
                                    ),
                                    crate_name: context.crate_name.clone(),
                                    module:     context.module.clone(),
                                    file:       context.file.clone(),
                                },
                            ));
                        }
                        continue;
                    }
                }
                for (inner_field, inner_context) in container_fields(&target, depth + 1) {
                    out.push((inner_field, FieldContext {
                        origin:     format!("{} flattens {}", context.origin, inner_context.origin),
                        crate_name: inner_context.crate_name,
                        module:     inner_context.module,
                        file:       inner_context.file,
                    }));
                }
            }
            continue;
        }
        out.push((field.clone(), context));
    }
    if let Some(ManualWire::Object { extra_fields, .. }) = manual {
        for (name, kind) in extra_fields {
            let wire_kind = match kind {
                ManualFieldKind::Str => "String",
            };
            out.push((
                SourceField {
                    ident:       (*name).to_string(),
                    ty:          parse_type_expr(wire_kind),
                    optional:    false,
                    default:     false,
                    skip_ser:    false,
                    skip_ser_if: false,
                    skip_deser:  false,
                    flatten:     false,
                    wire_name:   (*name).to_string(),
                },
                FieldContext {
                    origin:     format!(
                        "{} (computed by the manual Serialize impl)",
                        resolved.full_path()
                    ),
                    crate_name: resolved.crate_name.clone(),
                    module:     resolved.module.clone(),
                    file:       resolved.file.clone(),
                },
            ));
        }
    }
    out
}

fn flattened_type(resolved: &ResolvedType, field: &SourceField) -> Option<ResolvedType> {
    let segments = field.ty.path_segments()?;
    indexes().resolve_reference(
        segments,
        &resolved.crate_name,
        &resolved.module,
        &resolved.file,
    )
}

fn check_struct_fields(
    schema_name: &str,
    rust_path: &str,
    resolved: &ResolvedType,
    schema: &Value,
    findings: &mut Vec<String>,
) -> usize {
    let properties = spec_get(schema, "properties").and_then(Value::as_mapping);
    let required: BTreeSet<String> = spec_get(schema, "required")
        .and_then(Value::as_sequence)
        .map(|seq| {
            seq.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let fields = container_fields(resolved, 0);
    let mut comparisons = 0usize;

    // A schema without its own `properties` (a bare `oneOf` of variant
    // objects, presence-combo arms) has no field surface to compare.
    let Some(properties) = properties else {
        return 0;
    };

    // Spec properties must exist on the Rust side — but only when the
    // schema REQUIRES them: a non-required property the type never emits
    // still admits every wire payload (absence is allowed), so it is
    // over-promise, not a break. Required + absent on the wire IS a break.
    for (prop_key, prop_schema) in properties {
        let Some(prop_name) = prop_key.as_str() else {
            continue;
        };
        let Some((field, context)) = fields
            .iter()
            .find(|(field, _)| field.wire_name == prop_name)
        else {
            let known_drift = KNOWN_DRIFT
                .iter()
                .any(|(schema, prop, _)| *schema == schema_name && *prop == prop_name);
            if required.contains(prop_name) && !known_drift {
                findings.push(format!(
                    "schema `{schema_name}` property `{prop_name}` ({}) has no counterpart on \
                     `{rust_path}` but is `required` — the replacement type never emits it, so \
                     every wire payload violates the schema",
                    render_schema_shape(prop_schema)
                ));
            }
            continue;
        };
        comparisons += check_field(
            schema_name,
            prop_name,
            prop_schema,
            field,
            context,
            findings,
        );
    }

    // Rust fields must exist in the spec (presence axis) and serde-
    // mandatory fields must be listed in `required`.
    for (field, context) in &fields {
        if field.skip_ser {
            continue;
        }
        let present = properties.contains_key(Value::String(field.wire_name.clone()));
        if !present {
            findings.push(format!(
                "schema `{schema_name}` field `{}` ({}, Rust type `{}`): the field is missing \
                 from the schema's properties — the wire contract cannot express it",
                field.wire_name,
                context.origin,
                field.ty.render()
            ));
            continue;
        }
        comparisons += 1;
        if !field.optional
            && !field.default
            && !field.skip_deser
            && !required.contains(&field.wire_name)
        {
            findings.push(format!(
                "schema `{schema_name}` field `{}` ({}, Rust type `{}`): serde makes the field \
                 mandatory, but the schema's `required` does not list it — payloads that omit it \
                 fail the replacement type's deserialization; add it to `required`",
                field.wire_name,
                context.origin,
                field.ty.render()
            ));
        }
    }

    comparisons
}

/// Nullability + kind comparison for one property/field pair. Returns
/// whether a kind comparison was possible.
fn check_field(
    schema_name: &str,
    prop_name: &str,
    prop_schema: &Value,
    field: &SourceField,
    context: &FieldContext,
    findings: &mut Vec<String>,
) -> usize {
    let where_: String = format!(
        "schema `{schema_name}` field `{prop_name}` ({})",
        context.origin
    );

    // ---- nullability (the proven sprint-101 class) ----
    // `Option` + `skip_serializing_if = "Option::is_none"` never emits
    // null (the key is omitted), so a non-nullable spec type is honest.
    let demands_nullability = field.optional && !field.skip_ser_if;
    let nullable = schema_is_nullable(prop_schema, 0);
    if demands_nullability && !nullable {
        findings.push(format!(
            "{where_}: the Rust type is `{}` — null on the wire — but the spec says `{}` which \
             does not admit null; declare nullability in the schema, e.g. \
             `type: [<T>, null]` or `oneOf: [<shape>, {{type: null}}]`",
            field.ty.render(),
            render_schema_shape(prop_schema)
        ));
    } else if !field.optional && nullable {
        findings.push(format!(
            "{where_}: the spec says `{}` which admits null, but the Rust type `{}` cannot hold \
             null — payloads carrying null fail the replacement type's deserialization; \
             drop the null arm or make the field `Option` on the Rust side",
            render_schema_shape(prop_schema),
            field.ty.render()
        ));
    }

    // ---- kind comparison ----
    let shape = spec_shape(prop_schema, 0);
    if matches!(shape, SpecShape::Complex) {
        return 0;
    }
    let kind = rust_kind(
        &field.ty,
        &context.crate_name,
        &context.module,
        &context.file,
        0,
    );
    if matches!(kind, RustKind::Unknown) {
        return 0;
    }
    let mut mismatches: Vec<String> = Vec::new();
    if kind_matches(&kind, &shape, 0, &mut |mismatch| {
        mismatches.push(mismatch.to_string())
    }) {
        if !mismatches.is_empty() {
            findings.push(format!(
                "{where_}: the spec says `{}` but the Rust type `{}` is {} — {}",
                render_schema_shape(prop_schema),
                field.ty.render(),
                kind_description(&kind),
                mismatches.join("; ")
            ));
        }
        1
    } else {
        0
    }
}

/// Wire-kind comparison. Calls `report` with the mismatch explanation
/// and returns whether the comparison was actually performed.
fn kind_matches(
    kind: &RustKind,
    shape: &SpecShape,
    depth: u8,
    report: &mut dyn FnMut(&str),
) -> bool {
    if depth > 6 {
        return false;
    }
    match (kind, shape) {
        (_, SpecShape::Complex) => false,
        (RustKind::Any, _) => true,
        (RustKind::Unknown, _) => false,
        (RustKind::Str, SpecShape::Prim(t)) if t == "string" => true,
        (RustKind::Str, SpecShape::Enum(_)) => true,
        (RustKind::Bool, SpecShape::Prim(t)) if t == "boolean" => true,
        (RustKind::Int, SpecShape::Prim(t)) if t == "integer" || t == "number" => true,
        (RustKind::Float, SpecShape::Prim(t)) if t == "number" => true,
        (RustKind::Float, SpecShape::Prim(t)) if t == "integer" => {
            report("the wire can carry decimals the spec's `integer` forbids");
            true
        }
        (RustKind::Array(element), SpecShape::Array(items)) => match (element.as_ref(), items) {
            (_, None) => true,
            (RustKind::Unknown, _) => true,
            (element_kind, Some(item_shape)) => {
                kind_matches(element_kind, item_shape, depth + 1, report)
            }
        },
        (RustKind::Map, SpecShape::Object) => true,
        (RustKind::Named(resolved), SpecShape::Ref(target)) => {
            check_ref_name_consistency(resolved, target, report)
        }
        // A primitive Rust kind behind a $ref: the target's own shape
        // must match the primitive.
        (_, SpecShape::Ref(target)) => {
            match schema_by_name(target).map(|target| spec_shape(target, depth + 1)) {
                Some(target_shape) => kind_matches(kind, &target_shape, depth + 1, report),
                None => {
                    report(&format!(
                        "the $ref target `{target}` does not exist in the spec"
                    ));
                    true
                }
            }
        }
        (RustKind::Named(resolved), shape) => match &resolved.def {
            TypeDef::Newtype(inner, _) => {
                let wrapped = rust_kind(
                    inner,
                    &resolved.crate_name,
                    &resolved.module,
                    &resolved.file,
                    depth + 1,
                );
                kind_matches(&wrapped, shape, depth + 1, report)
            }
            TypeDef::Struct(_) => match shape {
                SpecShape::Object => true,
                other => {
                    report(&format!(
                        "the replacement is a struct and should be `$ref`'d or `type: object` \
                         (found {})",
                        shape_summary(other)
                    ));
                    true
                }
            },
            TypeDef::Enum(enum_def) => match shape {
                SpecShape::Prim(t) if t == "string" && enum_def.unit_only && !enum_def.untagged => {
                    true
                }
                SpecShape::Enum(values) if enum_def.unit_only && !enum_def.untagged => {
                    let mut mismatches = Vec::new();
                    check_enum_values(enum_def, values, &mut mismatches);
                    if !mismatches.is_empty() {
                        report(&mismatches.join("; "));
                    }
                    true
                }
                SpecShape::Object => true, // tagged enums ride objects/oneOf
                _ => false,
            },
        },
        _ => {
            report(&format!(
                "the wire shape ({}) does not match the declared schema kind ({})",
                kind_summary(kind),
                shape_summary(shape)
            ));
            true
        }
    }
}

fn kind_summary(kind: &RustKind) -> String {
    match kind {
        RustKind::Str => "string".to_string(),
        RustKind::Bool => "boolean".to_string(),
        RustKind::Int => "integer".to_string(),
        RustKind::Float => "number".to_string(),
        RustKind::Array(_) => "array".to_string(),
        RustKind::Map => "object (map)".to_string(),
        RustKind::Any => "any JSON".to_string(),
        RustKind::Named(resolved) => resolved.name.clone(),
        RustKind::Unknown => "unknown".to_string(),
    }
}

fn shape_summary(shape: &SpecShape) -> String {
    match shape {
        SpecShape::Prim(t) => format!("type: {t}"),
        SpecShape::Array(_) => "type: array".to_string(),
        SpecShape::Object => "type: object".to_string(),
        SpecShape::Ref(target) => format!("$ref: {target}"),
        SpecShape::Enum(_) => "enum".to_string(),
        SpecShape::Complex => "complex/unconstrained".to_string(),
    }
}

fn kind_description(kind: &RustKind) -> String {
    match kind {
        RustKind::Str => "a string on the wire".to_string(),
        RustKind::Bool => "a boolean on the wire".to_string(),
        RustKind::Int => "an integer on the wire".to_string(),
        RustKind::Float => "a float on the wire".to_string(),
        RustKind::Array(_) => "an array on the wire".to_string(),
        RustKind::Map => "an object (map) on the wire".to_string(),
        RustKind::Any => "any JSON value".to_string(),
        RustKind::Named(resolved) => format!("the type `{}`", resolved.full_path()),
        RustKind::Unknown => "an unknown shape".to_string(),
    }
}

/// A named Rust type behind a `$ref` property should point at the
/// schema the registry maps it to (when such a mapping exists).
fn check_ref_name_consistency(
    resolved: &ResolvedType,
    target: &str,
    report: &mut dyn FnMut(&str),
) -> bool {
    let table = replacement_table();
    let exact: Vec<&(String, String)> = table
        .iter()
        .filter(|(_, path)| path == &resolved.full_path())
        .collect();
    let expected = match exact.len() {
        1 => Some(exact[0].0.as_str()),
        0 => {
            let by_name: Vec<&(String, String)> = table
                .iter()
                .filter(|(_, path)| path.rsplit("::").next() == Some(resolved.name.as_str()))
                .collect();
            if by_name.len() == 1 {
                Some(by_name[0].0.as_str())
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(expected) = expected {
        if target != expected {
            report(&format!(
                "the Rust type `{}` is mapped as schema `{expected}`, so the property should \
                 `$ref` `{expected}`, not `{target}`",
                resolved.full_path()
            ));
        }
        return true;
    }
    if schema_by_name(target).is_none() {
        report(&format!(
            "the $ref target `{target}` does not exist in the spec"
        ));
        return true;
    }
    false
}

fn check_enum_values(enum_def: &EnumDef, spec_values: &[String], mismatches: &mut Vec<String>) {
    for variant in &enum_def.variants {
        if !spec_values.contains(variant) {
            mismatches.push(format!(
                "the Rust enum emits `{variant}` which the spec's enum forbids"
            ));
        }
    }
    if !enum_def.catch_all {
        for value in spec_values {
            if !enum_def.variants.contains(value) {
                mismatches.push(format!(
                    "the spec's enum admits `{value}` which the Rust enum cannot parse"
                ));
            }
        }
    }
}

fn check_enum(
    schema_name: &str,
    rust_path: &str,
    enum_def: &EnumDef,
    schema: &Value,
    findings: &mut Vec<String>,
) -> usize {
    let prefix = format!("schema `{schema_name}` → {rust_path}");
    if enum_def.untagged {
        return 0;
    }

    // Internally/adjacently tagged enums: compare the discriminator.
    if enum_def.tag.is_some() {
        return check_tagged_enum(schema_name, rust_path, enum_def, schema, findings);
    }

    // Unit-only enums: compare `enum:` variant sets.
    if enum_def.unit_only {
        let SpecShape::Enum(values) = spec_shape(schema, 0) else {
            return 0;
        };
        let mut mismatches = Vec::new();
        check_enum_values(enum_def, &values, &mut mismatches);
        if !mismatches.is_empty() {
            findings.push(format!(
                "{prefix}: the Rust enum variants do not match the spec's enum — {}",
                mismatches.join("; ")
            ));
        }
        return 1;
    }
    0
}

fn check_tagged_enum(
    schema_name: &str,
    rust_path: &str,
    enum_def: &EnumDef,
    schema: &Value,
    findings: &mut Vec<String>,
) -> usize {
    let prefix = format!("schema `{schema_name}` → {rust_path}");
    let Some(tag) = &enum_def.tag else { return 0 };
    let mut comparisons = 0usize;

    if let Some(discriminator) = spec_get(schema, "discriminator") {
        if let Some(property_name) = spec_get(discriminator, "propertyName").and_then(Value::as_str)
        {
            if property_name != tag {
                findings.push(format!(
                    "{prefix}: the spec's discriminator rides `{property_name}` but the Rust enum's \
                     serde tag is `{tag}` — the tag property name drifted"
                ));
            }
            comparisons += 1;
        }

        if let Some(mapping) = spec_get(discriminator, "mapping").and_then(Value::as_mapping) {
            let mut spec_keys: Vec<String> = mapping
                .keys()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            let mut rust_variants = enum_def.variants.clone();
            spec_keys.sort();
            rust_variants.sort();
            let mut mismatches = Vec::new();
            for variant in &rust_variants {
                if !spec_keys.contains(variant) {
                    mismatches.push(format!(
                        "the Rust enum emits tag `{variant}` which the discriminator mapping cannot route"
                    ));
                }
            }
            for key in &spec_keys {
                if !rust_variants.contains(key) {
                    mismatches.push(format!(
                        "the discriminator mapping routes `{key}` which the Rust enum cannot parse"
                    ));
                }
            }
            if !mismatches.is_empty() {
                findings.push(format!(
                    "{prefix}: the discriminator mapping does not match the Rust enum variants — {}",
                    mismatches.join("; ")
                ));
            }
            comparisons += 1;
        }
        return comparisons;
    }

    // No discriminator keyword: the tag rides a plain property of this
    // schema (`type: {enum: [...]}`), or each `oneOf` arm carries it.
    if let Some(tag_property) = spec_get(schema, "properties").and_then(|props| {
        spec_get(props, tag).and_then(|prop| prop.get("enum").and_then(Value::as_sequence))
    }) {
        let spec_values: Vec<String> = tag_property
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        let mut mismatches = Vec::new();
        check_enum_values(enum_def, &spec_values, &mut mismatches);
        if !mismatches.is_empty() {
            findings.push(format!(
                "{prefix}: the Rust enum variants do not match the schema's `{tag}` enum — {}",
                mismatches.join("; ")
            ));
        }
        // Adjacent content: the content key must exist as a property.
        if let Some(content) = &enum_def.content {
            let has_content = spec_get(schema, "properties")
                .and_then(|props| props.as_mapping())
                .is_some_and(|props| props.contains_key(Value::String(content.clone())));
            if !has_content {
                findings.push(format!(
                    "{prefix}: the Rust enum's adjacent content key `{content}` is missing from \
                     the schema's properties"
                ));
            }
        }
        return 1;
    }

    // Bare `oneOf` of variant objects: each arm should carry the tag as
    // a single-value enum, and the arms together must cover the variants.
    if let Some(arms) = spec_get(schema, "oneOf").and_then(Value::as_sequence) {
        let refs: Vec<String> = arms
            .iter()
            .filter_map(|arm| {
                arm.get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|r| r.rsplit('/').next())
                    .map(str::to_string)
            })
            .collect();
        if refs.len() != arms.len() || refs.is_empty() {
            return 0;
        }
        let mut spec_tags: Vec<String> = Vec::new();
        for arm in &refs {
            let Some(arm_schema) = schema_by_name(arm) else {
                continue;
            };
            let Some(arm_tags) = spec_get(arm_schema, "properties")
                .and_then(|props| spec_get(props, tag))
                .and_then(|prop| prop.get("enum").and_then(Value::as_sequence))
            else {
                continue;
            };
            spec_tags.extend(
                arm_tags
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            );
        }
        if spec_tags.is_empty() {
            return 0;
        }
        spec_tags.sort();
        let mut rust_variants = enum_def.variants.clone();
        rust_variants.sort();
        let mut mismatches = Vec::new();
        check_enum_values(enum_def, &spec_tags, &mut mismatches);
        if refs.len() != enum_def.variants.len() {
            mismatches.push(format!(
                "the spec's oneOf has {} arms but the Rust enum has {} variants",
                refs.len(),
                enum_def.variants.len()
            ));
        }
        if !mismatches.is_empty() {
            findings.push(format!(
                "{prefix}: the oneOf arms' `{tag}` discriminators do not match the Rust enum \
                 variants — {}",
                mismatches.join("; ")
            ));
        }
        return 1;
    }

    comparisons
}

fn check_newtype_schema(
    schema_name: &str,
    rust_path: &str,
    schema: &Value,
    resolved: &ResolvedType,
    inner: &TypeExpr,
    inner_rendered: &str,
    findings: &mut Vec<String>,
) -> usize {
    let where_ = format!("schema `{schema_name}` → {rust_path} (newtype over {inner_rendered})");
    let shape = spec_shape(schema, 0);
    if matches!(shape, SpecShape::Complex) {
        return 0;
    }
    let kind = rust_kind(
        inner,
        &resolved.crate_name,
        &resolved.module,
        &resolved.file,
        0,
    );
    if matches!(kind, RustKind::Unknown) {
        return 0;
    }
    let mut mismatches: Vec<String> = Vec::new();
    if kind_matches(&kind, &shape, 0, &mut |mismatch| {
        mismatches.push(mismatch.to_string())
    }) {
        if !mismatches.is_empty() {
            findings.push(format!(
                "{where_}: the spec says `{}` — {}",
                render_schema_shape(schema),
                mismatches.join("; ")
            ));
        }
        1
    } else {
        0
    }
}
