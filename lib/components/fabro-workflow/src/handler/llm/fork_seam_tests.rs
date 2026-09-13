//! Fork-only seam tests (2026-09-13 Bestandsaufnahme): pin the contracts
//! between OUR workflow assets and the engine's run-tool surface in a file
//! upstream does not have, so no upstream merge can overwrite or conflict
//! with this coverage. Each test names the incident that motivated it.
//!
//! Today's breaks were all at this seam: #832 deleted the old dispatch and
//! silently dropped four arms (tools stayed advertised, calls died with
//! "unknown Fabro run tool"), the create contract changed shape under the
//! conductor prompts, and pebble's skill expansion started binding prose
//! slash tokens. Engine test suites stayed green throughout — only live
//! conductor passes found the damage.

use std::sync::Arc;

use fabro_types::RunId;

use crate::handler::llm::fabro_tools::execute_fabro_run_tool;
use crate::services::FabroRunToolServices;

/// Minimal backend: every method bails. The dispatch-coverage test only
/// asserts that a tool name RESOLVES to an arm — argument-validation and
/// backend errors are fine, "unknown Fabro run tool" is the death sentence.
#[derive(Clone, Copy)]
struct BailingBackend;

#[async_trait::async_trait]
impl fabro_tool::FabroToolBackend for BailingBackend {
    async fn create_workflow_version(
        &self,
        _source: fabro_tool::ValidatedWorkflowVersionCreate,
    ) -> anyhow::Result<fabro_types::WorkflowVersionId> {
        anyhow::bail!("mock backend: create_workflow_version not implemented")
    }

    async fn create_run_from_intent(
        &self,
        _intent: fabro_types::RunIntent,
    ) -> anyhow::Result<RunId> {
        anyhow::bail!("mock backend: create_run_from_intent not implemented")
    }

