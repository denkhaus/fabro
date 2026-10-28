use std::sync::Arc;

use anyhow::Context as _;
use async_trait::async_trait;
use fabro_api::types;
use fabro_types::{
    PairId, PairMessageRecord, PairMessageRequest, PairRecord, PairTranscriptResponse, Run, RunId,
    RunIntent, RunPairStatusResponse, RunProjection, RunStreamItem, SessionId, StageId,
};

use crate::{FabroToolBackend, common, seeds};

#[derive(Clone)]
pub struct ClientBackend {
    client:                    Arc<::fabro_client::Client>,
    run_scope:                 Option<RunId>,
    workflow_version_packager: Option<Arc<dyn crate::WorkflowVersionPackager>>,
    run_seeds:                 Option<Arc<dyn crate::RunSeedsSource>>,
}

impl ClientBackend {
    #[must_use]
    pub fn new(client: Arc<::fabro_client::Client>) -> Self {
        Self {
            client,
            run_scope: None,
            workflow_version_packager: None,
            run_seeds: None,
        }
    }

    /// Bind the tracker reads to the run context's own workspace
    /// (fabro-d6de rework): `fabro_seeds_search` serves from this source —
    /// the current tracker of the repo the session works on — instead of a
    /// server-side seeds listing.
    #[must_use]
    pub fn with_run_seeds(mut self, source: Arc<dyn crate::RunSeedsSource>) -> Self {
        self.run_seeds = Some(source);
        self
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
        self.client.update_run_title(&run_id, title).await
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
        // The run tool has no label/page surface yet (denkhaus/work#1001
        // wires the CLI); it lists the first page unfiltered.
        self.client
            .list_work_items(project, lane, Some(limit), query, None, None)
            .await
    }

    async fn work_create(
        &self,
        project: &str,
        title: &str,
        body: &str,
        labels: Vec<String>,
    ) -> anyhow::Result<types::WorkItem> {
        self.client
            .create_work_item(project, title, body, labels)
            .await
    }

    async fn work_close(
        &self,
        project: &str,
        item_key: &str,
        reason: &str,
    ) -> anyhow::Result<types::WorkItem> {
        self.client.close_work_item(project, item_key, reason).await
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

    async fn list_seed_rows(
        &self,
        status: Option<&str>,
        assignee: Option<&str>,
        label: Option<&str>,
    ) -> anyhow::Result<Vec<crate::SeedsRow>> {
        // The run context's own tracker (fabro-d6de rework, user decision
        // 2026-10-19): rows come from the bound RunSeedsSource and the
        // tracker-level filters apply locally. No server-side listing —
        // the tracker always follows the repo the session works on.
        let Some(source) = self.run_seeds.as_ref() else {
            return Err(anyhow::anyhow!(
                "fabro_seeds_search is not available: this session is not bound to a run \
                 workspace tracker"
            ));
        };
        let rows = source.rows().await?;
        let rows = seeds::apply_tracker_filters(rows, status, assignee, label);
        Ok(rows)
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
}
