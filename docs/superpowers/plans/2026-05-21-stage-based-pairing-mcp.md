# Stage-Based Pairing API And MCP Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Simplify run pairing into a stage-based public API and expose it through a new `fabro_run_pair` MCP tool without leaking agent session IDs.

**Architecture:** Public pairing targets workflow stages, identified by `StageId` (`node_id@visit`). The live agent session ID remains an internal runtime binding used by `SteeringHub` active-pair bookkeeping and live run projection, but it is not present in HTTP API schemas, public pair event bodies, generated clients, MCP tool parameters, or MCP tool results. The MCP server calls the same stage-based HTTP/client API and can return shared pair structs directly.

**Tech Stack:** Rust, Axum, OpenAPI/progenitor, `fabro-types`, `fabro-api`, `fabro-client`, `fabro-server`, `fabro-workflow`, `fabro-interview`, `fabro-mcp-server`, `apps/fabro-web`, `cargo nextest`, Bun OpenAPI client generation.

---

## Contract

Public pair API shape:

```rust
pub struct PairTarget {
    pub stage_id: StageId,
    pub node_label: String,
}

pub struct PairStartRequest {
    pub stage_id: StageId,
}
```

`GET /api/v1/runs/{id}/pair` returns:

```json
{
  "run_id": "run_...",
  "current_pair": null,
  "targets": [
    {
      "stage_id": "implement@1",
      "node_label": "Implement"
    }
  ]
}
```

`POST /api/v1/runs/{id}/pair` accepts:

```json
{
  "stage_id": "implement@1"
}
```

Public API responses must not contain:

- `agent_session_id`
- `session_id`
- `node_id`
- `visit`
- `provider`
- `model`

Public pair event bodies, generated client DTOs, and MCP pair tool params/results must follow the same rule. `StageId` already contains `node_id` and `visit`. `node_label` remains because it is display data and is not derivable from `StageId`.

## File Map

- `docs/internal/events-strategy.md` and `docs/internal/testing-strategy.md`: read before implementation because this changes event metadata and tests.
- `lib/crates/fabro-types/src/pair.rs`: public pair DTO definitions.
- `docs/public/api-reference/fabro-api.yaml`: OpenAPI source of truth for HTTP pair API.
- `lib/crates/fabro-api/build.rs`: replacement mappings for shared pair types.
- `lib/crates/fabro-api/tests/pair_round_trip.rs`: JSON/type parity for pair DTOs.
- `lib/crates/fabro-api/tests/run_event_round_trip.rs`: event JSON expectations after pair target changes.
- `lib/crates/fabro-workflow/src/event/events.rs`: workflow event enum with stage-only public pair lifecycle data.
- `lib/crates/fabro-workflow/src/event/convert.rs`: convert workflow events into public run event bodies without pair session data.
- `lib/crates/fabro-workflow/src/steering_hub.rs`: internal pair lifecycle and live session binding.
- `lib/crates/fabro-interview/src/control_protocol.rs`: worker control protocol fixtures and pair start payload.
- `lib/crates/fabro-cli/src/commands/run/runner.rs`: worker control dispatch into `SteeringHub`.
- `lib/crates/fabro-server/src/server.rs`: live run projection and pair transport.
- `lib/crates/fabro-server/src/server/handler/pair.rs`: HTTP handlers and transcript reconstruction.
- `lib/crates/fabro-server/src/server/tests.rs`: pair transport and live projection tests.
- `lib/crates/fabro-client/src/client.rs`: client convenience methods.
- `lib/crates/fabro-mcp-server/src/run_tools/pair.rs`: new MCP pair tool implementation.
- `lib/crates/fabro-mcp-server/src/run_tools.rs`: export MCP pair tool types/functions.
- `lib/crates/fabro-mcp-server/src/server.rs`: register `fabro_run_pair`.
- `lib/packages/fabro-api-client`: regenerate TypeScript client after OpenAPI changes.
- `apps/fabro-web`: update any generated-client pair consumers and run frontend checks.

## Task 1: Read Strategy Docs

**Files:**
- Read: `docs/internal/events-strategy.md`
- Read: `docs/internal/testing-strategy.md`

- [ ] **Step 1: Read event strategy**

Run:

```bash
sed -n '1,220p' docs/internal/events-strategy.md
```

Expected: notes on when to add or modify event variants, stored fields, and progress JSONL behavior.

- [ ] **Step 2: Read testing strategy**

Run:

```bash
sed -n '1,220p' docs/internal/testing-strategy.md
```

Expected: guidance for unit vs integration tests, snapshots, and fixture placement.

## Task 2: Simplify Public Pair DTOs

**Files:**
- Modify: `lib/crates/fabro-types/src/pair.rs`
- Modify: `lib/crates/fabro-api/tests/pair_round_trip.rs`

- [ ] **Step 1: Write failing pair DTO tests**

Update `lib/crates/fabro-api/tests/pair_round_trip.rs` so the pair target JSON fixture only contains `stage_id` and `node_label`.

Use this fixture shape in the existing pair type parity test:

```rust
let target_json = json!({
    "stage_id": "code@1",
    "node_label": "Code"
});
```

Assert that the public target does not serialize removed fields:

```rust
let target = PairTarget {
    stage_id: "code@1".parse().unwrap(),
    node_label: "Code".to_string(),
};
let serialized = serde_json::to_value(&target).unwrap();
assert_eq!(
    serialized,
    json!({
        "stage_id": "code@1",
        "node_label": "Code"
    })
);
assert!(serialized.get("agent_session_id").is_none());
assert!(serialized.get("session_id").is_none());
assert!(serialized.get("node_id").is_none());
assert!(serialized.get("visit").is_none());
assert!(serialized.get("provider").is_none());
assert!(serialized.get("model").is_none());
```

Run:

```bash
cargo nextest run -p fabro-api pair_round_trip
```

