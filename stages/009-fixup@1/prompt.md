Goal: # Provider-aware model aliases and API IDs

## Outcome

Fabro workflows can name a model with one stable model slug or alias and run unchanged against whichever provider the operator has available. A model offering is identified by `(provider, ModelId)`, so the same `ModelId` and the same alias may appear on multiple providers. For an unqualified selector, Fabro filters to ready providers and then uses provider priority to choose one offering deterministically.

The motivating behavior is:

| Ready providers | Selector | Selected offering |
| --- | --- | --- |
| OpenAI only | `gpt-56-sol` | OpenAI's `gpt-5.6-sol` |
| OpenRouter only | `gpt-56-sol` | OpenRouter's `gpt-5.6-sol` offering |
| OpenAI and OpenRouter | `gpt-56-sol` | OpenAI, because its provider priority is higher |
| OpenAI and OpenRouter, explicit `provider = "openrouter"` | `gpt-56-sol` | OpenRouter, because an explicit provider is a pin |

The provider-facing API identifier remains an implementation detail of the selected offering. It defaults to the canonical model slug and can be overridden with `api_id` when a provider uses another convention.

## Scope and design decisions

### Vocabulary and identity

- `ProviderId` identifies who serves the request, such as `openai` or `openrouter`.
- `ModelId` is the canonical, human-facing model slug, such as `gpt-5.6-sol` or `claude-opus-4-8`. It never means an alias.
- An alias is an alternate user-facing selector, such as `gpt-56-sol` or `opus`.
- An offering is one provider's route to one `ModelId`. Its stable identity is `(ProviderId, ModelId)`.
- `api_id` is the opaque string sent to that offering's provider API.
- `family` remains model metadata used for display and matching; it is not a routing namespace and is not combined with `provider` or `api_id`.

Do not add a separate runtime `LogicalModel` type. Use the existing `Model` as the provider-specific offering and use the existing `ModelId` newtype for its canonical ID. Internally, tuple keys `(ProviderId, ModelId)` are enough; do not add an `OfferingId` type unless implementation pressure demonstrates a real invariant it would protect.

### Canonical configuration shape

Move model declarations under their provider, but keep the human model slug as the model table key:

```toml
[llm.providers.openai]
priority = 90

[llm.providers.openai.models."gpt-5.6-sol"]
display_name = "GPT-5.6 Sol"
family = "gpt-5"
aliases = ["gpt-56-sol"]
default = true

[llm.providers.openrouter]
priority = 25

[llm.providers.openrouter.models."gpt-5.6-sol"]
api_id = "openai/gpt-5.6-sol"
display_name = "GPT-5.6 Sol (via OpenRouter)"
family = "gpt-5"
aliases = ["gpt-56-sol"]
default = true
```

This shape provides a natural unique key without making humans author an API identifier or repeat `provider = "..."` inside every model. Model settings continue to field-merge by provider and model slug across configuration layers.

At catalog build time:

```text
effective_api_id = configured api_id, otherwise ModelId's exact slug
```

Reject an explicitly empty `api_id`. Do not perform provider-specific string rewrites, prefix inference, or template expansion. A future template feature may be authoring sugar that produces the same resolved `api_id`, but it is not part of this change.

### Alias and selection semantics

Build candidate sets rather than a global `identifier -> one model` map:

- Canonical model IDs may repeat across providers.
- Aliases may repeat across providers and may point to different canonical model IDs on different providers. This supports both strict synonyms and portable role-like aliases.
- Within one provider, a canonical ID or alias must identify exactly one offering. Reject two models on the same provider that claim the same alias.
- Across providers, an alias may collide with a canonical `ModelId`; the canonical-before-alias check order keeps canonical IDs reliable pins, and the shadowed alias stays reachable through its provider-qualified form. Within one provider, the previous rule already rejects the collision.
- An explicit provider restricts lookup to that provider and bypasses provider priority.
- An unqualified selector considers only eligible providers, then sorts by provider priority descending and canonical provider ID ascending.
- A canonical `ModelId` match is checked before alias matches.
- Disabled providers and disabled offerings are absent from candidate sets.

"Eligible" must be supplied by the caller rather than inferred inside the catalog:

