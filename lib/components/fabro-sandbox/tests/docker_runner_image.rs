#![cfg(feature = "docker")]

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use bollard::Docker;
use bollard::image::RemoveImageOptions;
use fabro_sandbox::config::DockerfileSource;
use fabro_sandbox::{DockerSandbox, DockerSandboxOptions, Sandbox, SandboxEvent};

fn event_names() -> (Arc<Mutex<Vec<String>>>, fabro_sandbox::SandboxEventCallback) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let callback: fabro_sandbox::SandboxEventCallback = Arc::new(move |event: SandboxEvent| {
        if let Ok(mut names) = sink.lock() {
            names.push(event_name(&event).to_string());
        }
    });
    (seen, callback)
}

/// Worked example, derived independently from the documented tagging scheme
/// (`fabro-runner-` + first 12 hex chars of sha256 of the dockerfile content)
/// so a scheme change fails here instead of silently tracking the code.
fn expected_runner_tag(dockerfile: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(dockerfile.as_bytes());
    format!("fabro-runner-{}", &hex::encode(digest)[..12])
}

fn event_name(event: &SandboxEvent) -> &'static str {
    match event {
        SandboxEvent::Initializing { .. } => "initializing",
        SandboxEvent::Ready { .. } => "ready",
        SandboxEvent::SnapshotCreating { .. } => "snapshot_creating",
        SandboxEvent::SnapshotReady { .. } => "snapshot_ready",
        SandboxEvent::SnapshotFailed { .. } => "snapshot_failed",
        SandboxEvent::InitializeFailed { .. }
        | SandboxEvent::CleanupStarted { .. }
        | SandboxEvent::CleanupCompleted { .. }
        | SandboxEvent::CleanupFailed { .. }
        | SandboxEvent::StartStarted { .. }
        | SandboxEvent::StartCompleted { .. }
        | SandboxEvent::StartFailed { .. }
        | SandboxEvent::StopStarted { .. }
        | SandboxEvent::StopCompleted { .. }
        | SandboxEvent::StopFailed { .. }
        | SandboxEvent::DeleteStarted { .. }
        | SandboxEvent::DeleteCompleted { .. }
        | SandboxEvent::DeleteFailed { .. }
        | SandboxEvent::SnapshotPulling { .. }
        | SandboxEvent::GitCloneStarted { .. }
        | SandboxEvent::GitCloneCompleted { .. }
        | SandboxEvent::GitCloneFailed { .. } => "irrelevant",
    }
}

fn unique_dockerfile() -> String {
    let salt = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("FROM buildpack-deps:noble\n# fabro test salt {salt}\n")
}

async fn initialize_and_collect(dockerfile: &str) -> Vec<String> {
    let mut sandbox = DockerSandbox::new(
        DockerSandboxOptions {
            dockerfile: Some(DockerfileSource::Inline(dockerfile.to_string())),
            skip_clone: true,
            ..DockerSandboxOptions::default()
        },
        None,
        None,
        None,
        None,
        None,
    )
    .expect("sandbox construction should succeed");
    let (seen, callback) = event_names();
    sandbox.set_event_callback(callback);
    sandbox
        .initialize()
        .await
        .expect("initialize should succeed");
    let mut names = seen.lock().unwrap().clone();
    names.retain(|name| name != "irrelevant");
    names
}

#[tokio::test]
#[ignore = "requires real Docker daemon and base image pull; run explicitly when changing runner image builds"]
async fn runner_image_builds_once_per_content_hash() {
    let Ok(docker) = Docker::connect_with_local_defaults() else {
        return;
    };

    let first = unique_dockerfile();
    let second = unique_dockerfile();
    assert_ne!(first, second);

    let events_first = initialize_and_collect(&first).await;
    assert!(
        events_first.contains(&"snapshot_creating".to_string()),
        "first use must build the image: {events_first:?}"
    );
    assert!(
        events_first.contains(&"snapshot_ready".to_string()),
        "built image must be reported ready: {events_first:?}"
    );

    // Same content again: image exists, no build, but ready is reported like
    // the pull path reports an already-local image.
    let events_reuse = initialize_and_collect(&first).await;
    assert!(
        !events_reuse.contains(&"snapshot_creating".to_string()),
        "unchanged dockerfile must not rebuild: {events_reuse:?}"
    );
    assert!(
        events_reuse.contains(&"snapshot_ready".to_string()),
        "reused image must be reported ready: {events_reuse:?}"
    );

    // Different content: a new image is built.
    let events_changed = initialize_and_collect(&second).await;
    assert!(
        events_changed.contains(&"snapshot_creating".to_string()),
        "changed dockerfile must rebuild: {events_changed:?}"
    );

    // Built images carry the Fabro managed label so inventory/GC can find them.
    let inspected = docker
        .inspect_image(&expected_runner_tag(&first))
        .await
        .expect("built image must exist on the daemon");
    let labels = inspected
        .config
        .as_ref()
        .and_then(|config| config.labels.clone())
        .unwrap_or_default();
    assert_eq!(
        labels.get("sh.fabro.managed").map(String::as_str),
        Some("true"),
        "built runner image must carry the managed label: {labels:?}"
    );

    // Cleanup: remove both test images so the daemon is not polluted.
    for dockerfile in [&first, &second] {
        let tag = expected_runner_tag(dockerfile);
        let _ = docker
            .remove_image(
                &tag,
                Some(RemoveImageOptions {
                    force: true,
                    ..RemoveImageOptions::default()
                }),
                None,
            )
            .await;
    }
}
