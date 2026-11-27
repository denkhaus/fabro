//! Presence pins for the Ask Fabro supervisor core (ADR-0046,
//! denkhaus/work#1451): the capability-stateful tool policy, the widened
//! supervisor registry, and the supervisor prompt. Fork-only: these pins
//! prove the feature survives an upstream merge that rewrites
//! `handler/sessions.rs`.

use fabro_types::{AskFabroCapability, ToolCategory, ToolSource, ToolSummary};
use pebble_coding_agent::extensions::EnvContext;
use pebble_coding_agent::tools::{ToolPermission, ToolPermissionPolicy};

use super::sessions::{
    ASK_FABRO_RUN_TOOL_NAMES, ASK_FABRO_SUPERVISOR_RUN_TOOL_NAMES, AskFabroToolPolicy,
    ask_fabro_allows_tool, build_ask_fabro_supervisor_global_system_prompt,
    build_ask_fabro_supervisor_system_prompt, build_ask_fabro_system_prompt,
};

fn descriptor(name: &str) -> pebble_agent::ToolDescriptor {
    pebble_agent::ToolDescriptor::new(
        pebble_agent::ToolId::try_new(name).expect("tool id"),
        lithos_llm::types::ToolDefinition::function(
            name.to_string(),
            format!("{name} test tool"),
            serde_json::json!({"type": "object"}),
        ),
    )
}

fn test_environment() -> EnvContext {
    EnvContext {
        working_directory: "/workspace".to_string(),
        ..EnvContext::default()
    }
}

fn supervisor_test_tools() -> Vec<ToolSummary> {
    let tool_summary = |name: &str| ToolSummary {
        name:        name.to_string(),
        description: "supervisor test tool".to_string(),
        source:      ToolSource::Native,
        category:    ToolCategory::Other,
        invoked:     false,
    };
    let mut tools: Vec<ToolSummary> = ASK_FABRO_SUPERVISOR_RUN_TOOL_NAMES
        .iter()
        .map(|name| tool_summary(name))
        .collect();
    tools.push(tool_summary("shell"));
    tools
}

/// The whole supervisor registry is grantable: every name resolves to a
/// catalog definition (the silent-loss guard) and the full-access policy
/// allows it.
#[test]
fn supervisor_registry_grants_the_full_run_tool_catalog_at_full_access() {
    let definitions = fabro_tool::tool_definitions();
    for name in ASK_FABRO_SUPERVISOR_RUN_TOOL_NAMES {
        assert!(
            definitions
                .iter()
                .any(|definition| definition.name == *name),
            "supervisor tool {name} is not in the fabro tool catalog"
        );
        assert!(
            ask_fabro_allows_tool(name, AskFabroCapability::FullAccess),
            "full access should allow {name}"
        );
    }
}

/// ADR-0016 keeps BOTH wait tools; the supervisor registry must carry the
/// fan-in sweep (`fabro_run_gather`, work#1688) beside the single-run
/// long-poll (`fabro_run_wait`), while the read-only posture stays
/// without either wait tool (its run tools are the two read doors).
#[test]
fn supervisor_registry_carries_run_gather_beside_run_wait_and_read_only_stays_without_it() {
    let gather = fabro_tool::FABRO_RUN_GATHER_TOOL_NAME;
    assert!(
        ASK_FABRO_SUPERVISOR_RUN_TOOL_NAMES.contains(&gather),
        "ADR-0016 keeps both wait tools: the supervisor registry must carry fabro_run_gather"
    );
    assert!(
        ask_fabro_allows_tool(gather, AskFabroCapability::FullAccess),
        "full access should allow {gather}"
    );
    assert!(
        !ask_fabro_allows_tool(gather, AskFabroCapability::ReadOnly),
        "the read-only posture must stay without the gather wait tool"
    );
}

/// The red-first pin (work#1451 acceptance): a read-only turn calling
/// `fabro_run_mark` is denied AND the denial names the audited posture;
/// a full-access turn is allowed the same call.
#[test]
fn run_mark_is_denied_at_read_only_with_the_audited_posture_and_allowed_at_full_access() {
    let scope =
        pebble_coding_agent::SessionScope::root(pebble_coding_agent::SessionId::new("ses_test"));
    let run_mark = descriptor(fabro_tool::FABRO_RUN_MARK_TOOL_NAME);

    let read_only_policy = AskFabroToolPolicy {
        capability: AskFabroCapability::ReadOnly,
    };
    match read_only_policy.permission(&scope, &run_mark) {
        ToolPermission::Deny { reason } => {
            assert!(
                reason.contains("Ask Fabro is read-only"),
                "the denial must name the audited posture: {reason}"
            );
        }
        other => panic!("read-only run_mark should be denied, got {other:?}"),
    }

    let full_access_policy = AskFabroToolPolicy {
        capability: AskFabroCapability::FullAccess,
    };
    assert_eq!(
        full_access_policy.permission(&scope, &run_mark),
        ToolPermission::Allow
    );

    // The supervisor does not inherit the sandbox: shell stays denied in
    // both postures, and the full-access denial names its own posture.
    match full_access_policy.permission(&scope, &descriptor("shell")) {
        ToolPermission::Deny { reason } => {
            assert!(
                reason.contains("Ask Fabro supervisor"),
                "the supervisor denial must name its posture: {reason}"
            );
        }
        other => panic!("full-access shell should be denied, got {other:?}"),
    }
}

