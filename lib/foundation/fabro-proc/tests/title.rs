//! The process-title platform contract (fabro-4d35): `title_set` must land
//! in `/proc/<pid>/cmdline`, because the scenario suites find workers by
//! their retitled command line (`pgrep -f "^fabro <run id> "`). A host
//! toolchain that silently drops cc-object constructors turns this pin red
//! in seconds instead of hanging every petri scenario for a minute.

#![cfg(target_os = "linux")]

use std::process::Command;
use std::time::{Duration, Instant};

const PROBE_TITLE: &str = "fabro 01PROBEPROBE start";
const PROBE_ENV: &str = "FABRO_PROC_TITLE_PROBE_CHILD";

/// The child half of the self-exec probe: capture argv, retitle, and hold
/// the process open long enough for the parent to read `/proc`.
#[test]
fn title_probe_child() {
    #[expect(
        clippy::disallowed_methods,
        reason = "sync env read in a self-exec probe child outside any Tokio runtime"
    )]
    if std::env::var_os(PROBE_ENV).is_none() {
        return;
    }
    let len = fabro_proc::title_init();
    assert!(len > 0, "the ctor must capture the argv span (len = {len})");
    fabro_proc::title_set(PROBE_TITLE);
    #[expect(
        clippy::disallowed_methods,
        reason = "sync sleep in a probe child outside any Tokio runtime"
    )]
    std::thread::sleep(Duration::from_secs(15));
}

#[test]
fn title_set_lands_in_proc_cmdline() {
    #[expect(
        clippy::disallowed_methods,
        reason = "intentional synchronous subprocess: the probe child must run before \
                  any runtime exists so only the ctor under test initializes it"
    )]
    let mut child = Command::new(std::env::current_exe().expect("the test binary path"))
        .arg("--exact")
        .arg("title_probe_child")
        .env(PROBE_ENV, "1")
        .spawn()
        .expect("the probe child spawns");
    let pid = child.id();
    let cmdline_path = std::path::PathBuf::from("/proc")
        .join(pid.to_string())
        .join("cmdline");
    let deadline = Instant::now() + Duration::from_secs(10);
    let observed = loop {
        #[expect(
            clippy::disallowed_methods,
            reason = "sync poll of /proc in a test thread"
        )]
        let Ok(bytes) = std::fs::read(&cmdline_path) else {
            #[expect(
                clippy::disallowed_methods,
                reason = "sync poll sleep in a test thread"
            )]
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        let first = bytes
            .split(|byte| *byte == 0)
            .next()
            .map(|token| String::from_utf8_lossy(token).into_owned())
            .unwrap_or_default();
        if first == PROBE_TITLE || deadline < Instant::now() {
            break first;
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "sync poll sleep in a test thread"
        )]
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        observed, PROBE_TITLE,
        "title_set must rewrite /proc/<pid>/cmdline — a host toolchain that drops \
         cc-object constructors (fabro-4d35: split .init_array, dead ctor) fails \
         here; fix the toolchain (observed: rustc 1.98.1 objects + 2022-era linkers \
         split .init_array; rustc 1.99 + lld merge it), not this test"
    );
}
