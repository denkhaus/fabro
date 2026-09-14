use async_trait::async_trait;

use crate::context::Context;
use crate::error::Result;
use crate::graph::Graph;
use crate::outcome::Outcome;
use crate::retry::RetryPolicy;

/// Which attempt of a stage execution this invocation is, and why the
/// previous attempt failed when there was one.
///
/// The executor's retry loop builds one per attempt. Session-continuing
/// handlers (agent stages) use the prior failure to continue the prior
/// conversation instead of re-sending the full stage prompt (fabro-183f);
/// every other handler ignores it and keeps its per-attempt behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AttemptInfo {
    /// 1-based attempt number within this stage execution.
    pub attempt:       u32,
    /// Summary of the failure that ended the previous attempt; `None` on
    /// the first attempt.
    pub prior_failure: Option<String>,
}

impl AttemptInfo {
    /// The first attempt of a stage execution.
    #[must_use]
    pub fn first() -> Self {
        Self {
            attempt:       1,
            prior_failure: None,
        }
    }

    /// A retry attempt continuing after `prior_failure` ended attempt
    /// `attempt - 1`.
    #[must_use]
    pub fn retry(attempt: u32, prior_failure: impl Into<String>) -> Self {
        Self {
            attempt,
            prior_failure: Some(prior_failure.into()),
        }
    }
}

#[async_trait]
pub trait NodeHandler<G: Graph>: Send + Sync {
    async fn execute(
        &self,
        node: &G::Node,
        context: &Context,
        graph: &G,
        attempt: &AttemptInfo,
    ) -> Result<Outcome<G::Meta>>;

    async fn context_for_edge_selection(&self, context: &Context, _graph: &G) -> Result<Context> {
        Ok(context.clone())
    }

    fn retry_policy(&self, _node: &G::Node, _graph: &G) -> RetryPolicy {
        RetryPolicy::none()
    }

    fn on_retries_exhausted(
        &self,
        _node: &G::Node,
        _last_outcome: Outcome<G::Meta>,
    ) -> Outcome<G::Meta> {
        Outcome::fail("max retries exceeded")
    }
}
