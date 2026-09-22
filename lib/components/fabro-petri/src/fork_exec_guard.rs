//! Fork exec guard (fabro-0c08): a sandbox container whose process table
//! is exhausted fails every NEW exec with the OCI 'resource temporarily
//! unavailable' class while the container itself stays alive — the
//! finalize diff then fails, the run dies, and un-checkpointed stage work
//! is lost (reproduced with a pids-limited container; seed evidence).
//!
//! Every checkpoint exec goes through one seam ([`checkpoint`'s `run`]),
//! which wraps its site dispatch here: a resource-exhausted failure is
//! retried on a bounded cooldown ladder; everything else (including the
//! final exhaustion) returns unchanged, so the existing typed error
//! mapping and semantics are preserved. Upstream owns `checkpoint.rs`;
//! this file is the fork-owned half of the seam.

use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

use crate::checkpoint::{CheckpointError, GitOutput};

/// The cooldown ladder after a resource-exhausted exec: three bounded
/// retries (the exhaustion clears as the container reaps stragglers;
/// longer ladders belong to the scheduler, not the checkpoint path).
pub const EXEC_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
];

/// Whether a failed exec's stderr carries the resource-exhaustion class:
/// the OCI/runc wording on `docker exec`, or the shell's fork failure.
#[must_use]
pub fn is_resource_unavailable(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr).to_lowercase();
    if !text.contains("resource temporarily unavailable") {
        return false;
    }
    text.contains("oci runtime")
        || text.contains("runc init")
        || text.contains("exec failed")
        || text.contains("can't fork")
        || text.contains("unable to spawn")
}

/// Runs `attempt`, retrying bounded times on the resource-exhaustion
/// class with a cooldown between tries. Any other outcome — success, a
/// different failure, or exhaustion after the last cooldown — returns as
///-is; the caller's existing error mapping stays authoritative.
pub async fn retry_on_resource_unavailable<F, Fut>(
    action: &str,
    delays: &[Duration],
    mut attempt: F,
) -> Result<GitOutput, CheckpointError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<GitOutput, CheckpointError>>,
{
    let mut ladder = delays.iter();
    loop {
        let output = attempt().await?;
        if output.success || !is_resource_unavailable(&output.stderr) {
            return Ok(output);
        }
        let Some(cooldown) = ladder.next() else {
            tracing::warn!(
                action,
                "checkpoint exec stayed resource-unavailable after bounded retries"
            );
            return Ok(output);
        };
        tracing::warn!(
            action,
            cooldown_secs = cooldown.as_secs(),
            "checkpoint exec hit process exhaustion; retrying after cooldown"
        );
        sleep(*cooldown).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exhausted() -> GitOutput {
        GitOutput {
            success: false,
            stdout:  Vec::new(),
            stderr:  b"OCI runtime exec failed: exec failed: unable to start container \
                      process: error executing setns process: Resource temporarily \
                      unavailable"
                .to_vec(),
        }
    }

    fn ok() -> GitOutput {
        GitOutput {
            success: true,
            stdout:  b"diff --git a/x b/x\n".to_vec(),
            stderr:  Vec::new(),
        }
    }

    fn refused() -> GitOutput {
        GitOutput {
            success: false,
            stdout:  Vec::new(),
            stderr:  b"fatal: not a git repository".to_vec(),
        }
    }

    #[tokio::test]
    async fn exhaustion_retries_until_the_process_table_clears() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&calls);
        let output =
            retry_on_resource_unavailable("diff", &[Duration::ZERO, Duration::ZERO], move || {
                let counter = std::sync::Arc::clone(&counter);
                async move {
                    let seen = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    Ok(if seen < 3 { exhausted() } else { ok() })
                }
            })
            .await
            .expect("the third attempt succeeds");
        assert!(output.success);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn other_failures_return_without_retry() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&calls);
        let output = retry_on_resource_unavailable("diff", &[Duration::ZERO], move || {
            let counter = std::sync::Arc::clone(&counter);
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(refused())
            }
        })
        .await
        .expect("the failure maps at the caller");
        assert!(!output.success, "the refusal returns unchanged");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "no retry outside the exhaustion class"
        );
    }

    #[tokio::test]
    async fn exhaustion_after_the_ladder_returns_the_last_output() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&calls);
        let output = retry_on_resource_unavailable("diff", &[Duration::ZERO], move || {
            let counter = std::sync::Arc::clone(&counter);
            async move {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(exhausted())
            }
        })
        .await
        .expect("bounded retries end with the caller's error mapping");
        assert!(!output.success);
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "one initial try plus one ladder step"
        );
    }

    #[test]
    fn classification_matches_the_incident_and_repro_wordings() {
        assert!(is_resource_unavailable(
            b"OCI runtime exec failed ... fork/exec /proc/self/fd/6: resource \
              temporarily unavailable"
        ));
        assert!(is_resource_unavailable(
            b"sh: can't fork: Resource temporarily unavailable"
        ));
        assert!(!is_resource_unavailable(b"fatal: not a git repository"));
        assert!(!is_resource_unavailable(
            b"error: pathspec 'x' did not match"
        ));
    }
}