- Runtime calls use providers whose adapters registered successfully. This accounts for credentials and adapter initialization, not merely an enabled catalog row.
- Static validation explicitly uses all enabled catalog providers and proves that at least one candidate exists without claiming that credentials are available.
- An explicit but unavailable provider remains a pin and produces a clear unavailable-provider error; Fabro must not silently switch it.

When a run is created, resolve every implicit selector once and persist the chosen canonical model ID and provider. Resume uses that materialized choice; it does not reconsider provider priority because credentials changed. Runtime fallbacks remain the mechanism for handling a later provider failure.

Preserve the existing passthrough behavior for uncatalogued models: when a provider is explicit, send the unknown model string unchanged and use the provider's default route policy. An unknown unqualified model may use the runtime's default ready provider as it does today, but it cannot participate in alias-based provider selection.

## Implementation plan

### 1. Make configuration provider-scoped

Files centered on:

- `lib/crates/fabro-config/src/layers/llm.rs`
- `lib/crates/fabro-config/src/builders.rs`
- `lib/crates/fabro-model/src/catalog.rs`
- `lib/crates/fabro-model/src/catalog/providers/*.toml`

Changes:

1. Add `models: MergeMap<ModelSettings>` to `ProviderSettings` and the equivalent model map to `ProviderCatalogSettings`.
2. Remove `provider` from the canonical model-row shape; the containing provider supplies it.
3. Normalize catalog data into provider/model pairs before catalog building, preserving layer precedence independently for each pair.
4. Convert every built-in provider TOML to `[providers.<provider>.models."<model-slug>"]`.
5. Re-key OpenRouter, Bedrock, and other aggregator offerings by Fabro's model slug rather than their provider API ID. Retain explicit `api_id` overrides for `author/model`, Bedrock profile IDs, deployment names, and other exceptions.
6. Remove redundant `api_id` fields where they equal the model slug.
7. Do not add an unverified provider offering merely to match the motivating example; exercise the exact example with a catalog fixture and use existing verified cross-provider models in the built-in catalog.

Compatibility:

- Accept the current `[llm.models."<id>"]` plus `provider = "..."` form as a temporary input shape. A row that omits provider adopts the provider of the unique known offering matching its id or alias; if none or several match, fail with an error naming the row.
- Normalize each source layer into the canonical provider-scoped form before combining layers, so old and new definitions retain correct precedence.
- Reject a single source that defines the same `(provider, model)` through both syntaxes instead of choosing silently.
- Keep built-ins and documentation exclusively on the new syntax. Do not add a filesystem rewrite migration yet because LLM catalog layers can come from more places than one owned settings file; the compatibility parser covers all of those boundaries safely.
- Ship a retired-identifier map for re-keyed built-in ids (old catalog key to provider plus new slug). Any selector or persisted model reference matching a retired id fails with a typed error naming the new address; nothing silently re-routes. One mechanism covers old config references, workflow graphs, and resumed pre-change runs.

### 2. Rebuild catalog identity and indexes

Files centered on:

- `lib/crates/fabro-model/src/ids.rs`
- `lib/crates/fabro-model/src/types.rs`
- `lib/crates/fabro-model/src/catalog.rs`
- `lib/crates/fabro-model/src/model_ref.rs`
- `lib/crates/fabro-model/src/billing.rs`

Changes:

1. Change `Model.id` from `String` to the transparent `ModelId` newtype and correct `ModelId` documentation so aliases are not described as model IDs. JSON remains a plain string.
2. Key resolved model settings by `(ProviderId, ModelId)` rather than model ID alone.
3. Replace the one-to-one `model_index` with:
    - an offering index keyed by `(ProviderId, ModelId)`;
    - canonical-ID candidates keyed by `ModelId`;
    - alias candidates keyed by alias string.
4. Pre-sort candidate vectors with the catalog's provider ordering so every caller receives the same priority and tie-break behavior.
5. Replace global `Catalog::get`-style assumptions with explicit methods:
    - lookup on a named provider;
    - selection from an eligible-provider set;
    - lookup of settings from a resolved `Model` offering;
    - listing every offering, optionally by provider.