/// The supervisor prompt (ADR-0046 scope 4) replaces the analyst contract
/// in full access and MANDATES the in-turn soft confirm before
/// `fabro_run_merge`; the read-only prompt stays the analyst contract,
/// byte-for-byte the pre-supervisor shape.
#[test]
fn supervisor_prompt_mandates_the_run_merge_soft_confirm() {
    let environment = test_environment();
    let tools = supervisor_test_tools();

    let supervisor = build_ask_fabro_supervisor_system_prompt(&environment, &tools);
    assert!(
        supervisor.contains("supervisor mode"),
        "the supervisor persona must be present"
    );
    assert!(
        supervisor.contains("fabro_run_merge"),
        "the merge mandate must name the tool"
    );
    assert!(
        supervisor.contains("MANDATORY soft confirm"),
        "the soft-confirm mandate must be present"
    );
    assert!(
        supervisor.contains(&format!("`{}`", fabro_tool::FABRO_RUN_MERGE_TOOL_NAME)),
        "the tool guidance must list the merge tool at full access"
    );

    let analyst = build_ask_fabro_system_prompt(&environment, &tools);
    assert!(
        analyst.contains("read-only, run-scoped analyst"),
        "the read-only posture keeps the analyst contract"
    );
    assert!(
        !analyst.contains("MANDATORY soft confirm"),
        "the read-only prompt carries no supervisor mandate"
    );
    assert!(
        !analyst.contains(&format!("`{}`", fabro_tool::FABRO_RUN_MERGE_TOOL_NAME)),
        "the read-only tool guidance must not list supervisor tools"
    );
}

/// The two supervisor templates' AUDIENCE contracts (work#1702, the
/// sprint-102 retro's K2 net): the audience mapping lives in
/// `AskFabroPrompt::transform`, far from the template files — these
/// pins fail loudly when an anchor sentence drifts across templates or
/// a template loses its audience contract. The runless GLOBAL template
/// teaches the explicit selector; the run-scoped template keeps the
/// anchor default.
#[test]
fn supervisor_template_audience_contracts_hold() {
    let environment = test_environment();
    let tools = supervisor_test_tools();

    let run_scoped = build_ask_fabro_supervisor_system_prompt(&environment, &tools);
    assert!(
        run_scoped.contains("default anchor"),
        "the run-scoped supervisor keeps the anchor default: {run_scoped}"
    );
    assert!(
        !run_scoped.contains("no ambient run anchor"),
        "the anchor-less teaching must not drift into the run-scoped template"
    );

    let global = build_ask_fabro_supervisor_global_system_prompt(&environment, &tools);
    assert!(
        global.contains("NO run anchor"),
        "the global supervisor names its anchor-less posture: {global}"
    );
    assert!(
        global.contains("name the run id explicitly"),
        "the global supervisor teaches the explicit run selector: {global}"
    );
}

/// The session archaeology tools (ADR-0046 phase 2, denkhaus/work#1679)
/// ride the supervisor registry ONLY: the supervisor set resolves both
/// names and grants them at full access, while the read-only set — and
/// the read-only posture itself — never does.
#[test]
fn session_archaeology_tools_are_supervisor_registry_only() {
    let definitions = fabro_tool::tool_definitions();
    for name in [
        fabro_tool::FABRO_SESSION_SEARCH_TOOL_NAME,
        fabro_tool::FABRO_SESSION_READ_TOOL_NAME,
    ] {
        assert!(
            ASK_FABRO_SUPERVISOR_RUN_TOOL_NAMES.contains(&name),
            "{name} belongs in the supervisor registry"
        );
        assert!(
            !ASK_FABRO_RUN_TOOL_NAMES.contains(&name),
            "{name} must never ride the read-only registry"
        );
        assert!(
            definitions.iter().any(|definition| definition.name == name),
            "{name} must resolve to a catalog definition"
        );
        assert!(
            !ask_fabro_allows_tool(name, AskFabroCapability::ReadOnly),
            "the read-only posture hides {name}"
        );
        assert!(
            ask_fabro_allows_tool(name, AskFabroCapability::FullAccess),
            "full access grants {name}"
        );
    }
}

/// The policy shape a read-only session meets when it somehow calls a
/// session archaeology tool: denied, with the audited posture named.
#[test]
fn session_read_is_denied_at_read_only_with_the_audited_posture() {
    let scope =
        pebble_coding_agent::SessionScope::root(pebble_coding_agent::SessionId::new("ses_test"));
    let session_read = descriptor(fabro_tool::FABRO_SESSION_READ_TOOL_NAME);

    let read_only_policy = AskFabroToolPolicy {
        capability: AskFabroCapability::ReadOnly,
    };
    match read_only_policy.permission(&scope, &session_read) {
        ToolPermission::Deny { reason } => {
            assert!(
                reason.contains("Ask Fabro is read-only"),
                "the denial must name the audited posture: {reason}"
            );
        }
        other => panic!("read-only fabro_session_read should be denied, got {other:?}"),
    }
}
