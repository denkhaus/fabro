//! Petri compiles at create time: `fabro_petri::check` materializes a bundle,
//! hands it to `Runtime::check`, and returns the admitted graphs or Petri's
//! diagnostics in Fabro's shape; `fabro_petri::admission` round-trips the
//! admitted graphs through Fabro's blob store.
//!
//! No sandbox plugin is needed: nothing here runs a graph.

#![expect(
    clippy::disallowed_methods,
    reason = "the tests read checked-in fixture files synchronously before any run"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fabro_auth::test_support::env_credential_source;
use fabro_llm::test_support::test_catalog;
use fabro_petri::admission;
use fabro_petri::check::{self, Bundle, CheckError, CheckRequest, DiagnosticSeverity, Launch};
use fabro_petri::runtime::{self, RuntimeSpec};
use fabro_store::{BlobStore, test_support};
use lithos_llm::catalog::ProviderId;

const COMMAND_WORKFLOW: &str = r#"digraph Command {
    graph [goal="Run one command"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    say [shape=parallelogram, script="echo hello from petri"]
    start -> say -> exit
}"#;

const UNKNOWN_ATTRIBUTE_WORKFLOW: &str = r#"digraph Bad {
    graph [goal="Refuse me"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    work [shape=box, prompt="Do the work", bogus="yes"]
    start -> work -> exit
}"#;

const UNKNOWN_MODEL_WORKFLOW: &str = r#"digraph Bad {
    graph [goal="Refuse me"]
    start [shape=Mdiamond]
    exit [shape=Msquare]
    work [shape=box, prompt="Do the work", model="no-such-model-9000"]
    start -> work -> exit
}"#;

const SETTINGS: &str = "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n";

/// The `.fabro/workflows/hello` bundle checked into this repository.
fn hello_bundle() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.fabro/workflows/hello")
}

fn bundle(files: &[(&str, &str)]) -> Bundle {
    Bundle {
        files:        files
            .iter()
            .map(|(path, text)| ((*path).to_string(), (*text).to_string()))
            .collect(),
        entrypoint:   "workflow.fabro".to_string(),
        project_toml: None,
    }
}

fn request(bundle: Bundle, runtime: RuntimeSpec) -> CheckRequest {
    CheckRequest {
        bundle,
        inputs: BTreeMap::new(),
        launch: Launch::default(),
        runtime,
    }
}

/// A runtime with a model client over the test catalog, with `openai`
/// eligible, as a server with an OpenAI key configured builds it.
fn runtime_with_openai() -> RuntimeSpec {
    let credentials = env_credential_source(|name| match name {
        "OPENAI_API_KEY" => Some("test-key".to_string()),
        _ => None,
    });
    let client = runtime::model_client(test_catalog(), credentials, None, &[ProviderId::new(
        "openai",
    )])
    .expect("the model client builds")
    .expect("openai is eligible");
    RuntimeSpec {
        model_client: Some(client),
        ..RuntimeSpec::default()
    }
}

#[tokio::test]
async fn the_hello_bundle_is_admitted_and_round_trips_through_the_blob_store() {
    let workflow = std::fs::read_to_string(hello_bundle().join("workflow.fabro"))
        .expect("the hello workflow is checked in");
    let settings = std::fs::read_to_string(hello_bundle().join("workflow.toml"))
        .expect("the hello settings are checked in");
    let request = request(
        bundle(&[("workflow.fabro", &workflow), ("workflow.toml", &settings)]),
        RuntimeSpec::default(),
    );

    let admitted = check::check(&request).expect("the hello bundle is admitted");

    assert!(
        admitted
            .warnings
            .iter()
            .all(|w| w.severity == DiagnosticSeverity::Warning),
        "{:?}",
        admitted.warnings
    );
    let blobs = BlobStore::new(test_support::in_memory_pool_with(&[
        fabro_db::BLOBS_MIGRATION_SQL,
    ]));
    let record = admission::persist(&blobs, &admitted)
        .await
        .expect("the graphs persist");
    assert!(record.children.is_empty());
    let (graph, children) = admission::load(&blobs, &record)
        .await
        .expect("the graphs load");
    assert_eq!(graph, admitted.graph);
    assert!(children.is_empty());
}

#[tokio::test]
async fn a_launch_binds_the_repository_and_the_model_default() {
    let repository = tempfile::tempdir().expect("a temp dir");
    let request = CheckRequest {
        bundle:  bundle(&[
            ("workflow.fabro", COMMAND_WORKFLOW),
            ("workflow.toml", SETTINGS),
        ]),
        inputs:  BTreeMap::new(),
        launch:  Launch {
            model:      Some("gpt-5.4".to_string()),
            provider:   None,
            repository: Some(repository.path().to_path_buf()),
        },
        runtime: RuntimeSpec::default(),
    };

    let admitted = check::check(&request).expect("the command bundle is admitted");

    let launch = &admitted.graph.params["fabro.launch"];
    assert_eq!(launch["model"], "gpt-5.4");
    assert_eq!(
        launch["clone"]["repository"],
        repository.path().to_string_lossy().as_ref()
    );
}

#[tokio::test]
async fn an_unknown_attribute_is_refused_with_petris_code() {
    let request = request(
        bundle(&[
            ("workflow.fabro", UNKNOWN_ATTRIBUTE_WORKFLOW),
            ("workflow.toml", SETTINGS),
        ]),
        RuntimeSpec::default(),
    );

    let Err(CheckError::Rejected(diagnostics)) = check::check(&request) else {
        panic!("an unknown attribute should be refused");
    };

    let error = diagnostics
        .iter()
        .find(|d| d.code == "attractor.unknown_attribute")
        .unwrap_or_else(|| panic!("no unknown-attribute diagnostic in {diagnostics:?}"));
    assert!(error.is_error());
    assert!(error.message.contains("bogus"), "{error:?}");
    assert_eq!(error.file, "workflow.fabro");
    assert!(error.line.is_some(), "{error:?}");
}

#[tokio::test]
async fn an_unknown_model_is_refused_at_admission_when_a_catalog_is_installed() {
    let request = request(
        bundle(&[
            ("workflow.fabro", UNKNOWN_MODEL_WORKFLOW),
            ("workflow.toml", SETTINGS),
        ]),
        runtime_with_openai(),
    );

    let Err(CheckError::Rejected(diagnostics)) = check::check(&request) else {
        panic!("an unknown model should be refused when the runtime has a catalog");
    };

    let error = diagnostics
        .iter()
        .find(|d| d.code == "attractor.model.unknown")
        .unwrap_or_else(|| panic!("no model diagnostic in {diagnostics:?}"));
    assert!(error.message.contains("no-such-model-9000"), "{error:?}");
}

#[tokio::test]
async fn a_known_model_is_pinned_at_admission() {
    let workflow = UNKNOWN_MODEL_WORKFLOW.replace("no-such-model-9000", "gpt-5.4");
    let request = request(
        bundle(&[("workflow.fabro", &workflow), ("workflow.toml", SETTINGS)]),
        runtime_with_openai(),
    );

    let admitted = check::check(&request).expect("a catalog model is admitted");

    let work = admitted
        .graph
        .body
        .nodes
        .iter()
        .find(|node| node.name == "work")
        .expect("the work node is in the graph");
    assert_eq!(work.step.config["provider"], "openai");
    assert_eq!(work.step.config["model"], "gpt-5.4");
    assert!(
        work.step.config.get("plan").is_some(),
        "{:?}",
        work.step.config
    );
}