6. Make pricing, billing, codec, profile, probe, default, and closest-model lookups use the composite identity. Resolve the run-level default model with the same selection algorithm (default-flagged candidates from eligible providers, ordered by provider priority) without requiring providers to agree on their defaults.
7. Replace `DuplicateModelIdentifier` with provider-scoped validation errors that name the provider, selector, and conflicting model IDs.
8. Add a typed selection error that distinguishes an unknown selector from a known selector with no eligible offering. Preserve error sources and render strings only at CLI/API boundaries.

### 3. Centralize provider-aware resolution

Files centered on:

- `lib/crates/fabro-model/src/catalog.rs`
- `lib/crates/fabro-types/src/settings/model_ref.rs`
- `lib/crates/fabro-workflow/src/handler/llm/routing.rs`
- `lib/crates/fabro-workflow/src/transforms/model_resolution.rs`
- `lib/crates/fabro-workflow/src/run_materialization.rs`
- `lib/crates/fabro-workflow/src/operations/start.rs`

Changes:

1. Implement one catalog selection algorithm taking a selector, optional explicit provider, and eligible provider IDs.
2. Make generic `ModelRef` parsing classify bare versus provider-qualified input only. It must not try to infer a unique provider for a bare alias, because a valid alias may now have several provider candidates.
3. Keep the existing `provider/model` qualified syntax in this change. The model slug never contains the provider API ID, so OpenRouter's slash is no longer part of the user-facing model address.
4. Update workflow graph model resolution and run materialization to receive the ready-provider snapshot already collected during run creation.
5. Materialize aliases to canonical `(provider, ModelId)` values in both node attributes and run defaults before persistence.
6. Keep static validation credential-independent by resolving against all enabled candidates only for existence/capability checks.
7. Update fallback resolution so:
    - a provider-only fallback still selects the closest compatible model;
    - a provider-qualified model/alias resolves within that provider;
    - a bare model/alias uses the fallback-time eligible set and provider priority;
    - provider-name/model-name ambiguity becomes a user-facing typed error; today AmbiguousModelRef is silently swallowed by fallback resolution, so pin this behavior change with a test.

### 4. Resolve the offering before LLM dispatch

Files centered on:

- `lib/crates/fabro-llm/src/client.rs`
- `lib/crates/fabro-llm/src/adapter_registry.rs`
- `lib/crates/fabro-llm/src/providers/common.rs`
- provider adapter modules under `lib/crates/fabro-llm/src/providers/`

Changes:

1. For requests without an explicit provider, select among the client's successfully registered providers using catalog priority.
2. For requests with an explicit provider, resolve the model or alias only on that provider and fail if the adapter is unavailable.
3. Canonicalize a cloned request to the selected `ModelId` before validation, costing, and dispatch; leave caller-owned request data unchanged.
4. Resolve route metadata and `api_id` from the selected composite offering. Provider adapters must pass their own canonical provider ID into catalog lookups rather than looking up settings by model string alone.
5. Ensure response costing and billing use the same resolved offering that was dispatched.
6. Keep explicit-provider unknown-model passthrough intact.

### 5. Update server, API, CLI, and web identities

Files centered on:

- `docs/public/api-reference/fabro-api.yaml`
- `lib/crates/fabro-server/src/server/handler/models.rs`
- `lib/crates/fabro-server/src/server/handler/sessions.rs`
- `lib/crates/fabro-cli/src/commands/model.rs`
- `apps/fabro-web/app/routes/settings-models.tsx`
- generated clients in `lib/crates/fabro-api` and `lib/packages/fabro-api-client`

Changes:

1. Continue returning one `Model` row per offering from `GET /models`. Document that `id` is unique within a provider and that `(provider, id)` is the resource identity.
2. Add an optional `provider` query parameter to `POST /models/{id}/test`. With a provider it tests that exact offering; without one it selects among ready providers by priority.
3. Include `provider` in `ModelTestResult` so the tested offering is explicit.
4. Make model-test lookup, auth issues, and probing use the selected offering rather than a global first match.
5. Update the CLI so bulk tests always pass each row's provider, and an explicit `--provider` plus `--model` remains pinned. Match returned results by `(provider, id)`.
6. Update the settings models page to key row state by `(provider, id)` and send the provider when testing a row; duplicate IDs must render and update independently.
7. Update session/playground/completion resolution to use ready provider IDs and persist or return the selected provider alongside the canonical model. Enumerate the OpenAPI schema changes this implies for session, playground, and completion resources; sessions currently store only a bare model-id string.
8. Regenerate Rust and TypeScript API clients from the OpenAPI source after changing the contract.

