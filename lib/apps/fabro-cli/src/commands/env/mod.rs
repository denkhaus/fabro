mod list;
mod pin_toolchain;
mod show;
mod update;

use anyhow::{Context as _, Result};
use fabro_api::types;
use fabro_client::api_failure_for;

use crate::args::{EnvCommand, EnvNamespace};
use crate::command_context::CommandContext;
use crate::server_client::Client;

pub(crate) async fn dispatch(ns: EnvNamespace, base_ctx: &CommandContext) -> Result<()> {
    let ctx = base_ctx.with_target(&ns.target)?;
    match ns.command {
        EnvCommand::List(args) => list::list_command(&args, &ctx).await,
        EnvCommand::Show(args) => show::show_command(&args, &ctx).await,
        EnvCommand::Update(args) => update::update_command(&args, &ctx).await,
        EnvCommand::PinToolchain(args) => pin_toolchain::pin_toolchain_command(&args, &ctx).await,
    }
}

/// Outcome of an optimistic-concurrency replace attempt.
pub(super) enum ReplaceOutcome {
    /// The prepared request matched the stored environment; nothing was
    /// written.
    Unchanged(types::Environment),
    /// The environment was replaced; carries the revision observed before the
    /// write plus the updated definition with its new revision.
    Replaced {
        previous_revision: String,
        environment:       types::Environment,
    },
}

/// Converts a stored environment into a replace request body. The REST API
/// only accepts inline Dockerfile sources, so an environment backed by a
/// Dockerfile path fails here instead of being silently rewritten.
pub(super) fn environment_to_replace_request(
    environment: &types::Environment,
) -> Result<types::ReplaceEnvironmentRequest> {
    let id = environment.id.to_string();
    let mut value = serde_json::to_value(environment)
        .with_context(|| format!("serializing environment {id}"))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("environment {id} did not serialize to a JSON object"))?;
    object.remove("id");
    object.remove("revision");
    serde_json::from_value(value).with_context(|| {
        format!(
            "converting environment {id} to a replace request (Dockerfile path sources are not \
             supported by the environments API)"
        )
    })
}

pub(super) fn is_revision_conflict(err: &anyhow::Error) -> bool {
    api_failure_for(err).is_some_and(|failure| failure.status.as_u16() == 409)
}

/// One optimistic-concurrency replace attempt cycle: `prepare` inspects the
/// freshly-read environment and either reports no write needed (`Ok(None)`)
/// or produces the request body to persist.
pub(super) async fn replace_attempt(
    client: &Client,
    id: &str,
    prepare: &mut dyn FnMut(
        &types::Environment,
    ) -> Result<Option<types::ReplaceEnvironmentRequest>>,
) -> Result<ReplaceOutcome> {
    let current = client.retrieve_environment(id).await?;
    let expected_revision = current.revision.to_string();
    let Some(request) = prepare(&current)? else {
        return Ok(ReplaceOutcome::Unchanged(current));
    };
    match client
        .replace_environment(id, &expected_revision, request)
        .await
    {
        Ok(environment) => Ok(ReplaceOutcome::Replaced {
            previous_revision: expected_revision,
            environment,
        }),
        Err(err) if is_revision_conflict(&err) => Err(err.context(format!(
            "environment {id} revision mismatch: expected {expected_revision}, server is at a \
             newer revision"
        ))),
        Err(err) => Err(err.context(format!("updating environment {id}"))),
    }
}

/// GET-revision + PUT If-Match with a single retry on a revision race. The
/// caller's `prepare` runs against each freshly-read environment, so
/// fail-closed checks (like pin parity) are re-verified before the retry.
pub(super) async fn replace_with_retry(
    client: &Client,
    id: &str,
    mut prepare: impl FnMut(&types::Environment) -> Result<Option<types::ReplaceEnvironmentRequest>>,
) -> Result<ReplaceOutcome> {
    match replace_attempt(client, id, &mut prepare).await {
        Ok(outcome) => Ok(outcome),
        Err(first) if is_revision_conflict(&first) => {
            match replace_attempt(client, id, &mut prepare).await {
                Ok(outcome) => Ok(outcome),
                Err(second) => Err(second.context(format!(
                    "environment {id} replace retried once after a revision race and failed again"
                ))),
            }
        }
        Err(first) => Err(first),
    }
}

/// Renders a 64-hex revision as its 12-char prefix for tables and progress
/// lines.
pub(super) fn short_revision(revision: &str) -> &str {
    revision.get(..12).unwrap_or(revision)
}

#[cfg(test)]
mod tests {
    use fabro_environment::Environment;

    use super::environment_to_replace_request;

    fn stored_environment() -> Environment {
        serde_json::from_value(serde_json::json!({
            "id": "toolchain",
            "revision": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "provider": "docker",
            "image": {
                "docker": "ghcr.io/denkhaus/fabro-toolchain:0000000000aa",
                "dockerfile": null
            },
            "resources": { "cpu": null, "memory": null, "disk": null },
            "network": { "mode": "allow_all", "allow": [] },
            "lifecycle": {
                "preserve": false,
                "stop_on_terminal": true,
                "auto_stop": null
            },
            "labels": {},
            "env": {}
        }))
        .unwrap()
    }

    #[test]
    fn stored_environment_converts_to_replace_request() {
        let request = environment_to_replace_request(&stored_environment()).unwrap();
        assert_eq!(
            request.image.docker.as_deref(),
            Some("ghcr.io/denkhaus/fabro-toolchain:0000000000aa")
        );
        assert_eq!(request.provider, fabro_types::SandboxProviderKind::DOCKER);
    }
}
