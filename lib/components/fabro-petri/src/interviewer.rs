//! The interviewer of a run nobody is watching.
//!
//! Until the questions adapter over Fabro's API lands (F3.2), a Petri run in
//! the server has no way to reach a person. A human gate that asks anyway
//! gets a failure that says so, the gate fails closed, and the reason
//! reaches the interview receipt, instead of a question that waits forever.

use petri_execution::{InterviewError, InterviewReply, InterviewRequest, Interviewer};
use tokio_util::sync::CancellationToken;

/// Fails every question with a clear error.
#[derive(Clone, Copy, Debug, Default)]
pub struct Unattended;

#[async_trait::async_trait]
impl Interviewer for Unattended {
    async fn reply(&self, request: InterviewRequest, _cancel: CancellationToken) -> InterviewReply {
        InterviewReply::Failed(InterviewError::new(format!(
            "node `{}` asked a question, but a Petri run has no interviewer yet: questions reach \
             nobody until the interview adapter lands",
            request.node
        )))
    }
}
