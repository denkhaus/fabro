//! Fork presence pin (fabro-0c08): the checkpoint exec guard is a
//! fork-added module in the upstream-owned crate fabro-petri; its inline
//! tests die with the file in one merge resolution. This fork-only test
//! binary reds on removal by exercising the public guard surface.

use fabro_petri::checkpoint::GitOutput;
use fabro_petri::fork_exec_guard::{
    EXEC_RETRY_DELAYS, is_resource_unavailable, retry_on_resource_unavailable,
};

fn exhausted_output() -> GitOutput {
    GitOutput {
        success: false,
        stdout:  Vec::new(),
        stderr:  b"OCI runtime exec failed ... resource temporarily unavailable".to_vec(),
    }
}

fn ok_output() -> GitOutput {
    GitOutput {
        success: true,
        stdout:  Vec::new(),
        stderr:  Vec::new(),
    }
}

#[test]
fn the_guard_module_exists_with_its_contract() {
    assert_eq!(
        EXEC_RETRY_DELAYS.len(),
        3,
        "the bounded ladder stays three steps"
    );
    assert!(
        is_resource_unavailable(b"runc init error(s): ... Resource temporarily unavailable"),
        "the incident wording classifies"
    );
}

#[tokio::test]
async fn the_checkpoint_seam_recovers_after_process_exhaustion() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&calls);
    let output =
        retry_on_resource_unavailable("diff --numstat", &[std::time::Duration::ZERO], move || {
            let counter = std::sync::Arc::clone(&counter);
            async move {
                let seen = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                Ok(if seen == 1 {
                    exhausted_output()
                } else {
                    ok_output()
                })
            }
        })
        .await
        .expect("the retried attempt maps at the caller");
    assert!(output.success, "the checkpoint exec recovers");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}
