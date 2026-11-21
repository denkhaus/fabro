//! Fork-only presence and behavior pins for the environment cook's
//! single-flight machinery (work#1440, work#1454). A `just vendor
//! import` from a wrong fork state rewrites `vendor/PINS.toml` cleanly
//! and would silently drop the feature; these tests keep it observable.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::image_flight;

/// 2020-01-01T00:00:00Z: an mtime no fresh clone would carry.
fn aged_mtime() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_577_836_800)
}

/// Stamps every regular file under `root` with `mtime`, so a fixture
/// can masquerade as a fresh clone or an aged checkout.
fn stamp_mtimes(dir: &Path, mtime: SystemTime) {
    for entry in std::fs::read_dir(dir).expect("read fixture dir") {
        let path = entry.expect("fixture entry").path();
        if path.is_dir() {
            stamp_mtimes(&path, mtime);
        } else {
            std::fs::File::open(&path)
                .expect("open fixture file")
                .set_modified(mtime)
                .expect("stamp fixture mtime");
        }
    }
}

/// A throwaway git checkout fixture: a directory with `.git`, one
/// tracked-looking file, a nested file, and the two skip-rule
/// candidates (a root `.dockerignore` and a nested `.git` directory).
fn context_fixture(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "petri-image-flight-{}-{}",
        label,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("nested/.git")).expect("create nested fixture dirs");
    std::fs::create_dir_all(root.join(".git")).expect("create root .git");
    std::fs::write(root.join("nested/keep.txt"), "kept").expect("write nested file");
    std::fs::write(root.join(".dockerignore"), "ignored").expect("write dockerignore");
    std::fs::write(root.join("nested/.git/objects"), "ignored").expect("write nested git file");
    root
}

/// The manifest digest is stable for an unchanged tree and skips what
/// `pack_context` skips: `.git` at every level and a root
/// `.dockerignore` do not move the digest, but a file's content (via
/// size) does.
#[test]
fn manifest_digest_is_stable_and_skips_the_pack_skip_list() {
    let root = context_fixture("digest");
    let one = image_flight::manifest_digest("FROM busybox", &root).expect("first digest");
    let two = image_flight::manifest_digest("FROM busybox", &root).expect("second digest");
    assert_eq!(one, two, "an unchanged tree must digest identically");

    // The Dockerfile text participates.
    let other = image_flight::manifest_digest("FROM alpine", &root).expect("alpine digest");
    assert_ne!(one, other, "a changed Dockerfile must move the digest");

    // Skip rules: mutating excluded entries never moves the digest.
    std::fs::write(root.join(".dockerignore"), "changed").expect("rewrite dockerignore");
    std::fs::write(root.join("nested/.git/objects"), "changed").expect("rewrite nested git");
    let skipped = image_flight::manifest_digest("FROM busybox", &root).expect("skip digest");
    assert_eq!(one, skipped, "excluded entries must not move the digest");

    // A size change in a kept file does.
    std::fs::write(root.join("nested/keep.txt"), "kept longer").expect("grow kept file");
    let grown = image_flight::manifest_digest("FROM busybox", &root).expect("grown digest");
    assert_ne!(one, grown, "a changed file must move the digest");

    let _ = std::fs::remove_dir_all(&root);
}

/// THE clone-stability pin (work#1454): every run checkout is a FRESH
/// clone/pack, so the same trunk tip arrives under a different set of
/// mtimes in every root. Two roots carrying IDENTICAL content but
/// different mtimes MUST digest equal — under the mtime manifest they
/// did not, so the tag-keyed single-flight guard never grouped
/// concurrent cooks (seven distinct `petri-env-` tags for one trunk
/// tip in 13 minutes, fabro container logs 2026-11-18). There is no
/// end-to-end concurrent-cook harness in this suite — the cook path
/// runs only against a real docker provider — so "same content, one
/// build" rests on this pin composed with `guard_for_is_per_tag`:
/// equal digests mint the equal tag that maps to one guard, hence one
/// build.
#[test]
fn manifest_digest_is_clone_stable_across_different_mtimes() {
    let aged = context_fixture("clone-aged");
    let fresh = context_fixture("clone-fresh");
    stamp_mtimes(&aged, aged_mtime());
    stamp_mtimes(&fresh, SystemTime::now());
    let one = image_flight::manifest_digest("FROM busybox", &aged).expect("aged digest");
    let two = image_flight::manifest_digest("FROM busybox", &fresh).expect("fresh digest");
    assert_eq!(
        one, two,
        "identical content must digest equal across different mtimes"
    );
    let _ = std::fs::remove_dir_all(&aged);
    let _ = std::fs::remove_dir_all(&fresh);
}

