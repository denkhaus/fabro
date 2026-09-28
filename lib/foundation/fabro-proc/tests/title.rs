//! Exercise the real argv allocation in a separate process, including musl.
#![cfg(target_os = "linux")]
#![expect(
    clippy::disallowed_methods,
    reason = "the test launches an isolated copy of itself and reads its kernel process title"
)]

use std::process::Command;
use std::{env, fs};

const TITLE: &str = "fabro title-probe running";

#[test]
fn process_title_is_visible_in_proc() {
    let output = Command::new(env::current_exe().unwrap())
        .args(["--exact", "title_child", "--ignored", "--nocapture"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[test]
#[ignore = "launched by process_title_is_visible_in_proc to isolate argv mutation"]
fn title_child() {
    let len = fabro_proc::title_init();
    assert!(len > TITLE.len(), "no usable argv buffer: {len}");
    fabro_proc::title_set(TITLE);
    let cmdline = fs::read("/proc/self/cmdline").unwrap();
    assert_eq!(
        cmdline.split(|byte| *byte == 0).next().unwrap(),
        TITLE.as_bytes()
    );
    assert_eq!(fabro_proc::title_init(), len);
    fabro_proc::title_set("fabro done");
    let cmdline = fs::read("/proc/self/cmdline").unwrap();
    assert_eq!(
        cmdline.split(|byte| *byte == 0).next().unwrap(),
        b"fabro done"
    );
}