### 6. Document the mental model

Files centered on:

- `lib/crates/fabro-dev/src/commands/docs_options_reference.rs`
- `docs/public/reference/user-configuration.mdx` (generated region)
- `docs/public/core-concepts/models.mdx`
- `docs/public/execution/run-configuration.mdx`
- `docs/public/execution/failures.mdx`

Document:

1. Provider, model slug, family metadata, alias, and API ID as distinct terms.
2. Provider-scoped model configuration and the `api_id = model slug` default.
3. The OpenAI/OpenRouter portability example and the priority table from this plan.
4. Explicit provider selection as a pin and unqualified selection as availability plus priority.
5. Alias reuse across providers, including the same-provider ambiguity rule.
6. Resolution-once behavior for persisted runs and the separate role of runtime fallback chains.
7. API IDs as opaque provider wire values that workflows should not reference.

Run `cargo dev docs refresh` after editing the generator-owned reference.

## Test plan

### Catalog and configuration tests

Add focused unit tests proving:

- two providers can declare the same canonical `ModelId`;
- two providers can declare the same alias;
- only OpenAI eligible selects OpenAI;
- only OpenRouter eligible selects OpenRouter and its overridden API ID;
- both eligible select the higher-priority provider;
- equal priorities use canonical provider ID as the tie-breaker;
- an explicit provider overrides priority;
- a disabled or ineligible provider is not selected;
- two different models on one provider cannot claim the same alias;
- an unqualified selector matching both a canonical ID and another provider's alias selects the canonical model, while the alias offering stays reachable provider-qualified;
- omitted `api_id` resolves to the exact model slug;
- explicit `api_id` is preserved and an empty override is rejected;
- provider/model layer merges do not overwrite the same slug on another provider;
- the temporary old config shape normalizes correctly and a same-source old/new collision errors clearly.
- a provider-less legacy row adopts the unique matching offering's provider, and a retired built-in id fails with the typed error naming its replacement.

### Routing and wire tests

Add `fabro-llm` tests with fake registered providers or local capture servers that submit the same alias under three availability configurations. Assert the selected adapter and the exact wire model value, including OpenRouter's `author/model` override. Also cover explicit provider, unknown passthrough, request-control validation, and cost lookup on duplicate model IDs.

### Workflow tests

Add crate-level workflow tests that create the same workflow with:

- only the direct provider ready;
- only the aggregator ready;
- both ready;
- an explicit lower-priority provider.

Assert the persisted graph and run settings contain the selected canonical model and provider. Add a resume-oriented test showing that changing the ready provider set does not re-resolve a materialized run. Add fallback tests for a shared bare alias and a provider-qualified alias, including propagation of a provider/model ambiguity error.

### API, CLI, and web tests

- Server: list two rows with the same ID but different providers; filter by provider; test each exact offering; test priority selection when provider is omitted.
- CLI: bulk model tests do not conflate duplicate IDs, and JSON output includes the selected provider.
- Web: duplicate-ID rows have independent React keys and test-result state, and each request includes the row provider.
- API generation: retain the existing `Model` Rust type replacement and add or update JSON parity/type-identity coverage as required by the API policy.

Use unit/crate integration tests for catalog and routing behavior. Use the existing command/API test layers only for their public contracts; no live provider credentials are required.

## Verification

Run, in this order:

```sh
cargo build -p fabro-api
cd lib/packages/fabro-api-client && bun run generate
cargo dev docs refresh
cargo nextest run -p fabro-model
cargo nextest run -p fabro-config
cargo nextest run -p fabro-llm
cargo nextest run -p fabro-workflow
cargo nextest run -p fabro-server
cd apps/fabro-web && bun test
cd apps/fabro-web && bun run typecheck
cargo dev docs check
cargo +nightly-2026-04-14 fmt --check --all
cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings
ulimit -n 4096 && cargo nextest run --workspace
cargo build --workspace
```

Before accepting any changed snapshots, run `cargo insta pending-snapshots` and inspect the complete pending set.

## Completion criteria

