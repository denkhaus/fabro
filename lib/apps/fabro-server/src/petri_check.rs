//! Petri's check as Fabro's judge of a workflow.
//!
//! The create handler, the validate and preflight endpoints and the offline
//! `fabro validate` all ask Petri the same question: does this bundle, under
//! these settings, lower to a graph Petri admits? This module builds the
//! request from a workflow bundle and the run's settings, runs the check,
//! and hands back the admitted graphs with Petri's diagnostics in Fabro's
//! shape. Fabro adds one rule of its own: a workflow with a node that runs a
//! model is refused when no LLM provider is ready, since Petri admits the
//! model nodes unchecked without a model client. A second rule of the same
//! family (fabro-b46e) refuses a run whose stages need a provider with no
//! stored credential — readiness, not admission, is Fabro's to judge, and a
//! miss that would surface as an in-stage authentication error (or, for a
//! graph that routes the failed leg onward, as a green run) is refused at
//! the fire instead.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use fabro_llm::lithos_catalog::Catalog;
use fabro_llm::selection;
use fabro_petri::check::{
    self, Admitted, Bundle, CheckError, CheckRequest, Diagnostic, Launch, ModelRequirement,
};
use fabro_petri::runtime::RuntimeSpec;
use fabro_types::diagnostic::{Diagnostic as FabroDiagnostic, Severity};
use fabro_types::settings::run::RunGoal;
use fabro_types::{ManifestPath, WorkflowSettings};
use fabro_workflow::Error as WorkflowError;
use fabro_workflow::workflow_bundle::WorkflowBundle;
use lithos_llm::catalog::ProviderId;

/// Fabro's rule for a model node with no provider ready to run it.
pub(crate) const NO_READY_PROVIDER_RULE: &str = "fabro.model.no_ready_provider";

/// Fabro's rule for a stage whose resolved provider has no stored
/// credential (fabro-b46e): the fire is refused naming provider and model.
pub(crate) const PROVIDER_NOT_READY_RULE: &str = "fabro.model.provider_not_ready";

/// What a readiness check needs (fabro-b46e): the catalog to resolve
/// selectors with, the providers with stored credentials, and whether an
/// operator override downgrades the refusal to a warning.
pub(crate) struct ModelReadiness<'a> {
    pub(crate) catalog: &'a Catalog,
    pub(crate) ready:   &'a [ProviderId],
    pub(crate) force:   bool,
}

/// Owned readiness inputs for a check that crosses a thread boundary
/// (fabro-b46e): the server's catalog and ready providers, or no
/// credential knowledge at all for an offline check, where neither
/// readiness rule fires.
pub(crate) enum Readiness {
    Offline,
    Server {
        catalog: std::sync::Arc<Catalog>,
        ready:   Vec<ProviderId>,
        force:   bool,
    },
}

impl Readiness {
    /// Whether any provider is ready: the input of the no-ready rule.
    pub(crate) fn has_ready_provider(&self) -> bool {
        match self {
            Self::Offline => true,
            Self::Server { ready, .. } => !ready.is_empty(),
        }
    }

    /// The per-provider rule's inputs, when credential readiness is known.
    pub(crate) fn as_model_readiness(&self) -> Option<ModelReadiness<'_>> {
        match self {
            Self::Offline => None,
            Self::Server {
                catalog,
                ready,
                force,
            } => Some(ModelReadiness {
                catalog,
                ready,
                force: *force,
            }),
        }
    }
}

/// Fabro's refusal of a model node when no provider is ready at all
/// ([`NO_READY_PROVIDER_RULE`]): the launch has no model to run on. Built
/// by the check at create and re-built over a stored admission when a
/// successor re-checks readiness (fabro-f93b).
pub(crate) fn no_ready_provider_diagnostic() -> FabroDiagnostic {
    FabroDiagnostic {
        rule: NO_READY_PROVIDER_RULE.to_string(),
        severity: Severity::Error,
        message: "no default model is available: no LLM provider is ready, and the \
                  workflow has a node that runs a model"
            .to_string(),
        fix: Some(
            "configure a provider credential (for example `OPENAI_API_KEY`) or a \
             `[run.model]`"
                .to_string(),
        ),
        ..FabroDiagnostic::default()
    }
}