/// Content-sensitivity pin (work#1454): two roots identical except ONE
/// file's content — SAME size, SAME mtimes — MUST digest differently.
/// This is the over-normalization guard: it proves content itself
/// moves the digest, not merely size or time, and under the mtime
/// manifest such an equal-size edit aliased into the other root's tag.
#[test]
fn manifest_digest_tracks_content_beyond_size_and_mtime() {
    let one_root = context_fixture("content-a");
    let other_root = context_fixture("content-b");
    // "kept" -> "kepT": four bytes either way, mtimes frozen equal.
    std::fs::write(other_root.join("nested/keep.txt"), "kepT").expect("swap content");
    stamp_mtimes(&one_root, aged_mtime());
    stamp_mtimes(&other_root, aged_mtime());
    let one = image_flight::manifest_digest("FROM busybox", &one_root).expect("first digest");
    let two = image_flight::manifest_digest("FROM busybox", &other_root).expect("second digest");
    assert_ne!(one, two, "an equal-size content edit must move the digest");
    let _ = std::fs::remove_dir_all(&one_root);
    let _ = std::fs::remove_dir_all(&other_root);
}

/// A directory without `.git` is refused: the context must be a git
/// checkout, exactly as `pack_context` demands.
#[test]
fn manifest_digest_refuses_a_non_checkout() {
    let root = std::env::temp_dir().join(format!("petri-image-flight-bare-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create bare fixture");
    let verdict = image_flight::manifest_digest("FROM busybox", &root)
        .expect_err("a bare directory must be refused");
    assert!(
        verdict.contains("no .git"),
        "the refusal must name the cause: {verdict}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The single-flight registry hands every caller of a tag the SAME
/// guard, and different tags different guards.
#[test]
fn guard_for_is_per_tag() {
    let a1 = image_flight::guard_for("petri-env-flight-a");
    let a2 = image_flight::guard_for("petri-env-flight-a");
    let b = image_flight::guard_for("petri-env-flight-b");
    assert!(Arc::ptr_eq(&a1, &a2), "one tag, one guard");
    assert!(!Arc::ptr_eq(&a1, &b), "distinct tags hold distinct guards");
}

/// The fast-path ledger records built tags and drops exactly what the
/// daemon-side sweep evicts. The ledger is process-global, so the
/// test rides unique tags and leaves them evicted.
#[tokio::test]
async fn ledger_round_trips_build_and_eviction() {
    let tag = "petri-env-flight-ledger";
    image_flight::note_built(tag);
    assert!(
        image_flight::is_built(tag),
        "a cooked tag is on the fast path"
    );
    image_flight::forget(&[tag.to_owned(), "petri-env-flight-never".to_owned()]);
    assert!(
        !image_flight::is_built(tag),
        "an evicted tag leaves the fast path"
    );
}

/// Presence pin: the ceiling and prefix exist with the decided shape
/// (work#1440: count-form ceiling, `petri-env-` prefix keyed by the
/// daemon-side sweep).
#[test]
fn ceiling_and_prefix_keep_their_decided_shape() {
    assert_eq!(image_flight::ENV_TAG_PREFIX, "petri-env-");
    assert_eq!(image_flight::ENV_IMAGE_CEILING, 6);
}

// Cross-process single-flight pins (work#1463). The cooks run one
// process per gear, so the in-process guard/ledger see only THIS
// process's cooks; the daemon presence probe is the cross-process
// half. No end-to-end concurrent-cook harness exists in this suite
// (the cook path runs only against a real docker provider), so the
// pins drive `image_build::cook_path` — the exact decision the ensure
// path composes with its ledger miss and the probe — the pure seam
// the item named for the simulation.
mod cross_process {
    use sandbox_driver::Error;

    use crate::image_build::{CookPath, cook_path};
    use crate::image_flight;

    /// Pin (a) of work#1463, red under the old code: two cooks from
    /// two "processes" — the second's ledger is empty (a fresh process
    /// static) — MUST let the second reuse WITHOUT a build when the
    /// daemon already holds the tag. Observed before the fix: every
    /// process held its own empty ledger and guard registry and built
    /// (three mints of one tag, 2026-11-19 fabro container logs).
    #[test]
    fn a_daemon_present_second_process_reuses_without_a_build() {
        let tag = "petri-env-flight-cross-present";
        image_flight::forget(&[tag.to_owned()]); // the ledger starts EMPTY (fresh process)
        assert!(
            !image_flight::is_built(tag),
            "the pin must start from a ledger miss, as a fresh process does"
        );
        assert_eq!(
            cook_path(Ok(true), tag),
            CookPath::Reuse,
            "a presence hit must reuse"
        );
        assert!(
            image_flight::is_built(tag),
            "the reused tag joins the fast-path ledger"
        );
        image_flight::forget(&[tag.to_owned()]);
    }

    /// A daemon miss builds, and does NOT pre-seed the ledger: only a
    /// successful cook may (the build path's own `note_built`).
    #[test]
    fn a_daemon_absent_cook_builds_without_touching_the_ledger() {
        let tag = "petri-env-flight-cross-absent";
        image_flight::forget(&[tag.to_owned()]);
        assert_eq!(cook_path(Ok(false), tag), CookPath::Build);
        assert!(
            !image_flight::is_built(tag),
            "the decision alone must not mark the tag built"
        );
    }

    /// A probe failure fails OPEN to Build — a duplicate build whose
    /// layers the daemon dedupes, never a stuck cook.
    #[test]
    fn an_unavailable_probe_fails_open_to_build() {
        let tag = "petri-env-flight-cross-unavail";
        image_flight::forget(&[tag.to_owned()]);
        let probe = Err(Error::unsupported(sandbox_driver::Capability::OneShot));
        assert_eq!(cook_path(probe, tag), CookPath::Build);
        assert!(!image_flight::is_built(tag));
    }
}

/// THE STORM PIN (work#1551): N concurrent FIRST cooks of the same fresh
/// tip against a real Docker daemon must yield exactly ONE build and N-1
/// reuses. Every earlier fix looked green because no test ever drove
/// concurrent cooks — the in-process guard serializes same-process cooks,
/// so the pin spawns REAL CHILD PROCESSES (the gears run one cook process
/// each, work#1463) and counts builds on the daemon's own event stream:
/// every cook that runs the build one-shot starts one exit-0 container
/// FROM the cooked tag, so `container create` events carrying
/// `image == <tag>` count the builds that ran, whatever the tree under
/// test reports about itself.
///
/// Requires a reachable Docker daemon; the pin passes trivially without
/// one, as the conformance run does. Runs only on the fork feature
/// (`env-image-evict`): the cook path it drives is the fork's.
#[cfg(feature = "env-image-evict")]
mod storm {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::process;
    use std::env::current_exe;
    use std::time::{SystemTime, UNIX_EPOCH};

    use bollard::Docker;
    use bollard::image::RemoveImageOptions;
    use bollard::system::EventsOptions;
    use futures_util::StreamExt;
    use sandbox_driver_docker::DockerProvider;
    use std::sync::Arc;

    use crate::image_build::{EnvironmentBuild, ensure_environment_image};

    /// How many concurrent first cooks the storm drives (the incident:
    /// five gears, load 42; the decided pin floor is three).
    const LANES: usize = 4;
    /// The build's slow layer, widening the first-probe window so every
    /// lane probes before any mint — the storm the incident observed.
    const BUILD_SLEEP_SECS: u64 = 6;
    /// The child lane's environment: the context root, the Dockerfile's
    /// path, the holder name, and the result file to write.
    const CHILD_FIXTURE: &str = "PETRI_STORM_FIXTURE";
    const CHILD_DOCKERFILE: &str = "PETRI_STORM_DOCKERFILE";
    const CHILD_HOLDER: &str = "PETRI_STORM_HOLDER";
    const CHILD_RESULT: &str = "PETRI_STORM_RESULT";

    fn read_dockerfile(fixture: &std::path::Path) -> String {
        std::fs::read_to_string(fixture.join("Dockerfile.toolchain"))
            .expect("the fixture's dockerfile reads back")
    }

    fn nonce() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is past the epoch")
            .as_nanos()
    }

    /// One throwaway git checkout with a manifest and a source file: a
    /// fresh tip good enough to cook — a REAL committed repository, as
    /// `pack_context` clones it. The Dockerfile carries the run's nonce
    /// in a LABEL — Dockerfile text is a digest input in every digest
    /// mode, so each pin run mints its own tag and cannot collide with
    /// a leftover from an earlier run.
    fn storm_fixture(run: u128) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "petri-image-flight-storm-{run}-{}",
            process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).expect("create fixture dirs");
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"storm-pin\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .expect("write fixture manifest");
        std::fs::write(root.join("src/lib.rs"), "pub fn probe() {}\n")
            .expect("write fixture source");
        std::fs::write(root.join("src/only-source.txt"), "source-only content\n")
            .expect("write fixture source-only file");
        std::fs::write(
            root.join("Dockerfile.toolchain"),
            format!(
                "FROM busybox:1.36\n\
                 LABEL petri.storm-nonce=\"{run}\"\n\
                 RUN sleep {BUILD_SLEEP_SECS}\n\
                 COPY . /src\n"
            ),
        )
        .expect("write fixture dockerfile");
        let git = |args: &[&str]| {
            let status = process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args([
                    "-c",
                    "user.email=storm@example.invalid",
                    "-c",
                    "user.name=storm pin",
                ])
                .args(args)
                .status()
                .expect("run git");
            assert!(status.success(), "git {args:?} must succeed: {status}");
        };
        git(&["init", "--quiet"]);
        git(&["add", "-A"]);
        git(&["commit", "--quiet", "-m", "storm fixture"]);
        root
    }

    /// One cook lane, run exactly as a gear's process runs it. Started by
    /// the parent through the test binary's own name; without the
    /// environment it is a no-op, so an ordinary test run never fires it.
    #[tokio::test(flavor = "multi_thread")]
    async fn storm_child_lane() {
        let (Ok(fixture), Ok(dockerfile), Ok(holder), Ok(result)) = (
            std::env::var(CHILD_FIXTURE),
            std::env::var(CHILD_DOCKERFILE),
            std::env::var(CHILD_HOLDER),
            std::env::var(CHILD_RESULT),
        ) else {
            return;
        };
        let provider: Arc<dyn sandbox_driver::SandboxProvider> = Arc::new(
            DockerProvider::connect()
                .await
                .expect("the parent only spawns lanes against a live daemon"),
        );
        let cooked = ensure_environment_image(
            &provider,
            &EnvironmentBuild {
                dockerfile,
                context: fixture,
            },
            &holder,
        )
        .await;
        let report = match &cooked {
            Ok(tag) => serde_json::json!({ "tag": tag }),
            Err(error) => serde_json::json!({ "error": error.to_string() }),
        };
        std::fs::write(&result, report.to_string()).expect("write the lane's result");
        cooked.expect("the lane's cook must succeed");
    }

    /// Drives LANES concurrent first cooks of one fresh tip, one cook
    /// process per lane, then asserts the daemon saw exactly one build.
    /// RED against the probe-only tree (work#1463): every lane probes
    /// before the first mint, every lane builds, and the count is LANES,
    /// not 1 (the 2026-11-21 incident: five builds, load 42).
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_first_cooks_of_one_tip_build_exactly_once() {
        if DockerProvider::connect().await.is_err() {
            return; // no daemon: trivially green, as the conformance run is
        }
        let Ok(docker) = Docker::connect_with_local_defaults() else {
            return;
        };
        let run = nonce();
        let fixture = storm_fixture(run);
        let scratch = std::env::temp_dir().join(format!(
            "petri-image-flight-storm-results-{run}-{}",
            process::id()
        ));
        std::fs::create_dir_all(&scratch).expect("create result dir");

        let since = unix_now();
        let mut lanes = Vec::new();
        for lane in 0..LANES {
            lanes.push(
                process::Command::new(current_exe().expect("the test binary"))
                    .args(["image_flight_tests::storm::storm_child_lane", "--exact"])
                    .env(CHILD_FIXTURE, fixture.to_string_lossy().as_ref())
                    .env(CHILD_DOCKERFILE, read_dockerfile(&fixture))
                    .env(CHILD_HOLDER, format!("petri-storm-{run}-{lane}-env-build"))
                    .env(
                        CHILD_RESULT,
                        scratch.join(format!("lane-{lane}.json")).to_string_lossy().as_ref(),
                    )
                    .spawn()
                    .expect("spawn a cook lane"),
            );
        }
        for mut lane in lanes {
            let status = lane.wait().expect("wait for the cook lane");
            assert!(status.success(), "a cook lane failed: {status}");
        }
        let until = unix_now();

        let mut tags = Vec::new();
        for lane in 0..LANES {
            let report = std::fs::read_to_string(scratch.join(format!("lane-{lane}.json")))
                .expect("every lane writes its result");
            let report: serde_json::Value =
                serde_json::from_str(&report).expect("a lane's result is JSON");
            assert!(
                report["error"].is_null(),
                "lane {lane} failed its cook: {report}"
            );
            tags.push(
                report["tag"]
                    .as_str()
                    .expect("a lane names its tag")
                    .to_owned(),
            );
        }
        let tag = tags[0].clone();
        assert!(
            tags.iter().all(|other| other == &tag),
            "one tip must mint one tag across lanes: {tags:?}"
        );

        // The daemon's own account of how many builds ran: one exit-0
        // container from the tag per build one-shot.
        let builds = count_build_containers(&docker, &tag, since, until).await;

        let _ = std::fs::remove_dir_all(&scratch);
        let _ = std::fs::remove_dir_all(&fixture);
        let _ = docker
            .remove_container(
                &format!("petri-cook-lock-{tag}"),
                None::<bollard::container::RemoveContainerOptions>,
            )
            .await;
        let _ = docker
            .remove_image(
                &tag,
                Some(RemoveImageOptions {
                    force: true,
                    ..Default::default()
                }),
                None,
            )
            .await;

        assert_eq!(
            builds, 1,
            "exactly one build must serve {LANES} concurrent first cooks of one tip \
             (observed {builds} build containers; the 2026-11-21 storm class, work#1551)"
        );
    }

    /// Counts `container create` events whose image is `tag` between the
    /// two unix timestamps: the daemon-side build counter.
    async fn count_build_containers(
        docker: &Docker,
        tag: &str,
        since: i64,
        until: i64,
    ) -> usize {
        let mut filters = HashMap::new();
        filters.insert("type".to_owned(), vec!["container".to_owned()]);
        filters.insert("event".to_owned(), vec!["create".to_owned()]);
        let options = EventsOptions::<String> {
            since: Some(since.to_string()),
            until: Some(until.to_string()),
            filters,
        };
        let mut stream = docker.events(Some(options));
        let mut builds = 0;
        while let Some(event) = stream.next().await {
            let event = event.expect("the daemon's event stream must read");
            if event
                .actor
                .as_ref()
                .and_then(|actor| actor.attributes.as_ref())
                .and_then(|attributes| attributes.get("image"))
                .is_some_and(|image| image == tag)
            {
                builds += 1;
            }
        }
        builds
    }

    fn unix_now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is past the epoch")
            .as_secs() as i64
    }
}
