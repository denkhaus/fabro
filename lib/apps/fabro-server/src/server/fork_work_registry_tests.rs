//! Fork presence pin (fabro-f1a9): the declared work-project registry —
//! `[work.projects.<id>]` settings heal the registry at startup. These
//! tests exist only on the fork; a merge resolution that drops them
//! dropped the feature — restore it, never relax the test.

use std::collections::BTreeMap;

use fabro_config::{WorkLayer, WorkProjectLayer};
use fabro_work::WorkStore;

use super::fork_work_registry::{declared_projects, materialize};

fn complete_row() -> WorkProjectLayer {
    WorkProjectLayer {
        display_name:             Some("Fabro".to_owned()),
        tracker_provider:         Some("github".to_owned()),
        tracker_repo:             Some("denkhaus/work".to_owned()),
        target_repo:              Some("denkhaus/fabro".to_owned()),
        project_label:            Some("project:fabro".to_owned()),
        lane_labels:              Some(BTreeMap::from([(
            "loop".to_owned(),
            "lane:loop".to_owned(),
        )])),
        claim_label_prefix:       Some("fabro:run:".to_owned()),
        claim_comment_enabled:    Some(false),
        backpressure_open_pr_cap: Some(0),
    }
}

fn layer_with(row: WorkProjectLayer) -> WorkLayer {
    WorkLayer {
        projects: fabro_config::MergeMap([("fabro".to_owned(), row)].into_iter().collect()),
    }
}

/// A complete row converts with its declared values; the optional fields
/// default (explicit-only target, empty lanes, no claim comments, drained
/// backpressure) when the row leaves them out.
#[test]
fn a_complete_row_converts_and_defaults_apply() {
    let mut row = complete_row();
    row.target_repo = None;
    row.lane_labels = None;
    row.claim_comment_enabled = None;
    row.backpressure_open_pr_cap = None;
    // The claim-label prefix is defaulted, not required: the one constant
    // the settings materializer and the API upsert share.
    row.claim_label_prefix = None;
    let declared = declared_projects(&layer_with(row)).expect("converts");
    let project = &declared[0];
    assert_eq!(project.project_id, "fabro");
    assert_eq!(project.tracker_repo, "denkhaus/work");
    assert_eq!(project.claim_label_prefix, "fabro:run:");
    assert_eq!(project.target_repo, None);
    assert!(project.lane_labels.is_empty());
    assert!(!project.claim_comment_enabled);
    assert_eq!(project.backpressure_open_pr_cap, 0);
}

/// A half-declared row fails loudly, naming the project and the missing
/// fields — the overnight-outage class never ships silently.
#[test]
fn a_half_declared_row_fails_naming_project_and_fields() {
    let mut row = complete_row();
    row.tracker_repo = None;
    row.project_label = None;
    let error = declared_projects(&layer_with(row)).expect_err("refuses");
    assert!(error.contains("work.projects.fabro"), "{error}");
    assert!(error.contains("tracker_repo"), "{error}");
    assert!(error.contains("project_label"), "{error}");
}

/// The materializer upserts declared rows, heals a drifted field on the
/// next pass, and leaves undeclared rows alone (the section declares row
/// CONTENT, not membership).
#[tokio::test]
async fn materialize_upserts_heals_and_leaves_undeclared_rows_alone() {
    let directory = tempfile::tempdir().unwrap();
    let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let store = WorkStore::new(database.clone_pool());

    materialize(&store, &layer_with(complete_row()))
        .await
        .expect("declared row upserts");
    let stored = store.project("fabro").await.unwrap().expect("stored");
    assert_eq!(stored.display_name, "Fabro");

    // An undeclared row survives the pass untouched.
    store
        .upsert_project(&fabro_work::WorkProject {
            project_id:               "other".to_owned(),
            display_name:             "Undeclared".to_owned(),
            tracker_provider:         "github".to_owned(),
            tracker_repo:             "denkhaus/work".to_owned(),
            target_repo:              None,
            project_label:            "project:other".to_owned(),
            lane_labels:              BTreeMap::new(),
            claim_label_prefix:       "fabro:run:".to_owned(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        })
        .await
        .unwrap();

    // A drifted declared field heals to the declared value.
    let mut drifted = complete_row();
    drifted.display_name = Some("Fabro (drifted)".to_owned());
    materialize(&store, &layer_with(drifted))
        .await
        .expect("heal pass");
    let healed = store.project("fabro").await.unwrap().expect("still there");
    assert_eq!(healed.display_name, "Fabro (drifted)");
    let untouched = store
        .project("other")
        .await
        .unwrap()
        .expect("undeclared row");
    assert_eq!(untouched.display_name, "Undeclared");
}

/// The fire gate's four refusal shapes and its ready arm — the overnight
/// outage class must refuse loudly instead of firing blind.
#[tokio::test]
async fn binding_ready_refuses_every_dead_shape_and_accepts_a_wired_project() {
    use super::fork_work_registry::{BindingRefusal, binding_ready};

    let directory = tempfile::tempdir().unwrap();
    let database = fabro_db::Database::connect(directory.path().join("fabro.sqlite3"))
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let store = WorkStore::new(database.clone_pool());

    // Unbound: a row that predates the reference.
    assert_eq!(
        binding_ready(&store, None).await,
        Err(BindingRefusal::Unbound)
    );

    // Unknown: the named project is not registered.
    assert_eq!(
        binding_ready(&store, Some("absent")).await,
        Err(BindingRefusal::Unknown("absent".to_owned()))
    );

    // Unconfigured: present but with no tracker wiring to claim against.
    let mut blank = complete_row();
    blank.tracker_repo = Some(String::new());
    materialize(&store, &layer_with(blank))
        .await
        .expect("blank row lands");
    assert_eq!(
        binding_ready(&store, Some("fabro")).await,
        Err(BindingRefusal::Unconfigured("fabro".to_owned()))
    );

    // Ready: a wired declared project.
    materialize(&store, &layer_with(complete_row()))
        .await
        .expect("wired row lands");
    assert_eq!(binding_ready(&store, Some("fabro")).await, Ok(()));
}
