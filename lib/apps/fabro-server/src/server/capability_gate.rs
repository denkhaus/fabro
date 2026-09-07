//! Capability gate for agent surfaces (ADR-0019, user decision 2026-09-08).
//!
//! Point 6 — the sandbox stays credential-free: agent-authored workflow
//! configuration must never be able to mint credentials for itself. The
//! line's agents write `.fabro/workflows/**` via PRs; a
//! `[run.integrations.github]` permissions block authored by them would be
//! self-escalation (the minted token lands in every agent shell call, and
//! agents hold exfiltration channels such as curl and web_fetch).
//!
//! Enforcement lives at the server's run-start seam: only runs created by a
//! human `User` principal may resolve a GitHub integration. Every other
//! principal (Worker, Agent, Webhook, Slack, System) gets the empty
//! integration — the engine's `no_token` path — regardless of what the
//! workflow config declares.

use fabro_types::Principal;
use fabro_types::settings::run::ResolvedGithubIntegration;

/// Neutralize a resolved GitHub integration for non-user principals.
///
/// Returns the integration unchanged for `Principal::User` and
/// [`ResolvedGithubIntegration::default`] (empty permissions — the engine's
/// no-token sentinel) for every agent-side principal. A token grant is only
/// ever the explicit act of a human running a workflow themselves.
#[must_use]
pub(crate) fn gate_github_integration(
    subject: &Principal,
    integration: ResolvedGithubIntegration,
) -> ResolvedGithubIntegration {
    if matches!(subject, Principal::User(_)) {
        return integration;
    }
    if integration.is_token_requested() {
        let principal_kind: &'static str = subject.clone().into();
        tracing::warn!(
            principal_kind,
            "credential grant denied: agent-authored config cannot mint GITHUB_TOKEN \
             (ADR-0019.6, credential-free sandbox)"
        );
    }
    ResolvedGithubIntegration::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn integration_with_permissions() -> ResolvedGithubIntegration {
        ResolvedGithubIntegration {
            permissions:             std::iter::once(("contents".to_string(), "write".to_string()))
                .collect(),
            additional_repositories: std::collections::BTreeSet::default(),
        }
    }

    #[test]
    fn user_principal_keeps_requested_integration() {
        let subject = Principal::User(fabro_types::UserPrincipal {
            identity:    fabro_types::IdpIdentity::new("fabro:dev", "dev")
                .expect("valid dev identity"),
            login:       "dev".to_string(),
            auth_method: fabro_types::AuthMethod::DevToken,
            avatar_url:  None,
        });
        let integration = gate_github_integration(&subject, integration_with_permissions());
        assert!(integration.is_token_requested());
    }

    #[test]
    fn worker_principal_gets_no_token_even_when_config_requests_one() {
        let subject = Principal::Worker {
            run_id: fabro_types::RunId::new(),
        };
        let integration = gate_github_integration(&subject, integration_with_permissions());
        assert!(
            !integration.is_token_requested(),
            "worker runs must take the no-token path regardless of config"
        );
    }

    #[test]
    fn system_principal_without_request_stays_empty() {
        let subject = Principal::System {
            system_kind: fabro_types::SystemActorKind::Engine,
        };
        let integration = gate_github_integration(&subject, ResolvedGithubIntegration::default());
        assert!(!integration.is_token_requested());
    }
}
