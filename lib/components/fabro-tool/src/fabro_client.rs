use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use fabro_api::types;
use fabro_store::platform_records::StagePosition;
use fabro_types::{
    PairId, PairMessageRecord, PairMessageRequest, PairRecord, PairTranscriptResponse, Run, RunId,
    RunIntent, RunPairStatusResponse, RunProjection, RunStreamItem, SessionId, StageId,
    WorkItemComment,
};
use tokio::time::sleep;

use crate::{FabroToolBackend, RunMarkOutcome, RunMergeResult, RunPushResolutionResult, common};

/// Transient title-update failures absorbed inside ONE tool call
/// (denkhaus/work#1545): retries beyond the first attempt.
const TITLE_UPDATE_RETRIES: u32 = 3;
/// Linear backoff between those retries: 250ms, 500ms, 750ms.
const TITLE_UPDATE_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Clone)]
pub struct ClientBackend {
    client:                    Arc<::fabro_client::Client>,
    run_scope:                 Option<RunId>,
    workflow_version_packager: Option<Arc<dyn crate::WorkflowVersionPackager>>,
}

impl ClientBackend {
    #[must_use]
    pub fn new(client: Arc<::fabro_client::Client>) -> Self {
        Self {
            client,
            run_scope: None,
            workflow_version_packager: None,
        }
    }

    #[must_use]
    pub fn with_workflow_version_packager(
        mut self,
        packager: Arc<dyn crate::WorkflowVersionPackager>,
    ) -> Self {
        self.workflow_version_packager = Some(packager);
        self
    }

    /// Restrict this backend to a single run.
    ///
    /// Ask Fabro sessions use this with a same-run worker token so accidental
    /// cross-run tool calls are rejected before they reach the API.
    #[must_use]
    pub fn with_run_scope(mut self, run_id: RunId) -> Self {
        self.run_scope = Some(run_id);
        self
    }

    fn ensure_run_scope(&self, run_id: &RunId) -> anyhow::Result<()> {
        if let Some(scope) = self.run_scope {
            if &scope != run_id {
                anyhow::bail!("run {run_id} is outside this tool session's run scope");
            }
        }
        Ok(())
    }
}

#[async_trait]
impl FabroToolBackend for ClientBackend {
    /// Package the supplied tree, then register dependencies before parents.
    /// Versions are immutable and content-addressed, so a failed upload can be
    /// retried with the same contents without cleanup.
    async fn create_workflow_version(
        &self,
        source: crate::ValidatedWorkflowVersionCreate,
    ) -> anyhow::Result<fabro_types::WorkflowVersionId> {
        anyhow::ensure!(
            self.run_scope.is_none(),
            "workflow version creation is outside this tool session's run scope"
        );
        let packager = self
            .workflow_version_packager
            .as_ref()
            .ok_or_else(common::workflow_version_tool_unavailable_error)?;
        let packaged = packager.package(source).await?;
        let versions = packaged
            .versions()
            .map(|(_, v)| v.version())
            .collect::<Vec<_>>();
        self.client.register_workflow_versions(versions).await?;
        Ok(packaged.root_id())
    }

    async fn read_run_sandbox_files(
        &self,
        run_id: &RunId,
        directory: &str,
    ) -> anyhow::Result<std::collections::BTreeMap<String, String>> {
        self.ensure_run_scope(run_id)?;
        let collected = self
            .client
            .list_run_sandbox_files(run_id, directory)
            .await?;
        Ok(collected.files.into_iter().collect())
    }

    async fn list_run_files(
        &self,
        run_id: &RunId,
        path: &str,
        depth: u32,
    ) -> anyhow::Result<Vec<crate::RunFileEntry>> {
        self.ensure_run_scope(run_id)?;
        let listing = self
            .client
            .list_sandbox_tool_files(run_id, path, depth)
            .await?;
        Ok(listing
            .data
            .into_iter()
            .map(|entry| crate::RunFileEntry {
                name:   entry.name,
                is_dir: entry.is_dir,
                size:   entry.size,
            })
            .collect())
    }

    async fn read_run_file(
        &self,
        run_id: &RunId,
        path: &str,
    ) -> anyhow::Result<crate::RunFileContent> {
        self.ensure_run_scope(run_id)?;
        let content = self.client.get_sandbox_tool_file(run_id, path).await?;
        Ok(crate::RunFileContent {
            path:    content.path,
            content: content.content,
            bytes:   content.bytes,
        })
    }