/// One provider the run will use for a model stage, resolved against the
/// catalog and the ready set (fabro-b869 step 4): the launch selection
/// would pick one of these. `model` names the stage's selector when one
/// was stated, for probes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequiredProvider {
    pub provider: ProviderId,
    pub model:    Option<String>,
}

/// Resolve one requirement's candidate providers: the pinned provider
/// canonicalized, else every enabled provider offering the model. Empty
/// when the selector names nothing the catalog knows (admission owns
/// those refusals).
fn requirement_providers(catalog: &Catalog, requirement: &ModelRequirement) -> Vec<ProviderId> {
    match &requirement.provider {
        Some(provider) => selection::require_provider(catalog, provider)
            .map(|canonical| vec![canonical])
            .unwrap_or_default(),
        None => match &requirement.model {
            Some(model) => catalog
                .offerings_matching(model)
                .iter()
                .map(|offering| offering.provider.id().clone())
                .collect(),
            None => Vec::new(),
        },
    }
}

/// The providers the run's model stages will actually use (fabro-b869
/// step 4): every requirement whose candidates include a credential-ready
/// provider maps to those ready candidates — the selection would pick one
/// of them. Requirements with no ready candidate are readiness misses
/// ([`model_readiness_diagnostics`]) and never reach a window check.
pub(crate) fn required_providers(
    catalog: &Catalog,
    admitted: Option<&Admitted>,
    ready: &[ProviderId],
) -> Vec<RequiredProvider> {
    let Some(admitted) = admitted else {
        return Vec::new();
    };
    let mut required: Vec<RequiredProvider> = Vec::new();
    for requirement in admitted.model_requirements() {
        let candidates = requirement_providers(catalog, &requirement);
        let usable = candidates
            .iter()
            .filter(|provider| ready.contains(provider))
            .cloned()
            .collect::<Vec<_>>();
        if usable.is_empty() {
            continue;
        }
        for provider in usable {
            if !required
                .iter()
                .any(|known| known.provider == provider && known.model == requirement.model)
            {
                required.push(RequiredProvider {
                    provider,
                    model: requirement.model.clone(),
                });
            }
        }
    }
    required
}

/// The credential-readiness diagnostics of a run's model requirements:
/// one per requirement whose provider is not ready (fabro-b46e). The
/// requirements come off the admitted graphs at create, or off a source
/// run's stored admission for a successor (fabro-f93b).
///
/// Skipped when no provider is ready at all — [`NO_READY_PROVIDER_RULE`]
/// owns that case — and per requirement when a selector does not resolve
/// (Petri's admission owns unknown models and providers) or the node states
/// neither (the launch default is ready by construction). `force` keeps the
/// finding but downgrades it to a warning, so an override stays visible.
pub(crate) fn model_readiness_diagnostics(
    readiness: &ModelReadiness<'_>,
    requirements: &[ModelRequirement],
) -> Vec<FabroDiagnostic> {
    if readiness.ready.is_empty() {
        return Vec::new();
    }
    let mut misses: BTreeMap<(String, Option<String>), ProviderMiss> = BTreeMap::new();
    for requirement in requirements {
        let candidates = requirement_providers(readiness.catalog, requirement);
        if candidates.is_empty() {
            continue;
        }
        let required_providers = candidates;
        let representative = required_providers.first().cloned();
        if required_providers
            .iter()
            .any(|provider| readiness.ready.contains(provider))
        {
            continue;
        }
        let providers = required_providers
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("`, `");
        // The vault entry an operator creates for the miss: the preferred
        // secret name of the first provider the selector names (fabro-b46e
        // names provider, model AND the missing secret).
        let expected_secret = representative.as_ref().and_then(|provider| {
            readiness
                .catalog
                .enabled_provider(provider.as_str())
                .and_then(fabro_auth::expected_secret_name)
        });
        misses
            .entry((providers, requirement.model.clone()))
            .or_default()
            .record(requirement.node.clone(), expected_secret);
    }
    misses
        .into_iter()
        .map(|((providers, model), miss)| {
            let mut nodes = miss.nodes;
            nodes.sort();
            let model = model.as_deref().map_or_else(
                || "the provider's default model".to_string(),
                |model| format!("model `{model}`"),
            );
            let secret = miss.expected_secret.map_or_else(String::new, |secret| {
                format!(" (expected secret: `{secret}`)")
            });
            FabroDiagnostic {
                rule: PROVIDER_NOT_READY_RULE.to_string(),
                severity: if readiness.force {
                    Severity::Warning
                } else {
                    Severity::Error
                },
                message: format!(
                    "stage(s) {} run {model} on provider(s) `{providers}`, for which no credential is stored{secret} — the run would die in those stages",
                    nodes.join(", ")
                ),
                fix: Some(
                    "store a credential for the provider (server vault / `fabro install`) or set run.model to a ready provider; `--force` fires anyway"
                        .to_string(),
                ),
                ..FabroDiagnostic::default()
            }
        })
        .collect()
}

