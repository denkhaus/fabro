//! A sandbox a test deletes even when it fails.
//!
//! A live test that creates a provider sandbox and deletes it on its last
//! line leaks a running (and billed) sandbox whenever it panics or fails an
//! assertion before that line. [`DeletedOnDrop`] owns the sandbox for the
//! test: the happy path still calls `delete` explicitly, and any other exit
//! deletes it from `Drop`.
//!
//! `Drop` is synchronous and may run while the test's runtime is unwinding
//! a panic, so the cleanup never uses that runtime: it spawns a thread with
//! a small runtime of its own and blocks until the delete finishes or a
//! bounded timeout passes. A live provider's handle cannot be driven from
//! that thread either, because its pooled HTTP connections are tasks on the
//! test's runtime, which nobody polls while it unwinds. The guard therefore
//! connects the provider afresh through the [`ProviderAccess`] the test
//! built the sandbox with and deletes the sandbox by id over that new
//! connection.

use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use fabro_types::SandboxProviderKind;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::time;

use crate::driver::ProviderAccess;
use crate::driver_sandbox::RunSandbox;
use crate::error::display_for_log;
use crate::provider_sandbox;

/// How long a drop-time delete may take before the guard gives up and
/// reports the sandbox as possibly leaked. Daytona's driver bounds each
/// delete call at 10s and may wait out a state change once; a reconnect
/// adds a few seconds of its own.
const DROP_DELETE_TIMEOUT: Duration = Duration::from_secs(90);

/// A run sandbox that is deleted when the guard drops, unless the test
/// deleted it explicitly through [`DeletedOnDrop::delete`].
///
/// Derefs to the [`RunSandbox`] so a test reads the same as before; code
/// that needs a shared handle takes one from [`DeletedOnDrop::shared`].
pub struct DeletedOnDrop {
    sandbox: Arc<RunSandbox>,
    /// Access for a fresh provider connection at drop time. `None` deletes
    /// through the handle the sandbox already holds.
    access:  Option<ProviderAccess>,
    deleted: bool,
}

impl DeletedOnDrop {
    /// Guards a sandbox built through `access`, as every live provider test
    /// builds one. A drop-time delete reconnects the provider through
    /// `access` and deletes the sandbox by id.
    pub fn new(sandbox: impl Into<Arc<RunSandbox>>, access: &ProviderAccess) -> Self {
        Self {
            sandbox: sandbox.into(),
            access:  Some(access.clone()),
            deleted: false,
        }
    }

    /// Guards a sandbox whose own handle can finish a delete from any
    /// thread: the scripted double, whose delete needs no live connection.
    /// Not for a live provider, whose handle is bound to the test's runtime
    /// (see the module docs).
    pub fn through_handle(sandbox: impl Into<Arc<RunSandbox>>) -> Self {
        Self {
            sandbox: sandbox.into(),
            access:  None,
            deleted: false,
        }
    }

    /// A shared handle to the sandbox for code that takes an `Arc`, such as
    /// a workflow runner or an agent environment. The guard keeps its own
    /// and still deletes the sandbox when it drops.
    #[must_use]
    pub fn shared(&self) -> Arc<RunSandbox> {
        Arc::clone(&self.sandbox)
    }

    /// Deletes the sandbox now, returning the driver's result. After a
    /// successful delete the drop does nothing; after a failed one it tries
    /// once more so a transient failure still leaves nothing behind.
    pub async fn delete(mut self) -> crate::Result<()> {
        let result = self.sandbox.delete().await;
        self.deleted = result.is_ok();
        result
    }
}

impl Deref for DeletedOnDrop {
    type Target = RunSandbox;

    fn deref(&self) -> &RunSandbox {
        &self.sandbox
    }
}

impl fmt::Debug for DeletedOnDrop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeletedOnDrop")
            .field("kind", self.sandbox.kind())
            .field("id", &self.sandbox.sandbox_info())
            .field("deleted", &self.deleted)
            .finish_non_exhaustive()
    }
}