    async fn create_run_from_intent(&self, intent: RunIntent) -> anyhow::Result<RunId> {
        anyhow::ensure!(
            self.run_scope.is_none(),
            "run creation is outside this tool session's run scope"
        );
        self.client.create_run_from_intent(intent).await
    }

    async fn resolve_run(&self, selector: &str) -> anyhow::Result<Run> {
        if self.run_scope.is_some() {
            let run_id: RunId = selector.parse().map_err(|err| {
                anyhow::anyhow!(
                    "run selector must be the owning run id for this tool session: {err}"
                )
            })?;
            self.ensure_run_scope(&run_id)?;
            return self.retrieve_run(&run_id).await;
        }
        self.client.resolve_run(selector).await
    }

    async fn retrieve_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        self.ensure_run_scope(run_id)?;
        self.client.retrieve_run(run_id).await
    }

    async fn set_run_title(&self, run: Option<&str>, title: &str) -> anyhow::Result<Run> {
        // `None` is the session's own run: the scope IS the target, so a
        // scoped session can always retitle itself and never needs a
        // selector. An explicit selector resolves first and the scope
        // guard then refuses a foreign run.
        let run_id = match run {
            Some(selector) => self.resolve_run(selector).await?.id,
            None => match self.run_scope {
                Some(run) => run,
                None => anyhow::bail!(
                    "run is required when this surface has no own run; pass the run id or \
                     workflow name"
                ),
            },
        };
        self.ensure_run_scope(&run_id)?;
        // The title door is quota-free and idempotent (the server
        // short-circuits an unchanged title), but its store reads and
        // the platform-record append run beside the run's own writes —
        // denkhaus/work#1545 watched three transient failures then a
        // clean success inside one planner stage, and every surfaced
        // failure counted toward the stage's error budget until the
        // pass sank. Absorb the transient tagged statuses here (429
        // and 5xx) so the agent's ONE tool call rides out the blip;
        // deterministic refusals (400/403/404) still fail immediately,
        // and the last error surfaces once the retries are spent.
        let mut attempt = 0;
        loop {
            match self.client.update_run_title(&run_id, title).await {
                Ok(run) => return Ok(run),
                Err(error) => {
                    let transient =
                        ::fabro_client::api_failure_for(&error).is_some_and(|failure| {
                            matches!(failure.status.as_u16(), 429 | 500 | 502 | 503 | 504)
                        });
                    if !transient || attempt >= TITLE_UPDATE_RETRIES {
                        return Err(error);
                    }
                    attempt += 1;
                    tracing::debug!(
                        attempt,
                        run_id = %run_id,
                        error = %error,
                        "run title update failed transiently; retrying"
                    );
                    sleep(TITLE_UPDATE_RETRY_BACKOFF.mul_f64(f64::from(attempt))).await;
                }
            }
        }
    }

    async fn mark_run_orchestration(
        &self,
        run: &str,
        field: fabro_types::OrchestrationField,
        verdict: fabro_types::OrchestrationVerdict,
    ) -> anyhow::Result<RunMarkOutcome> {
        // The target is FOREIGN by design (ADR-0039 stage D): the
        // adjudicator and the merger write verdicts about OTHER runs, so
        // this call deliberately skips the run-scope guard every
        // own-run tool carries — the server's workflow allowlist is the
        // gate, and the door refuses a self-mark anyway.
        let run_id = self.resolve_run(run).await?.id;
        let marked = self
            .client
            .mark_run_orchestration(&run_id, field, verdict)
            .await?;
        Ok(RunMarkOutcome {
            run_id:  marked.run_id,
            field:   marked.field,
            verdict: marked.verdict,
            by_run:  marked.by_run,
        })
    }

    async fn resolve_optional_own_run(&self, run: Option<&str>) -> anyhow::Result<RunId> {
        match run {
            Some(selector) => Ok(self.resolve_run(selector).await?.id),
            None => match self.run_scope {
                Some(run) => Ok(run),
                None => anyhow::bail!(
                    "run is required when this surface has no own run; pass the run id"
                ),
            },
        }
    }

    /// `fabro_run_queue` (ADR-0039 stage D part 2): the same queue the
    /// CLI facade reads — field, workflow filter, the settled-verdict
    /// filter (stage E), and the other-field requirement (work#1126)
    /// ride through; the server decorates each entry with its claim's
    /// work item and project.
    async fn run_queue_settled(
        &self,
        field: fabro_types::OrchestrationField,
        workflow: Option<&str>,
        verdict: Option<fabro_types::OrchestrationVerdict>,
        requires: Option<(
            fabro_types::OrchestrationField,
            fabro_types::OrchestrationVerdict,
        )>,
    ) -> anyhow::Result<Vec<types::RunQueueEntry>> {
        Ok(self
            .client
            .run_queue(field, workflow, verdict, requires)
            .await?
            .data)
    }

    /// `fabro_run_merge` (ADR-0039 stage E): the same merge the CLI
    /// facade drives — the technical integration act over the run's
    /// pull request, server-side GitHub credentials, the merger
    /// workflow allowlist enforced at the door.
    async fn merge_run(&self, run_id: &fabro_types::RunId) -> anyhow::Result<RunMergeResult> {
        let merged = self.client.run_merge(run_id).await?;
        Ok(RunMergeResult {
            run:          merged.run_id,
            merged:       merged.merged,
            pull_request: merged.pull_request,
        })
    }

    /// `fabro_run_push_resolution` (ADR-0047): the resolve push door —
    /// the same push the CLI facade drives, the server transferring the
    /// resolution objects itself and pushing the run's own branch with
    /// its own credentials.
    async fn push_run_resolution(
        &self,
        run_id: &fabro_types::RunId,
        commit: &str,
    ) -> anyhow::Result<RunPushResolutionResult> {
        let pushed = self.client.push_run_resolution(run_id, commit).await?;
        Ok(RunPushResolutionResult {
            run:        pushed.run_id,
            pushed:     pushed.pushed,
            run_branch: pushed.run_branch,
            head_sha:   pushed.head_sha,
        })
    }

    /// `fabro_observation_list` (ADR-0039 stage D part 2): a run's
    /// facts. The run-scope guard stays ON — a run-scoped session (Ask
    /// Fabro) reads its own run only, while the adjudicator's
    /// unscoped stage session verifies foreign packets.
    async fn list_run_observations(
        &self,
        run_id: &RunId,
    ) -> anyhow::Result<Vec<types::RunObservation>> {
        self.ensure_run_scope(run_id)?;
        Ok(self
            .client
            .list_run_observations(run_id)
            .await?
            .observations)
    }

    /// `fabro_run_diffs` (ADR-0051): the run's diff records — the
    /// `checkpoint` records (each stage's diff summary and patch blob)
    /// and the `run.diff` record — read over the platform-records
    /// endpoint, labelled through the checkpoint timeline the fork view
    /// already serves. The run-scope guard stays ON, matching every
    /// own-run read door.
    async fn list_run_diffs(&self, run_id: &RunId) -> anyhow::Result<Vec<crate::RunDiffEntry>> {
        use fabro_store::platform_records::PlatformRecordKind;

        use crate::run_diffs::run_diff_entries;

        self.ensure_run_scope(run_id)?;
        let checkpoints = self
            .client
            .list_petri_platform_records(run_id, Some(&PlatformRecordKind::Checkpoint.to_string()))
            .await?;
        let run_diffs = self
            .client
            .list_petri_platform_records(run_id, Some(&PlatformRecordKind::RunDiff.to_string()))
            .await?;
        let timeline = self.client.run_timeline(run_id).await?;
        Ok(run_diff_entries(checkpoints, run_diffs, &timeline))
    }

    /// `fabro_work_radius` (ADR-0038 read door): the closure radius the
    /// close gate enforces, read beside it.
    async fn work_item_radius(
        &self,
        project: &str,
        item_key: &str,
    ) -> anyhow::Result<types::WorkItemRadiusResponse> {
        self.client.work_item_radius(project, item_key).await
    }

    /// `fabro_work_comments` (ADR-0039 stage D part 2): the answer
    /// channel, in the forge's chronological order.
    async fn work_item_comments(
        &self,
        project: &str,
        item_key: &str,
    ) -> anyhow::Result<Vec<WorkItemComment>> {
        Ok(self
            .client
            .work_item_comments(project, item_key)
            .await?
            .comments)
    }

    /// `fabro_observation_record` (ADR-0038): the append endpoint stores
    /// the record at the run's next `seq`; the position rides both the
    /// request (the store's stage column) and the record payload (the
    /// fold's stage tie), the same shape a checkpoint worker writes.
    async fn record_observation(
        &self,
        run_id: &RunId,
        position: StagePosition,
        category: fabro_types::ObservationCategory,
        text: &str,
    ) -> anyhow::Result<u64> {
        use fabro_store::platform_records::{ObservationRecordedRecord, PlatformRecord};

        self.ensure_run_scope(run_id)?;
        let record = PlatformRecord::ObservationRecorded(ObservationRecordedRecord {
            category,
            text: text.to_string(),
            execution: position.execution,
            firing: position.firing,
        });
        let value = serde_json::to_value(&record).context("encoding the observation record")?;
        let serde_json::Value::Object(record) = value else {
            anyhow::bail!("the observation record did not encode as a JSON object");
        };
        let stored = self
            .client
            .append_petri_platform_record(run_id, types::PetriPlatformRecordAppendRequest {
                execution: Some(position.execution),
                firing: Some(position.firing),
                record,
            })
            .await?;
        Ok(stored.seq)
    }

    async fn work_next(
        &self,
        project: &str,
        run_id: &RunId,
        lane: &str,
        item_key: Option<&str>,
    ) -> anyhow::Result<types::WorkClaimNextResponse> {
        self.ensure_run_scope(run_id)?;
        match item_key {
            Some(item_key) => {
                let granted = self
                    .client
                    .grant_work_item(project, item_key, run_id, lane)
                    .await?;
                Ok(types::WorkClaimNextResponse {
                    outcome: match granted.outcome {
                        types::WorkClaimGrantOutcome::Granted => {
                            types::WorkClaimNextOutcome::Granted
                        }
                        types::WorkClaimGrantOutcome::AlreadyHeld => {
                            types::WorkClaimNextOutcome::AlreadyHeld
                        }
                        types::WorkClaimGrantOutcome::Conflict => {
                            types::WorkClaimNextOutcome::Empty
                        }
                    },
                    claim:   Some(granted.claim),
                    item:    granted.item,
                })
            }
            None => {
                self.client
                    .claim_next_work_item(project, run_id, lane)
                    .await
            }
        }
    }

    async fn work_update(
        &self,
        project: &str,
        item_key: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> anyhow::Result<types::WorkItem> {
        self.client
            .update_work_item(project, item_key, title, body)
            .await
    }

    async fn work_show(&self, project: &str, item_key: &str) -> anyhow::Result<types::WorkItem> {
        self.client.retrieve_work_item(project, item_key).await
    }

    async fn work_list(
        &self,
        project: &str,
        lane: Option<&str>,
        limit: usize,
        query: Option<&str>,
    ) -> anyhow::Result<Vec<types::WorkItem>> {
        let limit = u32::try_from(limit).unwrap_or(u32::MAX);
        // The run tool has no label/state/page surface yet (denkhaus/work#1001
        // wires the CLI; work#1476 wires the census); it lists the first
        // page, open items, unfiltered — the envelope's `data`.
        let response = self
            .client
            .list_work_items(project, lane, Some(limit), query, None, None, None, None)
            .await?;
        Ok(response.data)
    }

    async fn work_create(
        &self,
        project: &str,
        title: &str,
        body: &str,
        labels: Vec<String>,
        observations: Vec<fabro_types::ObservationRef>,
    ) -> anyhow::Result<types::WorkItem> {
        // `types::ObservationRef` IS the canonical type (with_replacement
        // in fabro-api's build.rs) — the refs pass through unchanged.
        self.client
            .create_work_item(project, title, body, labels, observations)
            .await
    }

    async fn observation_manage(
        &self,
        run_id: &RunId,
        seq: u64,
        action: &str,
        reason: &str,
        project: Option<&str>,
        item_key: Option<&str>,
    ) -> anyhow::Result<types::ObservationManageResponse> {
        self.client
            .manage_observation(&run_id.to_string(), seq, action, reason, project, item_key)
            .await
    }

    async fn observation_feed(
        &self,
        category: Option<&str>,
        triage: Option<&str>,
        run_id: Option<&str>,
        query: Option<&str>,
        sort: Option<&str>,
        limit: u32,
        page: u32,
    ) -> anyhow::Result<types::GlobalObservationList> {
        self.client
            .list_global_observations(category, triage, run_id, query, sort, limit, page)
            .await
    }

    /// The session archaeology doors (ADR-0046 phase 2, work#1679): no
    /// run-scope guard by design — the index spans sessions of all runs,
    /// and these arms only serve under the UNPINNED supervisor backend
    /// whose token carries `agent:supervisor`. A run-scoped analyst's
    /// token lacks the scope, so the server doors answer 403 even if a
    /// scoped surface ever wired the tools.
    async fn search_sessions(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> anyhow::Result<fabro_store::SessionSummaryPage> {
        let response = self.client.search_sessions(query, limit, offset).await?;
        Ok(fabro_store::SessionSummaryPage {
            data:     response.data,
            has_more: response.meta.has_more,
        })
    }

    async fn read_session_transcript(
        &self,
        session_id: &str,
        max_tokens: u64,
    ) -> anyhow::Result<fabro_types::SessionTranscript> {
        let session_id: fabro_types::SessionId = session_id.parse().map_err(|err| {
            anyhow::anyhow!("session_id must be a session id from fabro_session_search: {err}")
        })?;
        let response = self
            .client
            .read_session_transcript(session_id, max_tokens)
            .await?;
        Ok(response)
    }

    async fn work_close(
        &self,
        project: &str,
        item_key: &str,
        reason: &str,
    ) -> anyhow::Result<types::WorkItem> {
        // Agents never force: the closure-radius override (ADR-0038) is
        // the human surface; an open radius refuses the agent's close
        // with the listing, and the run routes it as feedback.
        self.client
            .close_work_item(project, item_key, reason, false)
            .await
            .map(|response| response.item)
    }

    async fn work_release(&self, claim_id: &str, reason: &str) -> anyhow::Result<types::WorkClaim> {
        self.client.release_work_claim(claim_id, reason).await
    }

    async fn start_run(&self, run_id: &RunId, resume: bool) -> anyhow::Result<Run> {
        self.ensure_run_scope(run_id)?;
        self.client.start_run(run_id, resume).await
    }

    async fn approve_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        self.ensure_run_scope(run_id)?;
        self.client.approve_run(run_id).await
    }

    async fn deny_run(&self, run_id: &RunId, reason: Option<String>) -> anyhow::Result<Run> {
        self.ensure_run_scope(run_id)?;
        self.client.deny_run(run_id, reason).await
    }

    async fn cancel_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        self.ensure_run_scope(run_id)?;
        self.client.cancel_run(run_id).await
    }

    async fn interrupt_run(&self, run_id: &RunId) -> anyhow::Result<()> {
        self.ensure_run_scope(run_id)?;
        self.client.interrupt_run(run_id, None, None).await?;
        Ok(())
    }

    async fn steer_run(&self, run_id: &RunId, text: String, interrupt: bool) -> anyhow::Result<()> {
        self.ensure_run_scope(run_id)?;
        self.client.steer_run(run_id, text, interrupt, None).await?;
        Ok(())
    }

    async fn archive_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        self.ensure_run_scope(run_id)?;
        self.client.archive_run(run_id).await
    }

    async fn unarchive_run(&self, run_id: &RunId) -> anyhow::Result<Run> {
        self.ensure_run_scope(run_id)?;
        self.client.unarchive_run(run_id).await
    }

    async fn list_store_runs(&self) -> anyhow::Result<Vec<Run>> {
        if let Some(run_id) = self.run_scope {
            return Ok(vec![self.retrieve_run(&run_id).await?]);
        }
        self.client.list_store_runs().await
    }

    async fn list_store_runs_by_parent(&self, parent_id: RunId) -> anyhow::Result<Vec<Run>> {
        self.ensure_run_scope(&parent_id)?;
        self.client.list_store_runs_by_parent(parent_id).await
    }

    async fn link_run_parent(&self, child_id: &RunId, parent_id: &RunId) -> anyhow::Result<Run> {
        self.ensure_run_scope(child_id)?;
        self.client.link_run_parent(child_id, parent_id).await
    }

    async fn unlink_run_parent(&self, child_id: &RunId) -> anyhow::Result<Run> {
        self.ensure_run_scope(child_id)?;
        self.client.unlink_run_parent(child_id).await
    }

    async fn get_run_state(&self, run_id: &RunId) -> anyhow::Result<RunProjection> {
        self.ensure_run_scope(run_id)?;
        self.client.get_run_state(run_id).await
    }

    async fn wait_run(
        &self,
        run_id: &RunId,
        until: crate::RunWaitUntil,
        timeout_ms: u64,
    ) -> anyhow::Result<types::RunWaitResult> {
        self.ensure_run_scope(run_id)?;
        let until = match until {
            crate::RunWaitUntil::Terminal => types::WaitRunUntil::Terminal,
            crate::RunWaitUntil::Merged => types::WaitRunUntil::Merged,
        };
        let timeout_ms = std::num::NonZeroU64::new(timeout_ms.max(1))
            .expect("timeout_ms is clamped to at least 1");
        self.client.wait_run(run_id, until, timeout_ms).await
    }

    async fn list_run_stream(
        &self,
        run_id: &RunId,
        after: u64,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<RunStreamItem>> {
        self.ensure_run_scope(run_id)?;
        match limit {
            Some(limit) => {
                self.client
                    .list_run_stream_until(run_id, after, limit)
                    .await
            }
            None => self.client.list_run_stream(run_id, after).await,
        }
    }

    async fn list_run_questions(&self, run_id: &RunId) -> anyhow::Result<Vec<types::ApiQuestion>> {
        self.ensure_run_scope(run_id)?;
        self.client.list_run_questions(run_id).await
    }

    async fn submit_run_answer(
        &self,
        run_id: &RunId,
        question_id: &str,
        body: types::SubmitAnswerRequest,
    ) -> anyhow::Result<()> {
        self.ensure_run_scope(run_id)?;
        self.client
            .submit_run_answer(run_id, question_id, body)
            .await
    }

    async fn get_run_pair_status(&self, run_id: &RunId) -> anyhow::Result<RunPairStatusResponse> {
        self.ensure_run_scope(run_id)?;
        self.client.get_run_pair_status(run_id).await
    }

    async fn start_run_pair(
        &self,
        run_id: &RunId,
        stage_id: StageId,
    ) -> anyhow::Result<PairRecord> {
        self.ensure_run_scope(run_id)?;
        self.client.start_run_pair(run_id, stage_id).await
    }

    async fn get_run_pair(&self, run_id: &RunId, pair_id: &PairId) -> anyhow::Result<PairRecord> {
        self.ensure_run_scope(run_id)?;
        self.client.get_run_pair(run_id, pair_id).await
    }

    async fn end_run_pair(&self, run_id: &RunId, pair_id: &PairId) -> anyhow::Result<PairRecord> {
        self.ensure_run_scope(run_id)?;
        self.client.end_run_pair(run_id, pair_id).await
    }

    async fn send_run_pair_message(
        &self,
        run_id: &RunId,
        pair_id: &PairId,
        request: PairMessageRequest,
    ) -> anyhow::Result<PairMessageRecord> {
        self.ensure_run_scope(run_id)?;
        self.client
            .send_run_pair_message(run_id, pair_id, request)
            .await
    }

    async fn get_run_pair_transcript(
        &self,
        run_id: &RunId,
        pair_id: &PairId,
        since_seq: Option<u32>,
        limit: Option<u32>,
    ) -> anyhow::Result<PairTranscriptResponse> {
        self.ensure_run_scope(run_id)?;
        self.client
            .get_run_pair_transcript(run_id, pair_id, since_seq, limit)
            .await
    }

    async fn read_run_blob(
        &self,
        run_id: &RunId,
        hash: &fabro_types::BlobHash,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        self.ensure_run_scope(run_id)?;
        let bytes = self.client.read_run_blob(run_id, hash).await?;
        Ok(bytes.map(|bytes| bytes.to_vec()))
    }

    async fn create_ask_session(&self, run_id: &RunId, title: &str) -> anyhow::Result<String> {
        self.ensure_run_scope(run_id)?;
        let session = self
            .client
            .create_run_session(*run_id, types::CreateRunSessionRequest {
                title:    Some(title.to_string()),
                model:    None,
                provider: None,
            })
            .await?;
        Ok(session.id.to_string())
    }

    async fn submit_ask_turn(
        &self,
        run_id: &RunId,
        session_id: &str,
        question: &str,
    ) -> anyhow::Result<crate::AskTurnOutcome> {
        self.ensure_run_scope(run_id)?;
        let session_id: SessionId = session_id
            .parse()
            .map_err(anyhow::Error::new)
            .with_context(|| format!("invalid Ask-Fabro session id {session_id}"))?;
        let mut stream = self
            .client
            .submit_session_turn_stream(session_id, question)
            .await?;

        let mut collector = crate::AskTurnCollector::default();
        while let Some(event) = stream.next_event().await? {
            collector.absorb(&event);
        }
        Ok(collector.finish())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use async_trait::async_trait;
    use fabro_types::{WorkflowVersion, WorkflowVersionId};
    use fabro_workflow_version::{CollectedWorkflowClosure, ValidatedWorkflowVersion};
    use serde_json::json;

    use super::*;
    use crate::{ValidatedWorkflowVersionCreate, WorkflowVersionPackager};

    struct FixedPackager(Vec<WorkflowVersion>);

    #[async_trait]
    impl WorkflowVersionPackager for FixedPackager {
        async fn package(
            &self,
            _: ValidatedWorkflowVersionCreate,
        ) -> anyhow::Result<CollectedWorkflowClosure> {
            let versions = self
                .0
                .iter()
                .map(|v| Ok((v.id()?, ValidatedWorkflowVersion::new(v.clone())?)))
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(CollectedWorkflowClosure::from_dependency_order(
                versions.last().unwrap().0,
                versions,
            ))
        }
    }

    fn version(
        entrypoint: &str,
        dependencies: BTreeMap<fabro_types::WorkflowPath, WorkflowVersionId>,
    ) -> WorkflowVersion {
        WorkflowVersion::new(
            entrypoint.parse().unwrap(),
            BTreeMap::from([(
                entrypoint.parse().unwrap(),
                format!(
                    "digraph {entrypoint} {{ {} }}",
                    dependencies
                        .keys()
                        .map(|p| format!("child [stack.child_workflow=\"{p}\"]"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            )]),
            dependencies,
        )
        .unwrap()
    }

    fn source() -> ValidatedWorkflowVersionCreate {
        ValidatedWorkflowVersionCreate {
            entrypoint: "root".parse().unwrap(),
            files:      BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn create_workflow_version_registers_packaged_closure_and_retries_after_failure() {
        let server = httpmock::MockServer::start_async().await;
        let child = version("child", BTreeMap::new());
        let child_id = child.id().unwrap();
        let root = version(
            "root",
            BTreeMap::from([("child".parse().unwrap(), child_id)]),
        );
        let root_id = root.id().unwrap();
        let child_upload = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/api/v1/workflow-versions")
                    .json_body_obj(&child);
                then.status(201)
                    .json_body(json!({"workflow_version_id": child_id}));
            })
            .await;
        let failed_root = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/api/v1/workflow-versions")
                    .json_body_obj(&root);
                then.status(400);
            })
            .await;
        let client = ::fabro_client::Client::new_no_proxy(&server.url("")).unwrap();
        let backend = ClientBackend::new(Arc::new(client))
            .with_workflow_version_packager(Arc::new(FixedPackager(vec![child, root.clone()])));

        assert!(backend.create_workflow_version(source()).await.is_err());
        child_upload.assert_calls_async(1).await;
        failed_root.assert_calls_async(1).await;

        // Immutable content: retrying re-sends the child and completes the root.
        failed_root.delete_async().await;
        let root_upload = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/api/v1/workflow-versions")
                    .json_body_obj(&root);
                then.status(201)
                    .json_body(json!({"workflow_version_id": root_id}));
            })
            .await;
        assert_eq!(
            backend.create_workflow_version(source()).await.unwrap(),
            root_id
        );
        child_upload.assert_calls_async(2).await;
        root_upload.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn create_workflow_version_without_packager_is_unavailable() {
        let client = ::fabro_client::Client::new_no_proxy("http://127.0.0.1:1").unwrap();
        let error = ClientBackend::new(Arc::new(client))
            .create_workflow_version(source())
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!(
                "{} is not available",
                crate::FABRO_WORKFLOW_VERSION_CREATE_TOOL_NAME
            )
        );
    }

    /// A `Run` fixture shaped like the server's decorated summary —
    /// enough JSON for the title door's response to decode.
    fn title_run(run_id: &fabro_types::RunId, title: &str) -> fabro_types::Run {
        use chrono::TimeZone;
        use fabro_types::test_support::test_principal;
        fabro_types::Run {
            id:                  *run_id,
            parent_id:           None,
            children_count:      0,
            claimed_seed_id:     None,
            title:               title.to_string(),
            goal:                "the workflow goal".to_string(),
            workflow:            fabro_types::WorkflowRef {
                slug:       Some("develop".to_string()),
                name:       Some("Develop".to_string()),
                graph_name: None,
                node_count: 0,
                edge_count: 0,
            },
            automation:          None,
            repository:          None,
            created_by:          test_principal(),
            origin:              fabro_types::RunOrigin::default(),
            labels:              std::collections::HashMap::new(),
            lifecycle:           fabro_types::RunLifecycle {
                conclusion_failure: None,
                status:             fabro_types::RunStatus::Pending {
                    reason: fabro_types::PendingReason::ApprovalRequired,
                },
                approval:           None,
                pending_control:    None,
                queue_position:     None,
                error:              None,
                archived:           false,
                archived_at:        None,
            },
            sandbox:             None,
            workflow_version_id: None,
            models:              Vec::new(),
            source_directory:    None,
            timestamps:          fabro_types::RunTimestamps {
                created_at:    chrono::Utc.with_ymd_and_hms(2026, 11, 21, 0, 0, 0).unwrap(),
                started_at:    None,
                last_event_at: None,
                completed_at:  None,
            },
            timing:              None,
            usage:               types::Usage::default(),
            size:                fabro_types::RunSize::default(),
            ask_fabro:           fabro_types::AskFabro::default(),
            diff:                None,
            pull_request:        None,
            current_question:    None,
            superseded_by:       None,
            retried_from:        None,
            links:               fabro_types::RunLinks { web: None },
        }
    }

    /// denkhaus/work#1545: the title door failed transiently three
    /// times then succeeded inside one planner stage, and every
    /// surfaced error counted toward the stage's budget until it sank.
    /// The backend now absorbs transient failures (429/5xx) inside one
    /// tool call: a persistent 500 is retried exactly
    /// `TITLE_UPDATE_RETRIES` extra times before surfacing, and a
    /// deterministic refusal (400) fails on the first attempt.
    #[tokio::test]
    async fn set_run_title_retries_transient_failures_but_not_deterministic_ones() {
        let server = httpmock::MockServer::start_async().await;
        let run_id = fabro_types::RunId::new();
        let path = format!("/api/v1/runs/{run_id}");
        let client = ::fabro_client::Client::new_no_proxy(&server.url("")).unwrap();
        let backend = ClientBackend::new(Arc::new(client)).with_run_scope(run_id);

        // A transient 500 (httpmock matches the earliest mock first, so
        // this one owns the endpoint until deleted): four attempts in
        // total — the original call plus three retries — then the last
        // error surfaces.
        let transient = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::PATCH).path(&path);
                then.status(500);
            })
            .await;
        let error = backend.set_run_title(None, "a meaningful run title").await;
        assert!(error.is_err(), "a persistent transient failure surfaces");
        transient.assert_calls_async(4).await;

        // Recovery: with the transient mock gone, the very next call
        // succeeds on its first attempt.
        transient.delete_async().await;
        let run = title_run(&run_id, "a meaningful run title");
        let success = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::PATCH).path(&path);
                then.status(200).json_body_obj(&run);
            })
            .await;
        let titled = backend
            .set_run_title(None, "a meaningful run title")
            .await
            .expect("the title lands once the blip is gone");
        assert_eq!(titled.title, "a meaningful run title");
        success.assert_calls_async(1).await;

        // A deterministic refusal (400: the server rejected the title
        // itself) never retries.
        success.delete_async().await;
        let refused = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::PATCH).path(&path);
                then.status(400).json_body(json!({
                    "errors": [{"detail": "run title exceeds the 100-character limit"}]
                }));
            })
            .await;
        assert!(
            backend
                .set_run_title(None, "a meaningful run title")
                .await
                .is_err(),
            "the deterministic refusal still fails"
        );
        refused.assert_calls_async(1).await;
    }
}
