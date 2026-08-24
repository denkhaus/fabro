//! Client-side manifest build with a dockerfile-based environment: the
//! project config's `image.dockerfile = { path = ... }` reference must be
//! bundled into the workflow files map so the server can inline it, and the
//! settings resolve must succeed even when a server-stored environment
//! catalog carries `image.docker` for the same id (higher layer wins).

use std::collections::HashMap;

use fabro_config::SettingsLayer;
use fabro_manifest::{ManifestBuildInput, build_run_manifest};

#[test]
fn manifest_build_bundles_dockerfile_and_resolves_over_stored_image() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".fabro/workflows/hello")).unwrap();
    std::fs::write(
        root.join(".fabro/project.toml"),
        r#"_version = 1

[run.environment]
id = "mise"

[environments.mise]
provider = "docker"

[environments.mise.image]
dockerfile = { path = "Dockerfile.mise" }
"#,
    )
    .unwrap();
    std::fs::write(root.join(".fabro/Dockerfile.mise"), "FROM ubuntu:24.04\n").unwrap();
    std::fs::write(
        root.join(".fabro/workflows/hello/workflow.toml"),
        "_version = 1\n\n[workflow]\ngraph = \"workflow.fabro\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join(".fabro/workflows/hello/workflow.fabro"),
        r#"digraph Hello { start [shape=Mdiamond] greet [prompt="hi"] exit [shape=Msquare] start -> greet -> exit }"#,
    )
    .unwrap();

    let user = root.join("user-settings.toml");
    std::fs::write(
        &user,
        "_version = 1\n\n[cli.target]\ntype = \"http\"\nurl = \"http://127.0.0.1:32276\"\n",
    )
    .unwrap();

    // Server-stored environment shape (environments API): mise registered
    // with a finished image.docker, exactly like the denkhaus lab store.
    let stored: SettingsLayer = r#"
[environments.mise]
provider = "docker"

[environments.mise.image]
docker = "fabro-runner:mise"
"#
    .parse()
    .unwrap();

    let built = build_run_manifest(ManifestBuildInput {
        workflow:             root.join(".fabro/workflows/hello/workflow.fabro"),
        cwd:                  root.to_path_buf(),
        run_overrides:        None,
        cli_overrides:        None,
        input_overrides:      HashMap::new(),
        args:                 None,
        environment_defaults: stored.environments,
        user_settings_path:   Some(user),
    })
    .expect("manifest build with dockerfile environment should succeed");

    let body = serde_json::to_value(&built.manifest).unwrap();
    let files = body["workflows"][".fabro/workflows/hello/workflow.fabro"]["files"]
        .as_object()
        .expect("workflow files map");
    let dockerfile = files
        .get(".fabro/Dockerfile.mise")
        .expect("Dockerfile.mise must be bundled");
    assert_eq!(dockerfile["content"], "FROM ubuntu:24.04\n");
    assert_eq!(dockerfile["ref"]["type"], "dockerfile");
}
