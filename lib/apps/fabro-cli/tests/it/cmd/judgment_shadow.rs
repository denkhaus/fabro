//! fabro-8e13 step 4: the judgment-shadow hook script against a scripted
//! twin of the System One endpoint. Spawns the real nu script (the same
//! artifact the engine hook executes) with FABRO_HOOK_CONTEXT pointing at
//! a fixture and the endpoint override aimed at a local httpmock server,
//! then asserts the fabro-judgment-v1 line. Skips when nu is not on PATH
//! (the toolchain image carries the vendored nu).

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "spawns the nu hook script synchronously; test infrastructure, not a Tokio path"
)]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::Command;

use httpmock::MockServer;

fn nu_available() -> bool {
    Command::new("nu")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn judgment_shadow_records_answers_from_a_scripted_twin() {
    if !nu_available() {
        return;
    }
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method("POST").path("/v1/systemone");
        then.status(200).json_body(serde_json::json!({
            "answers": {
                "verdict_pre_screen": {"type": "choice", "choice": "approved"},
                "residue:src/extra.rs": {"type": "choice", "choice": "harmless_churn"}
            },
            "usage": {"cost_usd": 0.0012}
        }));
    });

    let temp = std::env::temp_dir().join(format!("judgment-shadow-{}", std::process::id()));
    std::fs::create_dir_all(&temp).expect("temp dir");
    let context_path = temp.join("hook.json");
    let mut file = std::fs::File::create(&context_path).expect("context file");
    write!(
        file,
        r#"{{"event":"stage_complete","node_id":"reviewer","run_id":"twin-test","context_updates":{{"anomaly_files":["src/extra.rs"]}}}}"#
    )
    .expect("context fixture");

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map(std::path::Path::to_path_buf)
        .expect("repo root");
    let stream = repo_root.join(".fabro/judgments/twin-test.jsonl");
    let _ = std::fs::remove_file(&stream);

    let output = Command::new("nu")
        .arg(".fabro/scripts/judgment-shadow.nu")
        .current_dir(&repo_root)
        .env("FABRO_HOOK_CONTEXT", &context_path)
        .env("JUDGMENT_SHADOW_ENDPOINT", server.url("/v1/systemone"))
        .env("TYPESAFE_API_KEY", "twin-key")
        .output()
        .expect("the script runs");
    assert!(
        output.status.success(),
        "script failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let line = std::fs::read_to_string(&stream).expect("the judgment line");
    let entry: serde_json::Value = serde_json::from_str(line.trim()).expect("valid JSON line");
    assert_eq!(entry["schema"], "fabro-judgment-v1");
    assert_eq!(entry["run_id"], "twin-test");
    assert_eq!(entry["node"], "reviewer");
    assert!(
        entry["degraded"].is_null(),
        "a twin answer is not degraded: {entry}"
    );
    assert_eq!(
        entry["answers"]["verdict_pre_screen"]["choice"], "approved",
        "the twin's verdict answer lands verbatim: {entry}"
    );
    assert_eq!(
        entry["answers"]["residue:src/extra.rs"]["choice"], "harmless_churn",
        "the twin's residue answer lands verbatim: {entry}"
    );

    let _ = std::fs::remove_file(&stream);
    let _ = std::fs::remove_file(&context_path);
    let _ = std::fs::remove_dir(&temp);
    let _ = std::fs::remove_dir(repo_root.join(".fabro/judgments"));
}