/// One readiness miss being aggregated: the stages that need it and the
/// vault entry an operator would create.
#[derive(Default)]
struct ProviderMiss {
    nodes:           Vec<String>,
    expected_secret: Option<String>,
}

impl ProviderMiss {
    fn record(&mut self, node: String, expected_secret: Option<String>) {
        self.nodes.push(node);
        self.expected_secret = self.expected_secret.take().or(expected_secret);
    }
}

/// The launch Fabro binds around the settings: the run's model and provider
/// below them, and the environment the run selected and the goal the run
/// resolved above them. When the settings name neither model nor provider, the
/// default offering of the eligible providers is bound as the launch model
/// alone: a node that names no model runs on it, and a node that names a model
/// the catalog lacks stays unqualified, so Petri's admission refuses it.
pub(crate) fn launch(
    catalog: &Catalog,
    settings: &WorkflowSettings,
    eligible: &[ProviderId],
    environment: Option<&str>,
    repository: Option<PathBuf>,
) -> Launch {
    let model = settings.run.model.name.clone().or_else(|| {
        if settings.run.model.provider.is_some() {
            return None;
        }
        let eligible = eligible.iter().cloned().collect::<HashSet<_>>();
        selection::select_default(catalog, &eligible)
            .ok()
            .map(|offering| offering.model.id().to_string())
    });
    Launch {
        model,
        provider: settings.run.model.provider.clone(),
        environment: environment.map(str::to_owned),
        goal: launch_goal(settings),
        repository,
    }
}

/// The goal the run resolved, for Petri to bind over the bundle's layers
/// and the graph's own `goal`: the settings' inline `run.goal`, which the
/// create path has layered (an intent's override over the workflow layer
/// over the server's defaults) and whose workflow-layer `file` form is
/// inlined before layering. A `file` form that survives layering (a server
/// default) is left to the bundle's own `[run.goal]`, which Petri reads
/// itself; the text is not read here, away from the run's working
/// directory.
#[expect(
    clippy::disallowed_methods,
    reason = "goal text passes through in source form, as `materialize_admitted_run` displays it; \
              Petri renders `{{ inputs.* }}` and `{{ vars.* }}` in it as it renders `[run] goal`"
)]
fn launch_goal(settings: &WorkflowSettings) -> Option<String> {
    match settings.run.goal.as_ref()? {
        RunGoal::Inline(text) => Some(text.as_source()),
        RunGoal::File(_) => None,
    }
}

/// The launch with no catalog to pick a default from: what the settings
/// name, for a check away from the server.
pub(crate) fn launch_without_catalog(settings: &WorkflowSettings) -> Launch {
    Launch {
        model:       settings.run.model.name.clone(),
        provider:    settings.run.model.provider.clone(),
        environment: None,
        goal:        launch_goal(settings),
        repository:  None,
    }
}

