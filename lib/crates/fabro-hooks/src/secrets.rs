use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use fabro_redact::SecretRedactor;
use fabro_types::settings::interp::Namespace;

use crate::config::{HookDefinition, HookType};

type SecretLookupFuture = Pin<Box<dyn Future<Output = Option<String>> + Send + 'static>>;
type SecretLookup = dyn Fn(String) -> SecretLookupFuture + Send + Sync + 'static;

/// Per-run hook secret resolver.
///
/// The resolver is cheap to clone and must be constructed per run. It returns
/// only token-shaped vault secrets supplied by the worker and shares the run's
/// [`SecretRedactor`] so values resolved by hooks join the same redaction
/// registry as run-boundary environment and prepare-step secrets.
#[derive(Clone)]
pub struct HookSecretResolver {
    lookup:   Option<Arc<SecretLookup>>,
    redactor: SecretRedactor,
}

impl HookSecretResolver {
    #[must_use]
    pub fn new(redactor: SecretRedactor) -> Self {
        Self {
            lookup: None,
            redactor,
        }
    }

    #[must_use]
    pub fn with_lookup<F, Fut>(redactor: SecretRedactor, lookup: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<String>> + Send + 'static,
    {
        Self {
            lookup: Some(Arc::new(move |name| Box::pin(lookup(name)))),
            redactor,
        }
    }

    #[must_use]
    pub fn redactor(&self) -> &SecretRedactor {
        &self.redactor
    }

    pub async fn resolve_for_definition(&self, definition: &HookDefinition) -> ResolvedHookSecrets {
        self.resolve_names(secret_names_for_definition(definition))
            .await
    }

    async fn resolve_names<I, S>(&self, names: I) -> ResolvedHookSecrets
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut values = HashMap::new();
        let Some(lookup) = self.lookup.as_ref() else {
            return ResolvedHookSecrets::new(values, self.redactor.clone());
        };

        for name in names {
            let name = name.as_ref();
            if values.contains_key(name) {
                continue;
            }
            if let Some(value) = lookup(name.to_string()).await {
                values.insert(name.to_string(), value);
            }
        }

        ResolvedHookSecrets::new(values, self.redactor.clone())
    }
}

impl Default for HookSecretResolver {
    fn default() -> Self {
        Self::new(SecretRedactor::default())
    }
}

/// Secrets resolved for one hook firing.
#[derive(Clone, Default)]
pub struct ResolvedHookSecrets {
    values:   HashMap<String, String>,
    redactor: SecretRedactor,
}

impl ResolvedHookSecrets {
    #[must_use]
    pub fn new(values: HashMap<String, String>, redactor: SecretRedactor) -> Self {
        for value in values.values() {
            redactor.register(value);
        }
        Self { values, redactor }
    }

    #[must_use]
    pub fn empty_with_redactor(redactor: SecretRedactor) -> Self {
        Self::new(HashMap::new(), redactor)
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<String> {
        let value = self.values.get(name).cloned();
        if let Some(value) = value.as_deref() {
            self.redactor.register(value);
        }
        value
    }

    #[must_use]
    pub fn redactor(&self) -> &SecretRedactor {
        &self.redactor
    }
}

fn secret_names_for_definition(definition: &HookDefinition) -> Vec<String> {
    let Some(hook_type) = definition.resolved_hook_type() else {
        return Vec::new();
    };
    match hook_type.as_ref() {
        HookType::Command { command } => command
            .names(Namespace::Secrets)
            .into_iter()
            .map(str::to_string)
            .collect(),
        HookType::Http { url, .. } => url
            .names(Namespace::Secrets)
            .into_iter()
            .map(str::to_string)
            .collect(),
        HookType::Prompt { prompt, model } | HookType::Agent { prompt, model, .. } => {
            let mut names: Vec<String> = prompt
                .names(Namespace::Secrets)
                .into_iter()
                .map(str::to_string)
                .collect();
            if let Some(model) = model {
                names.extend(
                    model
                        .names(Namespace::Secrets)
                        .into_iter()
                        .map(str::to_string),
                );
            }
            names
        }
    }
}