Expected: failure because `PairTarget` still has the old fields and `PairStartRequest` still expects a selector.

- [ ] **Step 2: Update public types**

Change `lib/crates/fabro-types/src/pair.rs` to this public shape:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairTarget {
    pub stage_id:   StageId,
    pub node_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairStartRequest {
    pub stage_id: StageId,
}
```

Remove `PairTargetSelector` from the public exports if no internal caller needs it after later tasks. If a temporary compile bridge is needed during this task, leave it private to the module until the final cleanup task.

Change `PairMessageRecord` from selector-based target data to stage-based data:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairMessageRecord {
    pub message_id:        PairMessageId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_message_id: Option<String>,
    pub pair_id:           PairId,
    pub run_id:            RunId,
    pub stage_id:          StageId,
    pub text:              String,
    pub accepted_at:       DateTime<Utc>,
}
```

Keep transcript entries using `PairTarget` so transcript rows remain self-describing:

```rust
pub target: PairTarget
```

Remove model/provider data from pair transcript assistant messages:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairTranscriptAssistantMessage {
    pub seq:             u32,
    pub event_id:        String,
    pub ts:              DateTime<Utc>,
    pub pair_id:         PairId,
    pub target:          PairTarget,
    pub text:            String,
    pub tool_call_count: usize,
}
```

Delete `PairTranscriptModel` if no remaining public type uses it.

- [ ] **Step 3: Run pair type test**

Run:

```bash
cargo nextest run -p fabro-api pair_round_trip
```

Expected: compile failures in API/OpenAPI replacement code and callers that still reference removed fields. Those are addressed in the next tasks.

## Task 3: Update OpenAPI And Generated API Replacements

**Files:**
- Modify: `docs/public/api-reference/fabro-api.yaml`
- Modify: `lib/crates/fabro-api/build.rs`
- Modify: `lib/crates/fabro-api/tests/pair_round_trip.rs`
- Modify: `lib/crates/fabro-api/tests/run_event_round_trip.rs`

- [ ] **Step 1: Update OpenAPI schemas**

In `docs/public/api-reference/fabro-api.yaml`, change the schemas near the existing pair definitions to this shape:

```yaml
    PairTarget:
      type: object
      additionalProperties: false
      required:
        - stage_id
        - node_label
      properties:
        stage_id:
          $ref: "#/components/schemas/StageId"
        node_label:
          type: string

    PairStartRequest:
      type: object
      additionalProperties: false
      required:
        - stage_id
      properties:
        stage_id:
          $ref: "#/components/schemas/StageId"
```

Remove the `PairTargetSelector` schema and every `$ref` to it.

Change `PairMessageRecord` in OpenAPI so it uses `stage_id`:

```yaml
        stage_id:
          $ref: "#/components/schemas/StageId"
```

and remove the old `target` selector field from that record.

Change `PairTranscriptAssistantMessage` so it does not expose model/provider information:

```yaml
    PairTranscriptAssistantMessage:
      type: object
      additionalProperties: false
      required:
        - kind
        - seq
        - event_id
        - ts
        - pair_id
        - target
        - text
        - tool_call_count
      properties:
        kind:
          type: string
          enum: [assistant_message]
        seq:
          type: integer
          format: uint32
        event_id:
          type: string
        ts:
          type: string
          format: date-time
        pair_id:
          $ref: "#/components/schemas/PairId"
        target:
          $ref: "#/components/schemas/PairTarget"
        text:
          type: string
        tool_call_count:
          type: integer
          minimum: 0
```

Remove the `PairTranscriptModel` schema and any `$ref` to it.

- [ ] **Step 2: Update `fabro-api` replacements**

In `lib/crates/fabro-api/build.rs`, remove the replacement for `PairTargetSelector`:

```rust
("PairTargetSelector", "fabro_types::PairTargetSelector", &[]),
```

Keep the replacements for the shared public types that still exist:

```rust
("PairTarget", "fabro_types::PairTarget", &[]),
("PairRecord", "fabro_types::PairRecord", &[]),
("PairStartRequest", "fabro_types::PairStartRequest", &[]),
("PairMessageRequest", "fabro_types::PairMessageRequest", &[]),
("PairMessageRecord", "fabro_types::PairMessageRecord", &[]),
("PairTranscriptResponse", "fabro_types::PairTranscriptResponse", &[]),
```

- [ ] **Step 3: Update event round-trip fixtures**

In `lib/crates/fabro-api/tests/run_event_round_trip.rs`, update `run.pair.started` expected JSON so the `target` object contains only:

```json
{
  "stage_id": "code@1",
  "node_label": "Code"
}
```

Add an explicit negative assertion on the serialized public event body:

```rust
let serialized = serde_json::to_value(&event).unwrap();
let body = &serialized["body"];
assert!(body.to_string().contains("stage_id"));
assert!(!body.to_string().contains("agent_session_id"));
assert!(!body.to_string().contains("session_id"));
assert!(!body.to_string().contains("provider"));
assert!(!body.to_string().contains("model"));
assert!(!body.to_string().contains("\"node_id\""));
assert!(!body.to_string().contains("\"visit\""));
```

This assertion is scoped to the public pair event body. General agent events may still expose `session_id` through the existing events API.

- [ ] **Step 4: Regenerate/build Rust API**

Run:

```bash
cargo build -p fabro-api
```

Expected: generated API compiles or reports remaining references to the old `PairTargetSelector`/target fields.

- [ ] **Step 5: Run API tests**

Run:

```bash
cargo nextest run -p fabro-api pair_round_trip run_event_round_trip
```

Expected: pass after all pair OpenAPI fixtures and replacements match the simplified public structs.

## Task 4: Keep Pair Events Stage-Only

**Files:**
- Modify: `lib/crates/fabro-workflow/src/event/events.rs`
- Modify: `lib/crates/fabro-workflow/src/event/convert.rs`

- [ ] **Step 1: Keep `RunPairStarted` free of session identifiers**

In `lib/crates/fabro-workflow/src/event/events.rs`, keep `Event::RunPairStarted` stage-only:

```rust
RunPairStarted {
    pair_id: PairId,
    target:  PairTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor:   Option<Principal>,
},
```

Do not add `session_id` or `agent_session_id` to this event. The active session binding stays in `SteeringHub::ActivePair` while the run is live.

- [ ] **Step 2: Keep public run event body session-free**

In `lib/crates/fabro-workflow/src/event/convert.rs`, keep conversion to `fabro_types::RunPairStartedProps` limited to:

```rust
Event::RunPairStarted {
    pair_id, target, ..
} => EventBody::RunPairStarted(fabro_types::RunPairStartedProps {
    pair_id: *pair_id,
    target:  target.clone(),
}),
```

- [ ] **Step 3: Add event leak assertions**

In the workflow event conversion tests, serialize a `RunPairStarted` event and assert the public body contains only stage/display pair target data:

```rust
let value = serde_json::to_value(converted_event).unwrap();
let body = &value["body"];
assert!(body.to_string().contains("\"stage_id\""));
assert!(body.to_string().contains("\"node_label\""));
assert!(!body.to_string().contains("agent_session_id"));
assert!(!body.to_string().contains("session_id"));
assert!(!body.to_string().contains("\"node_id\""));
assert!(!body.to_string().contains("\"visit\""));
assert!(!body.to_string().contains("provider"));
assert!(!body.to_string().contains("model"));
```

- [ ] **Step 4: Update `RunPairStarted` tracing/log output**

In `lib/crates/fabro-workflow/src/event/events.rs`, update the `Event::log` / tracing arm for `RunPairStarted`. Replace any logging field that reads from `target.agent_session_id` with stage-only context:

```rust
Self::RunPairStarted {
    pair_id, target, ..
} => {
    info!(
        %pair_id,
        stage_id = %target.stage_id,
        node_label = %target.node_label,
        "Run pairing started",
    );
}
```

Do not log `session_id`, `agent_session_id`, provider, or model from the pair target.

- [ ] **Step 5: Run workflow event tests**

Run:

```bash
cargo nextest run -p fabro-workflow event
```

Expected: pass after public pair events use the simplified `PairTarget` and no pair event body exposes session or agent internals.

## Task 5: Refactor `SteeringHub` To Resolve Sessions Internally

**Files:**
- Modify: `lib/crates/fabro-workflow/src/steering_hub.rs`

- [ ] **Step 1: Change active pair bookkeeping**

Change the private active pair struct to store the resolved live session id separately from the public record:

```rust
#[derive(Debug, Clone)]
struct ActivePair {
    record:     PairRecord,
    session_id: String,
}
```

- [ ] **Step 2: Update `start_pair`**

Change `start_pair` so it accepts a public `PairTarget` and resolves the current active session by `target.stage_id`:

```rust
pub fn start_pair(
    &self,
    run_id: RunId,
    pair_id: PairId,
    target: PairTarget,
    actor: Option<Principal>,
) -> Result<PairRecord, PairControlError> {
    let active = self.active.read().expect("active lock poisoned");
    let Some(entry) = active.get(&target.stage_id) else {
        return Err(PairControlError::TargetNotActive);
    };
    let Some(pair_handle) = entry.pair_handle.as_ref() else {
        return Err(PairControlError::TargetNotActive);
    };
    let session_id = entry.session_id.clone();
    let interrupt_handle = Arc::clone(&entry.handle);
    let pair_handle = pair_handle.clone();
    drop(active);

    let mut active_pair = self.active_pair.lock().expect("active pair lock poisoned");
    if active_pair.is_some() {
        return Err(PairControlError::AlreadyPaired);
    }

    let text = human_joined_text();
    if !pair_handle.try_enqueue_bounded(
        SteeringItem::System { text: text.to_string() },
        PER_SESSION_QUEUE_CAP,
    ) {
        return Err(PairControlError::MessageNotAccepted);
    }

    let record = PairRecord {
        pair_id,
        run_id,
        status: PairStatus::Active,
        started_at: Utc::now(),
        ended_at: None,
        failure_reason: None,
        target,
    };

    self.emitter.emit(&Event::RunPairStarted {
        pair_id,
        target: record.target.clone(),
        actor: actor.clone(),
    });

    interrupt_handle.interrupt(actor);
    self.emitter.emit(&Event::AgentPairSystemMessage {
        node_id: record.target.stage_id.node_id().to_string(),
        visit: record.target.stage_id.visit(),
        session_id: session_id.clone(),
        pair_id,
        kind: PairSystemMessageKind::HumanJoined,
        text: text.to_string(),
    });

    *active_pair = Some(ActivePair {
        record: record.clone(),
        session_id,
    });
    Ok(record)
}
```

Adjust imports if `Arc` is not already in scope.

- [ ] **Step 3: Update `send_pair_message`**

Use `pair.session_id` and `pair.record.target.stage_id` instead of `pair.record.target.agent_session_id`:

```rust
let target = &pair.record.target;
let session_id = pair.session_id.clone();
let active = self.active.read().expect("active lock poisoned");
let Some(entry) = active.get(&target.stage_id) else {
    return Err(PairControlError::TargetNotActive);
};
if entry.session_id != session_id {
    return Err(PairControlError::TargetNotActive);
}
let Some(pair_handle) = entry.pair_handle.as_ref() else {
    return Err(PairControlError::TargetNotActive);
};
```

Emit `AgentPairUserMessage` with derived node/visit:

```rust
self.emitter.emit(&Event::AgentPairUserMessage {
    node_id: target.stage_id.node_id().to_string(),
    visit: target.stage_id.visit(),
    session_id,
    pair_id,
    message_id,
    client_message_id: client_message_id.clone(),
    text: text.clone(),
    actor,
});
```

Return stage-based message record:

```rust
Ok(PairMessageRecord {
    message_id,
    client_message_id,
    pair_id,
    run_id: pair.record.run_id,
    stage_id: target.stage_id.clone(),
    text,
    accepted_at: Utc::now(),
})
```

- [ ] **Step 4: Update `end_pair`**

Use `pair.session_id` when checking the current active entry and emitting `AgentPairSystemMessage`.

The `AgentPairSystemMessage` node data should derive from `target.stage_id`:

```rust
node_id: target.stage_id.node_id().to_string(),
visit: target.stage_id.visit(),
session_id: session_id.clone(),
```

- [ ] **Step 5: Update session-ended cleanup**

Change `pair_is_active_for` and `end_active_pair_for_target` so they compare:

```rust
pair.record.target.stage_id == *stage_id && pair.session_id == session_id
```

Do not compare against public target fields that no longer exist.

- [ ] **Step 6: Update steering hub tests**

In the `#[cfg(test)]` helpers at the bottom of `steering_hub.rs`, change `pair_target` to construct:

```rust
PairTarget {
    stage_id: stage_id.clone(),
    node_label: stage_id.node_id().to_string(),
}
```

Update assertions that previously expected `target.agent_session_id`.

- [ ] **Step 7: Run steering hub tests**

Run:

```bash
cargo nextest run -p fabro-workflow steering_hub
```

Expected: pass after all public target field references are removed.

## Task 6: Update Worker Control Protocol

**Files:**
- Modify: `lib/crates/fabro-interview/src/control_protocol.rs`
- Modify: `lib/crates/fabro-cli/src/commands/run/runner.rs`

- [ ] **Step 1: Keep `PairStart` target public**

`WorkerControlMessage::PairStart` can continue to carry `PairTarget`, but that `PairTarget` is now public stage data:

```rust
#[serde(rename = "pair.start")]
PairStart {
    run_id:  RunId,
    pair_id: PairId,
    target:  PairTarget,
    actor:   Principal,
},
```

The worker does not receive `agent_session_id`; `SteeringHub::start_pair` resolves the active session for `target.stage_id`.

- [ ] **Step 2: Update control protocol tests**

In `control_protocol.rs`, update the pair start fixture to:

```rust
PairTarget {
    stage_id: "code@1".parse().unwrap(),
    node_label: "Code".to_string(),
}
```

Assert the serialized pair target does not contain public pair leaks. Scope this assertion to the `target` object, not the whole worker-control envelope, because envelopes/actors may legitimately grow unrelated metadata later:

```rust
let json = serde_json::to_string(&envelope).unwrap();
let value: serde_json::Value = serde_json::from_str(&json).unwrap();
let target = &value["target"];
let target_text = target.to_string();
assert!(target_text.contains("stage_id"));
assert!(target_text.contains("node_label"));
assert!(!target_text.contains("agent_session_id"));
assert!(!target_text.contains("session_id"));
assert!(!target_text.contains("\"node_id\""));
assert!(!target_text.contains("\"visit\""));
assert!(!target_text.contains("provider"));
assert!(!target_text.contains("model"));
```

- [ ] **Step 3: Confirm runner dispatch still delegates to hub**

In `lib/crates/fabro-cli/src/commands/run/runner.rs`, keep the `PairStart` arm as:

```rust
WorkerControlMessage::PairStart {
    run_id,
    pair_id,
    target,
    actor,
} => {
    let _ = steering_hub.start_pair(run_id, pair_id, target, Some(actor));
}
```

- [ ] **Step 4: Run interview protocol tests**

Run:

```bash
cargo nextest run -p fabro-interview control_protocol
```

Expected: pass with the simplified pair start JSON.

## Task 7: Update Server Live Projection And Pair Handlers

**Files:**
- Modify: `lib/crates/fabro-server/src/server.rs`
- Modify: `lib/crates/fabro-server/src/server/handler/pair.rs`
- Modify: `lib/crates/fabro-server/src/server/tests.rs`

- [ ] **Step 1: Update active API target projection**

In `lib/crates/fabro-server/src/server.rs`, when handling `AgentSessionActivated`, store public pair targets:

```rust
managed_run
    .active_api_targets
    .insert(stage_id.clone(), PairTarget {
        stage_id: stage_id.clone(),
        node_label: event
            .node_label
            .clone()
            .unwrap_or_else(|| stage_id.node_id().to_string()),
    });
```

Keep `active_steerable_stages: HashMap<StageId, String>` as the session lease guard.

- [ ] **Step 2: Update deactivation cleanup**

In the `AgentSessionDeactivated` branch, remove `active_api_targets` only when `active_steerable_stages` still points at the deactivating session:

```rust
if managed_run
    .active_steerable_stages
    .get(stage_id)
    .is_some_and(|current| current == session_id)
{
    managed_run.active_steerable_stages.remove(stage_id);
    managed_run.active_api_targets.remove(stage_id);
}
```

This preserves the stale-deactivation protection previously provided by `target.agent_session_id`.

- [ ] **Step 3: Change pair target lookup to stage id**

In `lib/crates/fabro-server/src/server/handler/pair.rs`, change `pair_target_and_transport` to accept `&StageId`:

```rust
fn pair_target_and_transport(
    state: &AppState,
    id: &RunId,
    stage_id: &StageId,
) -> Result<(PairTarget, Option<super::super::RunAnswerTransport>), Response> {
    let runs = state.runs.lock().expect("runs lock poisoned");
    let Some(run) = runs.get(id) else {
        return Err(ApiError::not_found("Run not found.").into_response());
    };
    reject_unpairable_status(run.status)?;
    let Some(target) = run.active_api_targets.get(stage_id) else {
        return Err(pair_conflict(
            "Requested pair target is not active.",
            "pair_target_not_active",
        ));
    };
    Ok((target.clone(), run.answer_transport.clone()))
}
```

- [ ] **Step 4: Change start pair handler request**

In the `start_pair` HTTP handler, replace:

```rust
Json(req): Json<PairStartRequest>,
```

usage of `req.target` with:

```rust
let (target, transport) = match pair_target_and_transport(state.as_ref(), &id, &req.stage_id) {
    Ok(result) => result,
    Err(response) => return response,
};
```

The response remains `PairRecord`.

- [ ] **Step 5: Keep reconstructed pair windows stage-only**

Keep the private pair window struct free of session identifiers:

```rust
struct PairWindow {
    record:    PairRecord,
    start_seq: u32,
    end_seq:   Option<u32>,
}
```

In `reconstruct_pair_windows`, reconstruct from `RunPairStarted`, `RunPairEnded`, and `RunPairFailed` only. Do not read or store `event.session_id` for pair lifecycle windows.

- [ ] **Step 6: Update transcript matching**

Change transcript matching to use the pair window sequence range plus `stage_id`. Do not add old-history fallback behavior for previous `agent_session_id`-based pair records; this is a greenfield API surface and fixtures should be updated to the new model.

Use this logic for assistant/tool/error/warning events:

```rust
fn event_matches_pair_target(pair: &PairRecord, event: &fabro_types::RunEvent) -> bool {
    event.stage_id.as_ref() == Some(&pair.target.stage_id)
}
```

`transcript_page` already restricts scanned events to `window.start_seq..=window.end_seq`, so `stage_id` is enough for assistant/tool/error/warning events. Pair-specific user/system entries still match by `pair_id`.

When constructing `PairTranscriptAssistantMessage`, do not copy `props.model` into the pair transcript response. Keep only `text` and `tool_call_count` from the assistant event.

- [ ] **Step 7: Add transcript stage/window regression test**

In `lib/crates/fabro-server/src/server/handler/pair.rs` tests, add a regression test for the new stage/window transcript matching. The fixture should store events in this order:

```text
seq 1: agent.message for stage code@1 before pair start
seq 2: run.pair.started for pair_id pair_1 target code@1
seq 3: agent.message for stage code@1 inside pair window
seq 4: agent.tool.started for stage code@1 inside pair window
seq 5: agent.tool.completed for stage code@1 inside pair window
seq 6: run.pair.ended for pair_id pair_1
seq 7: agent.message for stage code@1 after pair end
```

Call the transcript projection for `pair_1` and assert:

```rust
let assistant_count = response
    .data
    .iter()
    .filter(|entry| matches!(entry, PairTranscriptEntry::AssistantMessage(_)))
    .count();
let tool_count = response
    .data
    .iter()
    .filter(|entry| matches!(entry, PairTranscriptEntry::ToolCall(_)))
    .count();
assert_eq!(assistant_count, 1);
assert_eq!(tool_count, 2);
```

Also assert the transcript text contains the inside-window assistant message and does not contain the before-start or after-end assistant messages. Do not add session-id filtering to make this pass; the intended behavior is stage plus pair window.

- [ ] **Step 8: Update server tests**

In `lib/crates/fabro-server/src/server/tests.rs` and `handler/pair.rs` test modules, replace old target fixtures with:

```rust
PairTarget {
    stage_id: "code@1".parse().unwrap(),
    node_label: "Code".to_string(),
}
```

Add the same negative assertion on every public pair HTTP response covered by tests: status, start, get, message acknowledgement, and transcript:

```rust
assert!(!body.to_string().contains("agent_session_id"));
assert!(!body.to_string().contains("session_id"));
assert!(!body.to_string().contains("provider"));
assert!(!body.to_string().contains("model"));
assert!(!body.to_string().contains("\"node_id\""));
assert!(!body.to_string().contains("\"visit\""));
```

- [ ] **Step 9: Run server pair tests**

Run:

```bash
cargo nextest run -p fabro-server pair
```

Expected: pass with stage-only pair request/response JSON.

## Task 8: Update Fabro Client

**Files:**
- Modify: `lib/crates/fabro-client/src/client.rs`

- [ ] **Step 1: Change start pair method**

Change the client method from selector-based:

```rust
pub async fn start_run_pair(
    &self,
    run_id: &RunId,
    target: PairTargetSelector,
) -> Result<PairRecord>
```

to stage-based:

```rust
pub async fn start_run_pair(
    &self,
    run_id: &RunId,
    stage_id: StageId,
) -> Result<PairRecord> {
    let body = PairStartRequest { stage_id };
    let response = self
        .send_api(|client| {
            let body = body.clone();
            async move {
                client
                    .start_run_pair()
                    .id(run_id.to_string())
                    .body(body)
                    .send()
                    .await
            }
        })
        .await?;
    convert_type(response.into_inner())
}
```

Add `StageId` to the imports from `fabro_types`.

- [ ] **Step 2: Remove selector imports**

Remove `PairTargetSelector` imports from `fabro-client` if no longer used.

- [ ] **Step 3: Run client build**

Run:

```bash
cargo build -p fabro-client
```

Expected: pass after all client callers use `StageId`.

## Task 9: Add `fabro_run_pair` MCP Tool

**Files:**
- Create: `lib/crates/fabro-mcp-server/src/run_tools/pair.rs`
- Modify: `lib/crates/fabro-mcp-server/src/run_tools.rs`
- Modify: `lib/crates/fabro-mcp-server/src/server.rs`

- [ ] **Step 1: Create pair tool action and params**

Create `lib/crates/fabro-mcp-server/src/run_tools/pair.rs` with:

```rust
use std::sync::Arc;

use fabro_client::Client;
use fabro_types::{PairId, PairMessageRequest, StageId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::common::{ToolError, ToolResult};

const MAX_PAIR_MESSAGE_BYTES: usize = 8192;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunPairAction {
    Status,
    Start,
    Get,
    Message,
    End,
    Transcript,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct FabroRunPairParams {
    pub(crate) action:            RunPairAction,
    pub(crate) run_id:            Option<String>,
    pub(crate) pair_id:           Option<String>,
    pub(crate) stage_id:          Option<String>,
    pub(crate) text:              Option<String>,
    pub(crate) client_message_id: Option<String>,
    pub(crate) since_seq:         Option<u32>,
    pub(crate) limit:             Option<u32>,
}
```

`run_id` is intentionally `Option<String>` so missing input reaches `TryFrom` and returns the tool-level `run_id is required` error instead of a serde deserialization error. The validator still requires it for every action.

- [ ] **Step 2: Add validated action types**

Add:

```rust
#[derive(Debug)]
pub(crate) struct ValidatedPairRun {
    pub(crate) run_id: String,
    pub(crate) action: ValidatedPairAction,
}

#[derive(Debug)]
pub(crate) enum ValidatedPairAction {
    Status,
    Start { stage_id: StageId },
    Get { pair_id: PairId },
    Message {
        pair_id:           PairId,
        text:              String,
        client_message_id: Option<String>,
    },
    End { pair_id: PairId },
    Transcript {
        pair_id:   PairId,
        since_seq: Option<u32>,
        limit:     Option<u32>,
    },
}
```

Implement `TryFrom<FabroRunPairParams> for ValidatedPairRun` with these exact validation errors:

```text
run_id is required
stage_id is required for action start
pair_id is required for action get
pair_id is required for action message
pair_id is required for action end
pair_id is required for action transcript
text is required for action message
text must be at most 8192 bytes for action message
invalid stage_id for action start: <parse error>
invalid pair_id for action <action>: <parse error>
```

Make `run_id` optional in `FabroRunPairParams` so missing input reaches `TryFrom` instead of failing serde deserialization before the tool can return the planned error:

```rust
let Some(run_id) = params
    .run_id
    .as_deref()
    .map(str::trim)
    .filter(|run_id| !run_id.is_empty())
else {
    return Err(ToolError::message("run_id is required"));
};
let run_id = run_id.to_string();
```

Parse `stage_id` with `raw.parse::<StageId>()`. Parse `pair_id` with `raw.parse::<PairId>()`.

- [ ] **Step 3: Add execution function**

Add:

```rust
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct PairRunResult {
    pub(crate) run_id: String,
    pub(crate) action: RunPairAction,
    pub(crate) result: Value,
}

pub(crate) async fn pair_run(
    client: Arc<Client>,
    params: ValidatedPairRun,
) -> ToolResult<PairRunResult> {
    let run_id = client
        .resolve_run(&params.run_id)
        .await
        .map_err(|err| ToolError::from_anyhow(&err))?
        .id;

    let (action, result) = match params.action {
        ValidatedPairAction::Status => (
            RunPairAction::Status,
            json!(client.get_run_pair_status(&run_id).await.map_err(|err| ToolError::from_anyhow(&err))?),
        ),
        ValidatedPairAction::Start { stage_id } => (
            RunPairAction::Start,
            json!(client.start_run_pair(&run_id, stage_id).await.map_err(|err| ToolError::from_anyhow(&err))?),
        ),
        ValidatedPairAction::Get { pair_id } => (
            RunPairAction::Get,
            json!(client.get_run_pair(&run_id, &pair_id).await.map_err(|err| ToolError::from_anyhow(&err))?),
        ),
        ValidatedPairAction::Message {
            pair_id,
            text,
            client_message_id,
        } => (
            RunPairAction::Message,
            json!(client
                .send_run_pair_message(
                    &run_id,
                    &pair_id,
                    PairMessageRequest {
                        text,
                        client_message_id,
                    },
                )
                .await
                .map_err(|err| ToolError::from_anyhow(&err))?),
        ),
        ValidatedPairAction::End { pair_id } => (
            RunPairAction::End,
            json!(client.end_run_pair(&run_id, &pair_id).await.map_err(|err| ToolError::from_anyhow(&err))?),
        ),
        ValidatedPairAction::Transcript {
            pair_id,
            since_seq,
            limit,
        } => (
            RunPairAction::Transcript,
            json!(client
                .get_run_pair_transcript(&run_id, &pair_id, since_seq, limit)
                .await
                .map_err(|err| ToolError::from_anyhow(&err))?),
        ),
    };

    Ok(PairRunResult {
        run_id: run_id.to_string(),
        action,
        result,
    })
}
```

If rustfmt expands these match arms, accept the formatted output.

- [ ] **Step 4: Add text summary**

Add:

```rust
pub(crate) fn pair_run_text(result: &PairRunResult) -> String {
    match result.action {
        RunPairAction::Status => format!("read pair status for Fabro run {}", result.run_id),
        RunPairAction::Start => format!("started pair for Fabro run {}", result.run_id),
        RunPairAction::Get => format!("read pair for Fabro run {}", result.run_id),
        RunPairAction::Message => format!("sent pair message for Fabro run {}", result.run_id),
        RunPairAction::End => format!("ended pair for Fabro run {}", result.run_id),
        RunPairAction::Transcript => {
            format!("read pair transcript for Fabro run {}", result.run_id)
        }
    }
}
```

- [ ] **Step 5: Add validation tests**

In `pair.rs`, add unit tests for:

```rust
#[test]
fn missing_or_blank_run_id_returns_tool_error() { ... }

#[test]
fn start_requires_stage_id() { ... }

#[test]
fn message_requires_pair_id_and_text() { ... }

#[test]
fn message_rejects_overlong_text() { ... }

#[test]
fn transcript_requires_pair_id() { ... }
```

Use `ValidatedPairRun::try_from(...)` and assert the exact error text contains the messages listed in Step 2.

- [ ] **Step 6: Export pair tool functions**

In `lib/crates/fabro-mcp-server/src/run_tools.rs`, add:

```rust
mod pair;
```

and:

```rust
pub(crate) use pair::{
    FabroRunPairParams, ValidatedPairRun, pair_run, pair_run_text,
};
```

- [ ] **Step 7: Register MCP tool**

In `lib/crates/fabro-mcp-server/src/server.rs`, add:

```rust
#[tool(
    name = "fabro_run_pair",
    description = "Inspect, start, message, end, or read transcript for a live Fabro run pairing session."
)]
async fn fabro_run_pair(
    &self,
    params: Parameters<run_tools::FabroRunPairParams>,
) -> Result<CallToolResult, ErrorData> {
    let params = match run_tools::ValidatedPairRun::try_from(params.0) {
        Ok(params) => params,
        Err(err) => return Ok(run_tools::error_result(err)),
    };
    let client = match self.client().await {
        Ok(client) => client,
        Err(err) => return Ok(run_tools::error_result(err)),
    };
    match run_tools::pair_run(client, params).await {
        Ok(result) => run_tools::success_result(&result, run_tools::pair_run_text(&result)),
        Err(err) => Ok(run_tools::error_result(err)),
    }
}
```

- [ ] **Step 8: Add MCP registration and schema tests**

In `lib/crates/fabro-mcp-server/src/server.rs`, add a `#[cfg(test)]` module that constructs `FabroMcpServer` and inspects `tool_router.list_all()`:

```rust
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use serde_json::Value;

    use super::*;
    use crate::FabroMcpServerSettings;

    #[test]
    fn fabro_run_pair_tool_is_registered_with_stage_based_schema() {
        let settings = FabroMcpServerSettings {
            cwd: PathBuf::from("."),
            config_path: PathBuf::from("fabro.toml"),
            client_factory: Arc::new(|| {
                Box::pin(async { panic!("client should not be constructed while listing tools") })
            }),
        };
        let server = FabroMcpServer::new(Arc::new(settings));
        let tools = server.tool_router.list_all();
        let tool = tools
            .iter()
            .find(|tool| tool.name.as_ref() == "fabro_run_pair")
            .expect("fabro_run_pair should be registered");
        let schema = Value::Object(tool.input_schema.as_ref().clone());
        let schema_text = schema.to_string();

        assert!(schema_text.contains("stage_id"));
        assert!(!schema_text.contains("agent_session_id"));
        assert!(!schema_text.contains("session_id"));
        assert!(!schema_text.contains("PairTargetSelector"));
        assert!(!schema_text.contains("\"target\""));
        assert!(!schema_text.contains("provider"));
        assert!(!schema_text.contains("model"));
        assert!(!schema_text.contains("\"node_id\""));
        assert!(!schema_text.contains("\"visit\""));
    }
}
```

- [ ] **Step 9: Add MCP result leakage tests**

In `lib/crates/fabro-mcp-server/src/run_tools/pair.rs`, add unit tests that serialize representative `PairRunResult` values for `status`, `start`, `message`, and `transcript` and assert no public result includes forbidden fields:

```rust
fn assert_no_public_pair_leaks(value: &serde_json::Value) {
    let text = value.to_string();
    assert!(!text.contains("agent_session_id"));
    assert!(!text.contains("session_id"));
    assert!(!text.contains("provider"));
    assert!(!text.contains("model"));
    assert!(!text.contains("\"node_id\""));
    assert!(!text.contains("\"visit\""));
}
```

Use public pair fixture values that contain `PairTarget { stage_id, node_label }` only.

- [ ] **Step 10: Run MCP server tests**

Run:

```bash
cargo nextest run -p fabro-mcp-server
```

Expected: pass after validation tests and tool registration compile.

## Task 10: Regenerate TypeScript API Client

**Files:**
- Modify generated files under: `lib/packages/fabro-api-client`

- [ ] **Step 1: Generate client**

Run:

```bash
cd lib/packages/fabro-api-client && bun run generate
```

Expected: TypeScript client updates pair DTOs to stage-based shape.

- [ ] **Step 2: Inspect generated diff**

Run:

```bash
git diff -- lib/packages/fabro-api-client | sed -n '1,240p'
```

Expected: pair schemas remove `agent_session_id`, `session_id`, `node_id`, `visit`, `provider`, and `model`; `PairStartRequest` gains `stage_id`.

- [ ] **Step 3: Verify generated pair DTOs do not leak internals**

Run:

```bash
rg -n "PairTargetSelector|agent_session_id|session_id|['\"]provider['\"]|[[:space:]]provider:|['\"]model['\"]|[[:space:]]model:|['\"]node_id['\"]|[[:space:]]node_id:|['\"]visit['\"]|[[:space:]]visit:" \
  lib/packages/fabro-api-client/src/models/pair-*.ts \
  lib/packages/fabro-api-client/src/models/run-pair-status-response.ts \
  lib/packages/fabro-api-client/src/api/human-in-the-loop-api.ts
```

Expected: no matches in generated pair DTOs or human-in-the-loop pair method signatures. If `rg` reports a generated pair file that should have been deleted, remove the stale generated file through the generator output or the normal generated-client cleanup path.

## Task 11: Update Frontend Consumers

**Files:**
- Search and modify as needed under: `apps/fabro-web`

- [ ] **Step 1: Search frontend pair consumers**

Run:

```bash
rg -n "startRunPair|getRunPairStatus|getRunPairTranscript|sendRunPairMessage|PairStartRequest|PairTargetSelector|agent_session_id|session_id|['\"]provider['\"]|[[:space:]]provider:|['\"]model['\"]|[[:space:]]model:|['\"]node_id['\"]|[[:space:]]node_id:|['\"]visit['\"]|[[:space:]]visit:" apps/fabro-web
```

Expected: review every match manually. Update real pair API consumers to use `stage_id` and the simplified generated DTOs. Ignore unrelated uses where the match is not part of the pair API, such as generic agent event rendering or text containing the word "pair".

- [ ] **Step 2: Update frontend pair request construction**

If `apps/fabro-web` constructs a pair start request, change it from:

```ts
await api.startRunPair(runId, {
  target: {
    stage_id: target.stage_id,
    agent_session_id: target.agent_session_id,
  },
});
```

to:

```ts
await api.startRunPair(runId, {
  stage_id: target.stage_id,
});
```

If there are no frontend pair API consumers, record that in the implementation notes and leave frontend source unchanged.

- [ ] **Step 3: Run frontend typecheck**

Run:

```bash
cd apps/fabro-web && bun run typecheck
```

Expected: pass.

- [ ] **Step 4: Run frontend tests**

Run:

```bash
cd apps/fabro-web && bun test
```

Expected: pass.

## Task 12: Final Cleanup And Verification

**Files:**
- Search all Rust/OpenAPI/TS files for removed public fields.

- [ ] **Step 1: Check public surfaces for leaked pair internals**

Run:

```bash
rg -n "PairTargetSelector|agent_session_id|session_id|['\"]provider['\"]|[[:space:]]provider:|['\"]model['\"]|[[:space:]]model:|['\"]node_id['\"]|[[:space:]]node_id:|['\"]visit['\"]|[[:space:]]visit:" \
  docs/public/api-reference/fabro-api.yaml \
  lib/crates/fabro-types/src/pair.rs \
  lib/crates/fabro-api/tests/pair_round_trip.rs \
  lib/crates/fabro-api/tests/run_event_round_trip.rs \
  lib/packages/fabro-api-client/src/models/pair-*.ts \
  lib/packages/fabro-api-client/src/models/run-pair-status-response.ts \
  lib/packages/fabro-api-client/src/api/human-in-the-loop-api.ts \
  lib/crates/fabro-mcp-server/src/run_tools/pair.rs \
  lib/crates/fabro-mcp-server/src/server.rs
```

Expected: no matches that expose those names through pair API schemas, pair DTOs, generated pair DTOs, or MCP pair params/results. Matches inside negative leakage assertions such as `assert!(!text.contains("session_id"))` are expected and should be reviewed, not deleted.

- [ ] **Step 2: Check internal runtime session usage separately**

Run:

```bash
rg -n "agent_session_id|session_id" \
  lib/crates/fabro-workflow/src/steering_hub.rs \
  lib/crates/fabro-workflow/src/handler/llm \
  lib/crates/fabro-server/src/server.rs \
  lib/crates/fabro-server/src/server/handler/pair.rs \
  lib/crates/fabro-interview/src/control_protocol.rs \
  lib/crates/fabro-cli/src/commands/run/runner.rs
```

Expected: internal `session_id` usage remains where it protects live session leases, emits ordinary agent events, or routes active pair messages. `agent_session_id` should not remain unless it belongs to unrelated legacy tests that were not part of the pair API and have been consciously reviewed.

- [ ] **Step 3: Run caller migration search**

Run:

```bash
rg -n "start_run_pair|PairStartRequest|PairTargetSelector|agent_session_id|\\.target" \
  lib apps docs/public/api-reference/fabro-api.yaml
```

Expected: review every match manually. Valid remaining matches include `PairRecord.target`, transcript entry `target`, public negative leakage assertions, and internal non-pair session handling. Invalid matches include selector-based `start_run_pair` calls, `PairStartRequest { target: ... }`, or public `agent_session_id` exposure.

- [ ] **Step 4: Run focused test suite**

Run:

```bash
cargo nextest run -p fabro-api pair_round_trip run_event_round_trip
cargo nextest run -p fabro-workflow steering_hub
cargo nextest run -p fabro-interview control_protocol
cargo nextest run -p fabro-server pair
cargo nextest run -p fabro-mcp-server
(cd apps/fabro-web && bun run typecheck)
(cd apps/fabro-web && bun test)
```

Expected: all pass.

- [ ] **Step 5: Run workspace build**

Run:

```bash
cargo build --workspace
```

Expected: pass.

- [ ] **Step 6: Run formatting check**

Run:

```bash
cargo +nightly-2026-04-14 fmt --check --all
```

Expected: pass. If it fails, run `cargo +nightly-2026-04-14 fmt --all` and repeat the check.

- [ ] **Step 7: Run clippy**

Run:

```bash
cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings
```

Expected: pass.

- [ ] **Step 8: Run workspace tests**

Run:

```bash
cargo nextest run --workspace
```

Expected: pass. If macOS reports `Too many open files`, rerun with:

```bash
ulimit -n 4096 && cargo nextest run --workspace
```

## Acceptance Criteria

- `POST /api/v1/runs/{id}/pair` accepts only `stage_id` for target selection.
- `GET /api/v1/runs/{id}/pair` returns targets with only `stage_id` and `node_label`.
- Pair records, message acknowledgements, transcript entries, public pair event bodies, generated pair DTOs, and MCP pair params/results do not expose `agent_session_id`, `session_id`, `provider`, `model`, raw `node_id`, or raw `visit`.
- Runtime still uses session IDs internally to avoid stale session cleanup and route active pair messages, but that state does not cross the public pair API or MCP boundary.
- `fabro_run_pair` is registered with actions `status`, `start`, `get`, `message`, `end`, and `transcript`.
- `fabro_run_pair` input schema contains `stage_id` and does not contain selector/session fields.
- MCP callers can start pairing with `run_id + stage_id`.
- Generated Rust and TypeScript API clients match the simplified OpenAPI contract.
- Frontend pair consumers, if any, use the simplified generated DTOs.
- Focused pair tests, MCP tests, frontend typecheck/tests, workspace build, fmt, clippy, and workspace tests pass.