    async fn resolve_run(&self, _selector: &str) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: resolve_run not implemented")
    }

    async fn retrieve_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: retrieve_run not implemented")
    }

    async fn start_run(
        &self,
        _run_id: &RunId,
        _resume: bool,
    ) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: start_run not implemented")
    }

    async fn approve_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: approve_run not implemented")
    }

    async fn deny_run(
        &self,
        _run_id: &RunId,
        _reason: Option<String>,
    ) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: deny_run not implemented")
    }

    async fn cancel_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: cancel_run not implemented")
    }

    async fn interrupt_run(&self, _run_id: &RunId) -> anyhow::Result<()> {
        anyhow::bail!("mock backend: interrupt_run not implemented")
    }

    async fn steer_run(
        &self,
        _run_id: &RunId,
        _text: String,
        _interrupt: bool,
    ) -> anyhow::Result<()> {
        anyhow::bail!("mock backend: steer_run not implemented")
    }

    async fn archive_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: archive_run not implemented")
    }

    async fn unarchive_run(&self, _run_id: &RunId) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: unarchive_run not implemented")
    }

    async fn list_store_runs(&self) -> anyhow::Result<Vec<fabro_api::types::Run>> {
        anyhow::bail!("mock backend: list_store_runs not implemented")
    }

    async fn list_store_runs_by_parent(
        &self,
        _parent_id: RunId,
    ) -> anyhow::Result<Vec<fabro_api::types::Run>> {
        anyhow::bail!("mock backend: list_store_runs_by_parent not implemented")
    }

    async fn link_run_parent(
        &self,
        _child_id: &RunId,
        _parent_id: &RunId,
    ) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: link_run_parent not implemented")
    }

    async fn unlink_run_parent(&self, _child_id: &RunId) -> anyhow::Result<fabro_api::types::Run> {
        anyhow::bail!("mock backend: unlink_run_parent not implemented")
    }

    async fn get_run_state(&self, _run_id: &RunId) -> anyhow::Result<fabro_types::RunProjection> {
        anyhow::bail!("mock backend: get_run_state not implemented")
    }

    async fn list_run_events(
        &self,
        _run_id: &RunId,
        _after: Option<u32>,
        _limit: Option<usize>,
    ) -> anyhow::Result<Vec<fabro_types::EventEnvelope>> {
        anyhow::bail!("mock backend: list_run_events not implemented")
    }

    async fn list_run_events_until(
        &self,
        _run_id: &RunId,
        _after: Option<u32>,
        _limit: usize,
    ) -> anyhow::Result<Vec<fabro_types::EventEnvelope>> {
        anyhow::bail!("mock backend: list_run_events_until not implemented")
    }

    async fn run_pull_request_state(&self, _run_id: &RunId) -> anyhow::Result<Option<String>> {
        anyhow::bail!("mock backend: run_pull_request_state not implemented")
    }

    async fn create_ask_session(&self, _run_id: &RunId, _title: &str) -> anyhow::Result<String> {
        anyhow::bail!("mock backend: create_ask_session not implemented")
    }

    async fn submit_ask_turn(
        &self,
        _run_id: &RunId,
        _session_id: &str,
        _question: &str,
    ) -> anyhow::Result<fabro_tool::AskTurnOutcome> {
        anyhow::bail!("mock backend: submit_ask_turn not implemented")
    }

    async fn wait_run(
        &self,
        _run_id: &RunId,
        _until: fabro_tool::RunWaitUntil,
        _timeout_ms: u64,
    ) -> anyhow::Result<fabro_api::types::RunWaitResult> {
        anyhow::bail!("mock backend: wait_run not implemented")
    }

    async fn list_runs_of_workflow(
        &self,
        _workflow: &str,
        _created_since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<fabro_api::types::Run>> {
        anyhow::bail!("mock backend: list_runs_of_workflow not implemented")
    }

    async fn existing_sandbox_run_ids(
        &self,
    ) -> anyhow::Result<Option<std::collections::HashSet<String>>> {
        anyhow::bail!("mock backend: existing_sandbox_run_ids not implemented")
    }

    async fn list_run_questions(
        &self,
        _run_id: &RunId,
    ) -> anyhow::Result<Vec<fabro_api::types::ApiQuestion>> {
        anyhow::bail!("mock backend: list_run_questions not implemented")
    }

    async fn submit_run_answer(
        &self,
        _run_id: &RunId,
        _question_id: &str,
        _body: fabro_api::types::SubmitAnswerRequest,
    ) -> anyhow::Result<()> {
        anyhow::bail!("mock backend: submit_run_answer not implemented")
    }

    async fn get_run_logs(&self, _run_id: &RunId) -> anyhow::Result<Option<Vec<u8>>> {
        anyhow::bail!("mock backend: get_run_logs not implemented")
    }

    async fn get_run_pair_status(
        &self,
        _run_id: &RunId,
    ) -> anyhow::Result<fabro_types::RunPairStatusResponse> {
        anyhow::bail!("mock backend: get_run_pair_status not implemented")
    }

    async fn start_run_pair(
        &self,
        _run_id: &RunId,
        _stage_id: fabro_types::StageId,
    ) -> anyhow::Result<fabro_types::PairRecord> {
        anyhow::bail!("mock backend: start_run_pair not implemented")
    }

    async fn get_run_pair(
        &self,
        _run_id: &RunId,
        _pair_id: &fabro_types::PairId,
    ) -> anyhow::Result<fabro_types::PairRecord> {
        anyhow::bail!("mock backend: get_run_pair not implemented")
    }

    async fn end_run_pair(
        &self,
        _run_id: &RunId,
        _pair_id: &fabro_types::PairId,
    ) -> anyhow::Result<fabro_types::PairRecord> {
        anyhow::bail!("mock backend: end_run_pair not implemented")
    }

    async fn send_run_pair_message(
        &self,
        _run_id: &RunId,
        _pair_id: &fabro_types::PairId,
        _request: fabro_types::PairMessageRequest,
    ) -> anyhow::Result<fabro_types::PairMessageRecord> {
        anyhow::bail!("mock backend: send_run_pair_message not implemented")
    }

    async fn get_run_pair_transcript(
        &self,
        _run_id: &RunId,
        _pair_id: &fabro_types::PairId,
        _since_seq: Option<u32>,
        _limit: Option<u32>,
    ) -> anyhow::Result<fabro_types::PairTranscriptResponse> {
        anyhow::bail!("mock backend: get_run_pair_transcript not implemented")
    }
}

fn services() -> FabroRunToolServices {
    FabroRunToolServices {
        backend:        Arc::new(BailingBackend),
        current_run_id: RunId::new(),
        inspects:       vec![
            "develop".to_string(),
            "merge-upstream".to_string(),
            "revisor".to_string(),
        ],
        run_wide:       false,
    }
}

/// Incident run 01M2DH2P56GPRBRCTV3HN1GW8E (2026-09-13): the #832 merge
/// replaced the run-tool dispatch and lost the arms for fabro_run_wait,
/// fabro_run_logs, fabro_runs_list, and fabro_ask — the catalog kept
/// advertising all twelve, every call to the four fell through to
/// "unknown Fabro run tool". This test fails the moment ANY catalog tool
/// lacks a dispatch arm, whatever the next refactor renames or moves.
#[tokio::test]
async fn every_catalog_run_tool_has_a_dispatch_arm() {
    let services = services();
    for definition in fabro_tool::tool_definitions() {
        let outcome =
            execute_fabro_run_tool(definition.name, serde_json::json!({}), &services).await;
        if let Err(error) = outcome {
            let message = error.to_string();
            assert!(
                !message.contains("unknown Fabro run tool"),
                "`{}` is in the fabro_tool catalog but has no dispatch arm \
                 (advertised to agents, every call would die): {message}",
                definition.name
            );
        }
    }
}