- A workflow using one shared alias runs unchanged for an OpenAI-only operator and an OpenRouter-only operator.
- When both are ready, provider priority selects deterministically.
- Explicit provider selection always pins the provider.
- The selected offering's exact `api_id` reaches the provider wire request.
- No catalog, routing, billing, API, CLI, or UI lookup treats model ID alone as a globally unique offering identity.
- Built-ins and public documentation use provider-scoped model-slug keys and omit redundant API IDs.
- Existing user catalog syntax remains readable through the compatibility normalization path.

## Unresolved questions

- What release or date should end support for the legacy top-level `[llm.models]` syntax? This does not block implementation; the plan keeps it as a compatibility input and makes the new provider-scoped form canonical.


## Completed stages
- **toolchain**: succeeded
  - Script: `command -v cargo >/dev/null || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && sudo ln -sf $HOME/.cargo/bin/* /usr/local/bin/; }; cargo --version 2>&1`
  - Output:
    ```
    cargo 1.96.0 (30a34c682 2026-05-25)
    ```
- **preflight_compile**: succeeded
  - Script: `cargo check -q --workspace 2>&1`
  - Output: (empty)
- **preflight_lint**: succeeded
  - Script: `cargo +nightly-2026-04-14 clippy -q --workspace --all-targets -- -D warnings 2>&1`
  - Output: (empty)
- **implement**: failed
- **simplify_fable**: failed
- **simplify_sol**: failed
- **verify**: failed
  - Script: `git fetch origin main 2>&1 && git merge --no-edit --no-stat origin/main 2>&1 && cargo +nightly-2026-04-14 fmt --all 2>&1 && cargo dev docs refresh 2>&1 && cargo +nightly-2026-04-14 fmt --check --all 2>&1 && { command -v rg >/dev/null 2>&1 || { echo 'rg is required for verify'; exit 127; }; } && ! rg -n 'AuthMode::Disabled|RunAuthMethod|RunSubjectProvenance|\bActorRef\b|\bActorKind\b|AuthenticatedSubject|AuthenticatedService|AuthorizeRunScoped|AuthorizeRunBlob|AuthorizeStageArtifact|AuthorizeCommandLog|auth_method\s*==\s*"disabled"' lib/crates apps lib/packages docs/public/api-reference/fabro-api.yaml 2>&1 && cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings 2>&1 && cargo nextest run --workspace --status-level slow --profile ci 2>&1 && cargo dev docs check 2>&1 && bun install --frozen-lockfile 2>&1 && (cd apps/fabro-web && bun run typecheck) 2>&1 && (cd apps/fabro-web && bun run test) 2>&1 && (cd lib/packages/fabro-api-client && bun run typecheck) 2>&1 && cargo dev build -- -p fabro-cli --release 2>&1`
  - Output:
    ```
    (421 lines omitted)
    542 |        let model = if let Some(model) = args.model.clone() {
        |  __________________-
    543 | |          model
        | |          ----- expected because of this
    544 | |      } else {
    545 | |/         catalog
    546 | ||             .default_for_provider(&provider_id)
    547 | ||             .map(|model| model.id.clone())
    548 | ||             .ok_or_else(|| {
    ...   ||
    552 | ||             })?
        | ||_______________^ expected `String`, found `ModelId`
    553 | |      };
        | |______- `if` and `else` have incompatible types
        |
    help: try using a conversion method
        |
    552 |             })?.to_string()
        |                ++++++++++++
    
       Compiling dialoguer v0.12.0
       Compiling debugid v0.8.0
    For more information about this error, try `rustc --explain E0308`.
    error: could not compile `fabro-agent` (lib) due to 1 previous error
    warning: build failed, waiting for other jobs to finish...
    ```

## Context
- failure_class: deterministic
- failure_signature: verify|deterministic|script failed with exit code: <n> ## output x) compiling html5ever v0.<n>.<n> compiling num v0.<n>.<n> compiling slatedb-txn-obj v0.<n>.<n> compiling figment v0.<n>.<n> compiling foyer v0.<n>.<n> compiling croner v3.<n>.<n> compiling flatbu


The verify step failed. Read the build output from context and fix all format, clippy, Rust test, docs, TypeScript typecheck/test, and build failures.