impl Drop for DeletedOnDrop {
    #[expect(
        clippy::print_stderr,
        reason = "The guard runs during a failing test; its report has to reach the captured test output."
    )]
    fn drop(&mut self) {
        if self.deleted {
            return;
        }
        let id = self.sandbox.sandbox_info();
        if id.is_empty() {
            // Never created on the provider: nothing to delete.
            return;
        }
        let kind = self.sandbox.kind().clone();
        eprintln!("DeletedOnDrop: deleting the {kind} sandbox {id} the test left behind");
        let outcome = delete_on_own_thread(
            kind.clone(),
            id.clone(),
            Arc::clone(&self.sandbox),
            self.access.clone(),
        );
        match outcome {
            Ok(()) => eprintln!("DeletedOnDrop: deleted the {kind} sandbox {id}"),
            Err(error) => {
                eprintln!("DeletedOnDrop: the {kind} sandbox {id} may be leaked: {error}");
            }
        }
    }
}

/// Runs the delete to completion on a dedicated thread with its own
/// runtime, bounded by [`DROP_DELETE_TIMEOUT`].
#[expect(
    clippy::disallowed_methods,
    reason = "Drop is synchronous and the test's runtime may be unwinding; the delete needs a thread and runtime of its own."
)]
fn delete_on_own_thread(
    kind: SandboxProviderKind,
    id: String,
    sandbox: Arc<RunSandbox>,
    access: Option<ProviderAccess>,
) -> Result<(), String> {
    let thread = std::thread::Builder::new()
        .name("sandbox-delete-on-drop".to_string())
        .spawn(move || -> Result<(), String> {
            let runtime = RuntimeBuilder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("could not build a runtime for the delete: {error}"))?;
            runtime.block_on(async {
                time::timeout(
                    DROP_DELETE_TIMEOUT,
                    delete_afresh(&kind, &id, &sandbox, access.as_ref()),
                )
                .await
                .map_err(|_| {
                    format!(
                        "the delete did not finish within {}s",
                        DROP_DELETE_TIMEOUT.as_secs()
                    )
                })?
            })
        })
        .map_err(|error| format!("could not spawn the delete thread: {error}"))?;
    thread
        .join()
        .map_err(|_| "the delete thread panicked".to_string())?
}

/// Deletes sandbox `id` over a fresh provider connection when `access` is
/// given, through the sandbox's own handle otherwise.
async fn delete_afresh(
    kind: &SandboxProviderKind,
    id: &str,
    sandbox: &RunSandbox,
    access: Option<&ProviderAccess>,
) -> Result<(), String> {
    let Some(access) = access else {
        return sandbox
            .delete()
            .await
            .map_err(|error| display_for_log(&error));
    };
    let fresh = provider_sandbox::attach_provider_sandbox(
        kind.clone(),
        access,
        id,
        false,
        sandbox.working_directory().to_string(),
        None,
        None,
        None,
    )
    .await
    .map_err(|error| format!("could not reconnect: {}", display_for_log(&error)))?;
    fresh
        .delete()
        .await
        .map_err(|error| display_for_log(&error))
}

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;

    use super::*;
    use crate::test_support::MockSandbox;

    #[tokio::test]
    async fn deletes_once_when_dropped_without_an_explicit_delete() {
        let mock = MockSandbox::default();
        let guard = DeletedOnDrop::through_handle(mock.sandbox());
        assert_eq!(
            guard.working_directory(),
            "/work",
            "reads through to the sandbox"
        );
        assert_eq!(mock.driver().delete_count(), 0);

        drop(guard);

        assert_eq!(mock.driver().delete_count(), 1);
    }

    #[tokio::test]
    async fn an_explicit_delete_runs_once() {
        let mock = MockSandbox::default();
        let guard = DeletedOnDrop::through_handle(mock.sandbox());
        let shared = guard.shared();

        guard.delete().await.unwrap();

        assert_eq!(mock.driver().delete_count(), 1);
        drop(shared);
        assert_eq!(
            mock.driver().delete_count(),
            1,
            "a shared handle does not delete"
        );
    }

    #[test]
    fn a_panic_before_the_delete_still_deletes_once() {
        let mock = MockSandbox::default();
        let sandbox = mock.sandbox();

        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = DeletedOnDrop::through_handle(sandbox);
            panic!("the test failed before its delete");
        }));

        assert!(outcome.is_err(), "the panic still propagates");
        assert_eq!(mock.driver().delete_count(), 1);
    }

    #[tokio::test]
    async fn a_panic_inside_a_runtime_still_deletes_once() {
        let mock = MockSandbox::default();
        let sandbox = mock.sandbox();

        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let _guard = DeletedOnDrop::through_handle(sandbox);
            panic!("the test failed before its delete");
        }));

        assert!(outcome.is_err(), "the panic still propagates");
        assert_eq!(mock.driver().delete_count(), 1);
    }
}