/// The workflow assets' node-level `fabro_tools` allowlists must only name
/// catalog tools. An unknown name is silently ignored at registration
/// (register_named_fabro_run_tools), so a typo'd or renamed tool yields a
/// stage with the tool advertised by prompts but never callable.
#[test]
fn workflow_fabro_tools_allowlists_name_catalog_tools() {
    let catalog: Vec<&str> = fabro_tool::tool_definitions()
        .iter()
        .map(|definition| definition.name)
        .collect();
    for entry in workflow_fabro_files() {
        let (path, source) = entry;
        for (line_no, line) in source.lines().enumerate() {
            if let Some(rest) = line.split("fabro_tools=\"").nth(1) {
                let list = rest.split('"').next().unwrap_or("");
                for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                    assert!(
                        catalog.contains(&name),
                        "{path}:{} — fabro_tools names `{name}` which is not in the \
                         run-tool catalog (known: {:?})",
                        line_no + 1,
                        catalog
                    );
                }
            }
        }
    }
}

/// Incident fabro-68d3 (2026-09-13, conductor pass 01M2DFBRB3): pebble's
/// skill expansion binds `/name` tokens that follow whitespace and kills
/// the session structurally when the name is not a registered skill. Fork
/// prompts legitimately MENTION commands; until the engine softens the
/// rule, no workflow prompt may contain a whitespace-preceded slash token
/// outside a path (a path continues with `/`).
#[test]
fn workflow_prompts_carry_no_expandable_slash_tokens() {
    for entry in workflow_prompt_files() {
        let (path, source) = entry;
        for (line_no, line) in source.lines().enumerate() {
            for (index, _) in line.match_indices(' ') {
                let rest = &line[index + 1..];
                if let Some(token) = rest.strip_prefix('/') {
                    let name: String = token
                        .chars()
                        .take_while(|c| {
                            c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-'
                        })
                        .collect();
                    let expandable = !name.is_empty()
                        && token.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                        && !rest[name.len() + 1..].starts_with('/');
                    assert!(
                        !expandable,
                        "{path}:{} — expandable slash token `/{name}` in prompt text kills \
                         agent sessions via pebble skill expansion (fabro-68d3): {line}",
                        line_no + 1
                    );
                }
            }
        }
    }
}

/// Incident run 01M2DH2P56GP develop leg: the #832 run-intent contract
/// replaced inline {workflow, workflow_source} payloads with registered
/// workflow versions. The conductor legs' prompts must teach the two-step
/// dance and must not carry JSON-teaching occurrences of the rejected
/// shape (prose mentions inside REJECTED warnings are allowed).
#[test]
fn conductor_legs_teach_the_workflow_version_contract() {
    for leg in ["develop-leg.md", "revise-leg.md"] {
        let base = repo_root().join(".fabro/workflows/conductor/prompts");
        let source = std::fs::read_to_string(base.join(leg))
            .unwrap_or_else(|error| panic!("conductor prompt {leg} should exist: {error}"));
        assert!(
            source.contains("fabro_workflow_version_create"),
            "{leg} must teach the fabro_workflow_version_create registration step"
        );
        assert!(
            source.contains("workflow_version_id"),
            "{leg} must create runs from workflow_version_id"
        );
        assert!(
            !source.contains("{\"runs\": [{\"workflow\":"),
            "{leg} still teaches the pre-#832 inline create shape"
        );
    }
}

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repo root")
        .to_path_buf()
}

fn workflow_fabro_files() -> Vec<(String, String)> {
    let dir = repo_root().join(".fabro/workflows");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let fabro = entry.path().join("workflow.fabro");
            if let Ok(source) = std::fs::read_to_string(&fabro) {
                out.push((fabro.display().to_string(), source));
            }
        }
    }
    assert!(
        !out.is_empty(),
        "workflow.fabro files should exist under .fabro"
    );
    out
}

fn workflow_prompt_files() -> Vec<(String, String)> {
    let dir = repo_root().join(".fabro/workflows");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let prompts = entry.path().join("prompts");
            if let Ok(files) = std::fs::read_dir(&prompts) {
                for file in files.flatten() {
                    if file.path().extension().is_some_and(|e| e == "md") {
                        if let Ok(source) = std::fs::read_to_string(file.path()) {
                            out.push((file.path().display().to_string(), source));
                        }
                    }
                }
            }
        }
    }
    assert!(
        !out.is_empty(),
        "workflow prompts should exist under .fabro"
    );
    out
}