/// The check request for `bundle`'s `entrypoint`: every file of every
/// workflow in the bundle at its bundle-relative path, the run's inputs and
/// variables, the launch and the runtime. `unbound_is_warning` makes a
/// template that reads an input nothing binds a warning, for a validation
/// before the run's inputs exist; a run's admission never sets it.
pub(crate) fn check_request(
    bundle: &WorkflowBundle,
    entrypoint: &ManifestPath,
    settings: &WorkflowSettings,
    vars: &HashMap<String, String>,
    launch: Launch,
    runtime: RuntimeSpec,
    unbound_is_warning: bool,
) -> Result<CheckRequest, WorkflowError> {
    let mut files = BTreeMap::new();
    for workflow in bundle.workflows().values() {
        for (path, text) in &workflow.files {
            files.insert(path.to_string(), text.clone());
        }
        files.insert(workflow.path.to_string(), workflow.source.clone());
        if let Some(config) = &workflow.config {
            files.insert(config.path.to_string(), config.source.clone());
        }
    }
    let mut inputs = BTreeMap::new();
    for (name, value) in &settings.run.inputs {
        let value = serde_json::to_value(value).map_err(|err| {
            WorkflowError::engine_with_source(
                format!("run input `{name}` does not encode as JSON"),
                err,
            )
        })?;
        inputs.insert(name.clone(), value);
    }
    Ok(CheckRequest {
        bundle: Bundle {
            files,
            entrypoint: entrypoint.to_string(),
            project_toml: None,
        },
        inputs,
        vars: vars
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        launch,
        runtime,
        unbound_is_warning,
    })
}

/// What a check said: the admitted graphs when Petri admitted the workflow,
/// and every diagnostic, Petri's and Fabro's, in Fabro's shape.
pub(crate) struct Checked {
    pub(crate) admitted:    Option<Admitted>,
    pub(crate) diagnostics: Vec<FabroDiagnostic>,
}

impl Checked {
    pub(crate) fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// Run Petri's check. A refusal comes back as its diagnostics with no
/// graphs; an admission as the graphs with Petri's warnings, plus Fabro's
/// refusal of a model node when `has_ready_provider` is false. Blocking: it
/// lowers the graph and runs the admission passes synchronously.
pub(crate) fn check(
    request: &CheckRequest,
    has_ready_provider: bool,
) -> Result<Checked, WorkflowError> {
    match check::check(request) {
        Ok(admitted) => {
            let mut diagnostics = admitted
                .warnings
                .iter()
                .map(fabro_diagnostic)
                .collect::<Vec<_>>();
            if !has_ready_provider && admitted.needs_model() {
                diagnostics.push(no_ready_provider_diagnostic());
            }
            Ok(Checked {
                admitted: Some(admitted),
                diagnostics,
            })
        }
        Err(CheckError::Rejected(diagnostics)) => Ok(Checked {
            admitted:    None,
            diagnostics: diagnostics.iter().map(fabro_diagnostic).collect(),
        }),
        Err(other) => Err(WorkflowError::engine_with_source(
            "Petri could not check the workflow",
            other,
        )),
    }
}

/// Petri's diagnostic in Fabro's shape: the code is the rule, the hint is
/// the fix, the bundle-relative file and position are the source location.
fn fabro_diagnostic(diagnostic: &Diagnostic) -> FabroDiagnostic {
    FabroDiagnostic {
        rule: diagnostic.code.clone(),
        severity: if diagnostic.is_error() {
            Severity::Error
        } else {
            Severity::Warning
        },
        message: diagnostic.message.clone(),
        fix: diagnostic.hint.clone(),
        source_path: Some(diagnostic.file.clone()),
        line: diagnostic.line,
        column: diagnostic.column,
        ..FabroDiagnostic::default()
    }
}
