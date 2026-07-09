Goal: # Interpolation follow-up — LLM provider `extra_headers` → `InterpString`

**Date:** 2026-07-07
**Part of:** the interpolation-unification effort (see
`2026-06-02-interpolation-unification-plan.md`). This closes the
`llm.providers.<id>.extra_headers.*` row in the master plan's "Category
audit" table.
**Format follows:** `2026-07-01-secrets-plan-C-redaction-and-hooks.md`.
**Standalone:** this plan migrates **`extra_headers` only**. `base_url`
promotion is explicitly **deferred** — see the one-line exception below. There
is nothing to gate on; ship it against `main` as it stands.

> **Token notation in this file.** This plan will be fed to the implement-plan
> workflow, whose goal/prompt is MiniJinja-rendered with strict-undefined —
> so a literal double-brace token in this prose would either expand or error.
> Interpolation tokens are therefore written here **without** their enclosing
> double curly braces: read `env.NAME`, `secrets.NAME`, `vars.NAME`,
> `inputs.NAME` as the double-curly-brace-wrapped token form used everywhere
> else in the codebase. **When you produce code, tests, and docs, write the
> real double-curly-brace syntax** (e.g. a header value of
> `env.PORTKEY_API_KEY` must appear in the actual TOML/Rust/Markdown as that
> name wrapped in double curly braces with single interior spaces).

> **Plain-English rule.** This vault plan uses internal shorthand (`DD-x`, the
> master plan's `D9`/`D11`, "Phase 6") for traceability. **Do not** carry any
> of it into shippable artifacts — code, comments, commit messages, PR
> title/body, or docs. Describe *what the change does* in plain English. If you
> catch a stray reference while editing, strip or rewrite it.

---

## Overall goal

Make `llm.providers.<id>.extra_headers.*` interpolate under the project's single
narrow-token type, matching the rest of the interpolation effort. Migrate the
value type from the bespoke `HeaderValueRef` object
(`{ literal = .. }` / `{ env = .. }` / `{ vault = .. }`) to `InterpString`
(narrow `ns.NAME` tokens). The three old forms map onto tokens:

- `{ literal = "X" }` → the plain text `X`
- `{ env = "NAME" }` → `env.NAME`
- `{ vault = "NAME" }` → `secrets.NAME` (Token-only, via `vault_get_token`)

`extra_headers` is a **server/catalog-scope survivor that resolves at the run
boundary**: the resolution context provides `env` + `secrets` only. A `vars.*`
or `inputs.*` token in a header is a typed error (they are not in scope) — this
falls out for free from the resolution context; no extra guard is needed.

### These are LLM-provider headers, not hook headers

`extra_headers` are **host-side, outbound to the LLM provider** (e.g. a Portkey
gateway virtual key). They **legitimately carry credentials**, so a `secrets.*`
token (Token-only) **is allowed** here. This is the opposite of HTTP-hook
headers, which reject `secrets.*` tokens. Do **not** conflate the two: do not
copy the hook-header "reject secrets" guard into the provider-header path.

### `base_url` is out of scope (documented exception)

`llm.providers.<id>.base_url` **stays a plain literal `String`** in this PR — it
is not promoted to `InterpString` here. This is an intentional, documented
exception; do not touch `base_url`'s type, its resolution, or its docs beyond
the one clarifying sentence noted in the Docs section. (`api_key_url` likewise
stays a plain literal — display-only "where to get a key" link, never a
connection target.)

---

## Architecture facts the implementer needs (verified on `main` @ `d5dcd1179`, 2026-07-09, with file:line — re-verify before starting; treat cited line numbers as anchors, not gospel)

### The hard layering constraint (drives the whole design)

`fabro-types` **depends on** `fabro-model` (`lib/crates/fabro-types/Cargo.toml`
lists `fabro-model = { path = "../fabro-model" }`). `InterpString` lives in
`lib/crates/fabro-types/src/settings/interp.rs`. Therefore **`fabro-model`
cannot use `InterpString`** — that would be a dependency cycle. This is exactly
why `fabro-model` carries its own reference types (`CredentialRef`,
`HeaderValueRef`) instead of `InterpString`.

Consequence, and the central design decision (DD-1): the **catalog types in
`fabro-model` store the header value as its raw source `String`**, and the value
is parsed into `InterpString` and resolved **in `fabro-auth`** (which depends on
both `fabro-model` and `fabro-types`) at credential-resolution time. The
user-facing authoring surface (`fabro-config`) is typed `InterpString`. This
mirrors the existing `CredentialRef` pattern (model layer stores a
reference string/enum; `fabro-auth` resolves it against env/vault).

### Where the field is defined today

- **Config layer (user-facing authoring surface)** — `ProviderSettings` in
  `lib/crates/fabro-config/src/layers/llm.rs`:
  - `pub extra_headers: Option<HashMap<String, HeaderValueRef>>` — **:81**
    (doc comment **:77-79** describes "typed so secret-bearing values stay as
    references").
  - `pub use fabro_model::{ CredentialRef, CredentialRefParseError,
    HeaderValueRef, ReasoningEffortFeature };` re-export — **:33-35**.
- **`HeaderValueRef` type** — `lib/crates/fabro-model/src/catalog.rs`:
  - enum **:347-352**; `impl Serialize` **:354-369**; `impl Display`
    **:371-379** (renders `literal:<redacted>` / `env:NAME` / `vault:ID`);
    `HeaderValueRefInput` (untagged) **:381-386**; `HeaderValueRefSerde`
    **:388-397**; `impl Deserialize` **:399-414** (maps a bare string to
    `D::Error::custom("header value must be a table")` at **:407-410**);
    `impl TryFrom<HeaderValueRefSerde>` **:416-442**; `non_empty_header_value`
    **:444-450**; `HeaderValueRefParseError` **:452-458**.
- **Catalog settings struct** `ProviderCatalogSettings` in
  `lib/crates/fabro-model/src/catalog.rs`:
  - `pub extra_headers: Option<HashMap<String, HeaderValueRef>>` — **:61**.
- **Resolved catalog view** `CatalogProvider` in
  `lib/crates/fabro-model/src/catalog.rs`:
  - `pub extra_headers: HashMap<String, HeaderValueRef>` — **:515**.
- **Merge** `merge_provider_settings`: `extra_headers` at **:1221**
  (`higher.extra_headers.or(fallback.extra_headers)` — whole-map replace).
- **Construction** from settings → `CatalogProvider`: **:1376**
  (`extra_headers: settings.extra_headers.clone().unwrap_or_default()`).
- **Config→catalog conversion** `provider_settings_to_catalog` in
  `lib/crates/fabro-config/src/builders.rs:329-346` — `extra_headers` is a
  straight field move at **:341** today.
- **`Combine for Option<HashMap<String, HeaderValueRef>>`** —
  `lib/crates/fabro-config/src/layers/combine.rs:125-129` (whole-map replace via
  `.or()`; `HeaderValueRef` imported at **:18**). Note: `InterpString` is
  **already imported** in this file at **:13**
  (`use fabro_types::settings::{Duration, InterpString, Size};`).
- **Re-exports of `HeaderValueRef` to strip:**
  `lib/crates/fabro-model/src/lib.rs:22`,
  `lib/crates/fabro-config/src/layers/mod.rs:24`,
  `lib/crates/fabro-config/src/lib.rs:48`.

### The two resolution sites (where headers attach to outbound requests)

Data flow: config layer → `ProviderCatalogSettings` → merged `CatalogProvider`
→ **`fabro-auth` resolves** → `ApiCredential { extra_headers:
HashMap<String,String>, .. }` (`lib/crates/fabro-auth/src/resolve.rs:36-45`) →
`AdapterConfig` (`lib/crates/fabro-llm/src/adapter_registry.rs`) → adapter
`with_default_headers(...)` → outbound HTTP.

Exactly **two** functions read `CatalogProvider.extra_headers` (the
`HeaderValueRef` map). Confirmed by `rg -n '\.extra_headers' lib/crates` — every
other `.extra_headers` read is on the already-resolved
`ApiCredential`/`AdapterConfig` `HashMap<String,String>` and needs no change:

1. **`EnvCredentialSource`** (env only, no vault) —
   `lib/crates/fabro-auth/src/env_source.rs`. `resolved_extra_headers`
   **:77-94** matches `HeaderValueRef::{Literal,Env,Vault}` — today `Vault(_)` →
   `None` → mapped to `ResolveError::NotConfigured(provider.id)` at **:90**.
   `use fabro_model::{..., HeaderValueRef, ...}` import at **:5**.
2. **`CredentialResolver`** (used by the vault-backed source, env + vault) —
   `lib/crates/fabro-auth/src/resolve.rs`. `resolved_extra_headers_for_catalog`
   **:355-377** matches `Literal` → itself, `Env(name)` → `self.lookup_env`,
   `Vault(name)` → `vault.get(name)` (**:371**, untyped `vault.get`, not
   Token-only today), missing → `ResolveError::NotConfigured` at **:373**.
   `use fabro_model::{..., HeaderValueRef, ...}` import at **:5**. Called from
   four `to_api_credential`/`api_credential_from_provider_auth` sites
   (**:393, :411, :428, :460**), all propagating via `?` — the function
   signature `Result<HashMap<String,String>, ResolveError>` is unchanged, only
   the body changes.

`ApiCredential::from_api_key` (`resolve.rs:50-69`, the sync interactive/probe
constructor) sets `extra_headers: HashMap::new()` (**:63**) — it never resolves
headers, so **no change** is needed there or at its call sites.

**Which source is used at the run boundary:** the primary in-run path builds a
vault-backed `CredentialResolver` with **both** worker-process env and the run's
vault available (`operations/start.rs`). `EnvCredentialSource` is the
local/standalone path (env only; a `secrets.*` header token is `Unavailable`
there → fails closed).

### `InterpString` API you will call (`lib/crates/fabro-types/src/settings/interp.rs`)

- Construct from source: `InterpString::parse(&s)` (**:185**, infallible,
  permissive — an unknown double-brace-shaped span stays literal text) or the
  `From<String>`/`From<&str>` impls.
- Detect a pure literal: `is_literal()` (**:222**).
- Serialize back to source: `as_source()` (**:261**) — **clippy-gated**
  (`disallowed_methods`); every intentional call needs an
  `#[expect(clippy::disallowed_methods, reason = "...")]`. `Serialize` already
  round-trips via `as_source` (**:530-538**), so serde stores/loads source text.
- Resolve: `resolve_with(&mut ResolveCtx)` (**:287**) →
  `Result<String, ResolveError>` — the resolved string directly. (The former
  `Resolved { value, provenance }` wrapper was removed when unused provenance
  tracking was deleted, merged 2026-07-09 as "Remove unused provenance
  tracking from config interpolation" — there is **no `.value` access
  anywhere**; the code snippets below already reflect this.) A token in a
  namespace the ctx has **no lookup for** → `ResolveError { kind: Unavailable }`;
  a lookup miss → `kind: Missing`.
- `ResolveCtx::new()` (**:125**) `.with_env(f)` (**:130**) `.with_secrets(f)`
  (**:142**). Both take `impl FnMut(&str) -> Option<String>`. Providing only
  `env`+`secrets` (no `with_vars`) is exactly the catalog-scope contract: a
  `vars.*` token → `Unavailable`, and `inputs.*` is hard-wired to `None`
  (**:156**) so `inputs.*` → a template-pointing `Unavailable` message
  (**:508-516**). **No extra guard needed to reject vars/inputs — the omitted
  lookups do it.**
- `ResolveError` (**:465-496**) `Display` (**:498-525**) names only
  `namespace` + `name`, quoting the name via `{:?}` (e.g. `secret "API_KEY"
  referenced by ... is not set`). It **never echoes a resolved value** — this
  satisfies the "error messages never echo a possible literal secret"
  requirement for free.
- Import path used elsewhere in `fabro-config`:
  `use fabro_types::settings::InterpString;`. For the resolver, import both
  `InterpString` and (aliased to avoid clashing with `fabro-auth`'s own
  `ResolveError`) the interp error, e.g.
  `use fabro_types::settings::{InterpString, ResolveCtx};` and refer to the
  interp error as `fabro_types::settings::ResolveError`.

### The guardrail being removed (must be a deliberate call — DD-4)

`HeaderValueRef` **deliberately rejects bare string values** so a raw credential
can't be pasted as a header literal. Three tests enforce/exercise this in
`lib/crates/fabro-config/src/layers/llm.rs`:
`header_value_ref_rejects_bare_string` (**:459-478**, asserts the deserializer
message does **not** echo `sk-portkey-literal`) and
`provider_extra_headers_reject_bare_string_values` (**:603-618**, same). The
deserializer at `catalog.rs:407-410` maps a bare string to the custom error
"header value must be a table".

`InterpString` **allows bare literals by design** (any string parses;
deserialize is infallible). So this migration **removes that guardrail**:
`x-api-key = "sk-…"` becomes a valid plaintext header literal. This is fine for
`X-Title`-style headers but is a real change for credential-shaped ones. The
default mitigation is **docs-only**: steer users to `secrets.*` (see Docs). An
optional soft `tracing::warn!` lint is specified in a clearly-delimited section
at the end of Implementation; ship it only if Scott flips the default.

`InterpString` resolution already guarantees no value is echoed on error, so the
docs-only path introduces no leak.

### Public API surface — nothing leaks, no wire change

- The public `Provider` projection (`lib/crates/fabro-model/src/provider.rs`)
  **omits `extra_headers` entirely** (doc comment **:10-11**). Server test
  `leaked extra_headers` (`lib/crates/fabro-server/src/server/tests.rs:6754`)
  asserts the `/providers` response has no `extra_headers` key — still true.
- `extra_headers` is not on the OpenAPI wire. No `fabro-api` /
  `fabro-api-client` / `apps/fabro-web` change is expected.

### Complete file inventory (grep-traced: `rg -n HeaderValueRef` + `rg -n '\.extra_headers'` + `rg '\{\s*(literal|env|vault)\s*='`)

**Production code that references `HeaderValueRef` (8 files):**

1. `lib/crates/fabro-model/src/catalog.rs` — **delete** `HeaderValueRef` + its
   serde/`Display`/`TryFrom`/helper + `HeaderValueRefParseError`; change catalog
   field types (`ProviderCatalogSettings.extra_headers` :61,
   `CatalogProvider.extra_headers` :515) to `String`-valued maps.
2. `lib/crates/fabro-model/src/lib.rs:22` — drop `HeaderValueRef` from the
   `pub use catalog::{...}` block.
3. `lib/crates/fabro-config/src/layers/llm.rs` — field type (:81), doc comment
   (:77-79), re-export (:33-35), and all `HeaderValueRef` tests
   (:403-534, :570-618, :865-934).
4. `lib/crates/fabro-config/src/layers/combine.rs` — `Combine` impl (:125-129)
   and import (:18).
5. `lib/crates/fabro-config/src/layers/mod.rs:24` — drop the re-export.
6. `lib/crates/fabro-config/src/lib.rs:48` — drop the re-export.
7. `lib/crates/fabro-auth/src/env_source.rs` — resolution (:77-94), import (:5),
   error-handling arm (:120), and tests (:283-330, :332-378).
8. `lib/crates/fabro-auth/src/resolve.rs` — resolution (:355-377), import (:5),
   new `Interpolation` error variant + `auth_issue_message` arm; add tests.

**Other code that must change (not caught by `HeaderValueRef` grep):**

9. `lib/crates/fabro-config/src/builders.rs:329-346` — `provider_settings_to_catalog`
   must collapse the authoring `InterpString` values to catalog source strings.
10. `lib/crates/fabro-llm/src/client.rs:1653` — test TOML uses
    `x-portkey-api-key = { literal = "pk-live" }`; this is parsed through the
    catalog deserializer and will **fail to parse** once the catalog field is a
    plain string. Update to `x-portkey-api-key = "pk-live"`.
11. `lib/crates/fabro-model/src/catalog/providers/openrouter.toml:18-19` — a
    **comment** showing the old `{ literal = .. }` header form. Cosmetic
    (comments don't parse), but update it for accuracy.

**Stale doc plan (do NOT edit):**
`docs/superpowers/plans/2026-05-17-provider-adapter-boundary-cleanup.md:64`
references `HeaderValueRef` as historical plan text — leave it.

### Generated vs hand-written docs (important: `user-configuration.mdx` is partly generated)

`lib/crates/fabro-dev/src/commands/docs_options_reference.rs` **generates into**
`docs/public/reference/user-configuration.mdx` between the fences
`{/* generated:options */}` (line **153**) and `{/* /generated:options */}`
(line **474**). So:

- **Generated (edit the `.rs` source, then run `cargo dev docs`):** the LLM
  catalog TOML example at `docs_options_reference.rs:233-236` and the
  `extra_headers` table row at **:249** — these render into
  `user-configuration.mdx` :187-190 and :203 (both **inside** the fence). Do
  **not** hand-edit those `.mdx` lines; `cargo dev docs check` would fail.
- **Hand-written (edit the `.mdx` directly):**
  - `docs/public/reference/user-configuration.mdx:96-98` — the first
    `extra_headers` example, which sits **before** the fence (line 153).
  - `docs/public/integrations/openrouter.mdx:139-141`.
  - `docs/public/core-concepts/models.mdx:57-58`.
- **Historical changelog — leave as-is:**
  `docs/public/changelog/2026-05-13.mdx:28-29` shows the old form as it shipped
  on that date. Do not rewrite history; add a **new** changelog entry for this
  breaking change instead (see Docs).

---

## Design decisions (fixed)

- **DD-1 — Catalog stores source `String`; `fabro-auth` resolves.** Forced by
  the `fabro-types` → `fabro-model` dependency direction. `fabro-model` catalog
  fields become `Option<HashMap<String, String>>` (`ProviderCatalogSettings`)
  and `HashMap<String, String>` (`CatalogProvider`). The stored string is
  `InterpString` **source**, re-parsed and resolved in `fabro-auth`.
  - *Rejected alternative — relocate `InterpString` into `fabro-model`.* That
    moves a foundational type (plus its `is_env_style_name` helper) across
    crates — a foundation-scale change, not a field migration. Not worth it for
    one re-parse at credential-build time. Only fall back to this if a reviewer
    strongly demands typed-end-to-end.
- **DD-2 — Authoring surface (`fabro-config`) is typed `InterpString`.** Serde
  round-trips as source. The config→catalog conversion collapses it to source
  via an allowlisted `as_source()` — the standard config→catalog serialization
  boundary.
- **DD-3 — Resolution scope = env + secrets only.** Build the `ResolveCtx` with
  `with_env` + (when a vault is present) `with_secrets`, and **no** `with_vars`.
  `vars.*`/`inputs.*` then fail as `Unavailable` for free. `secrets.*` uses the
  Token-only vault lookup (`vault_get_token`,
  `lib/crates/fabro-auth/src/vault_ext.rs:23`), consistent with the merged
  run-boundary secrets work (`Token` → string; `File`/`Oauth` ineligible). This
  is a like-for-like preservation of today's `HeaderValueRef::Vault` →
  `vault.get(name)` behavior, **tightened to Token-only**: a header pointing at
  a `File`/`Oauth` vault entry now fails closed. Call this out in the PR body —
  it is a deliberate, safe tightening.
- **DD-4 — Removing the "no bare credential values" guardrail is deliberate;
  default mitigation is docs-only.** Document (PR + docs) that secret-bearing
  headers should use `secrets.NAME` (or `env.NAME`), never bare text. The soft
  `tracing::warn!` lint is **optional** (delimited section below); default =
  docs-only. Any error/reject text must never echo a literal —
  `InterpString` resolution already guarantees this; the optional lint logs the
  header **name** and guidance only, never the value.
- **DD-5 — `base_url` and `api_key_url` stay plain literal `String`.** Not in
  scope for this PR; documented exception (see Overall goal / Docs). No type,
  resolution, or behavior change to either.
- **DD-6 — New `fabro-auth` error variant `Interpolation`, fail-closed, no value
  leak.** A missing/unavailable/wrong-schema header token surfaces as an
  `auth_issue`, not a swallowed success. Its inner `Display` (the interp
  `ResolveError`) names only namespace+name.

---

## Conventions

- **TDD.** Write the failing test first, watch it fail (red), then implement
  (green). Every behavior below has a named test in the Tests section.
- **Hermetic tests.** Use temp-dir vaults for secrets (`tempfile::tempdir()` +
  `Vault::load`, as in `resolve.rs`/`vault_ext.rs` tests); **strip ambient
  provider API keys** in any test that builds a catalog/credentials against
  process env (known workflow-test flake when provider keys leak in from the
  environment — inject env via an explicit `EnvLookup` closure, never read real
  `std::env`).
- **Rust style.** No manual shell escaping (not expected here). Import **types**
  by name, call **functions** via their parent module. `strum` for any enum
  string/int conversions (not expected here — the enum is being deleted).
  **No glob imports** in production code.
- **Never print or log a resolved header value.** Rely on existing content-based
  redaction; do not add new logging of resolved values.
- **Redactor-registration gap (record only; do not implement here).** This PR
  makes `fabro-auth` a secrets-resolution site that does not flow through the
  run boundary's vault lookup — so once a per-run exact-match redaction
  registry exists (wired in `operations/start.rs`), a secret resolved into a
  provider header will not be registered in it. Exposure is low (headers are
  host-side, outbound-only, never logged); the follow-up is to thread a
  registering secrets lookup into `VaultCredentialSource` where it is built at
  the run boundary. Leave a short code comment at the `fabro-auth` resolve
  site noting this, and state the gap plainly in the PR description.
- **Plain-English** commits/PR/comments/docs — no internal shorthand.
- **Verify gate** (see Verify) must be green before done.

---

## Implementation

Work in this order (each step's behavior has a named test in the Tests section;
write the test first).

### Step 1 — Config authoring surface → `InterpString` (`fabro-config`)

File `lib/crates/fabro-config/src/layers/llm.rs`:

1a. Change `ProviderSettings.extra_headers` (**:81**) from
`Option<HashMap<String, HeaderValueRef>>` to
`Option<HashMap<String, InterpString>>`. Update the doc comment (**:77-79**) to
describe narrow interpolation tokens: "Extra HTTP headers attached to every
outgoing provider request after credential resolution. Values are interpolation
strings: literal text, `env.NAME`, or `secrets.NAME` (write the real
double-brace token form). Put credentials in a secret and reference them with a
`secrets.NAME` token, not a bare literal."

1b. In the `pub use fabro_model::{...}` block (**:33-35**), remove
`HeaderValueRef`. **Keep** `CredentialRef` and `CredentialRefParseError` (still
used by the `CredentialRef` tests and `ProviderAuthConfig`). Add
`use fabro_types::settings::InterpString;`.

File `lib/crates/fabro-config/src/layers/combine.rs`:

1c. Replace the `Combine for Option<HashMap<String, HeaderValueRef>>` impl
(**:125-129**) with `Combine for Option<HashMap<String, InterpString>>` — same
whole-map `.or()` body (preserves the wholesale-replace merge). Drop
`HeaderValueRef` from the import at **:18** (`InterpString` is already imported
at **:13**, so no new import line is needed).

Files `lib/crates/fabro-config/src/layers/mod.rs:24` and
`lib/crates/fabro-config/src/lib.rs:48`: remove the `HeaderValueRef` re-exports
(keep the neighboring re-exports).

### Step 2 — Catalog types → source `String`; delete `HeaderValueRef` (`fabro-model`)

File `lib/crates/fabro-model/src/catalog.rs`:

2a. `ProviderCatalogSettings.extra_headers` (**:61**) →
`Option<HashMap<String, String>>`.

2b. `CatalogProvider.extra_headers` (**:515**) → `HashMap<String, String>`.

2c. `merge_provider_settings` (**:1221**, `.or()`) and construction (**:1376**,
`.clone().unwrap_or_default()`) compile unchanged — both work for the `String`
map. No logic change.

2d. **Delete** `HeaderValueRef` and everything that exists only to serve it:
enum (**:347-352**), `impl Serialize` (**:354-369**), `impl Display`
(**:371-379**), `HeaderValueRefInput` (**:381-386**), `HeaderValueRefSerde`
(**:388-397**), `impl Deserialize` (**:399-414**), `impl TryFrom` (**:416-442**),
`non_empty_header_value` (**:444-450**), and `HeaderValueRefParseError`
(**:452-458**). Remove `HeaderValueRef` from the `pub use catalog::{...}` block
in `lib/crates/fabro-model/src/lib.rs:22`. Leave `deserialize_knowledge_cutoff`
(**:460**) untouched.

Note: the builtin provider TOMLs (`src/catalog/providers/*.toml`) declare no
`extra_headers`, so nothing in the embedded catalog regresses. Update the
`openrouter.toml` comment (step 5b).

### Step 3 — Collapse authoring `InterpString` → catalog source string (`fabro-config`)

File `lib/crates/fabro-config/src/builders.rs`, `provider_settings_to_catalog`
(**:329-346**), the `extra_headers` field move (**:341**):

```
extra_headers: settings
    .extra_headers
    .map(|m| m.into_iter().map(|(k, v)| (k, v.as_source())).collect()),
```

Add `#[expect(clippy::disallowed_methods, reason = "collapse the authoring
InterpString header values to their catalog source strings; they are re-parsed
and resolved at the credential boundary")]` on the function (or narrowly on this
statement). `base_url` (**:340**) is unchanged (still `Option<String>`).

### Step 4 — Resolve headers in `fabro-auth` (the real interpolation)

4a. **New error variant.** In `ResolveError`
(`lib/crates/fabro-auth/src/resolve.rs:127-152`) add:

```
#[error("{provider} header interpolation failed: {source}")]
Interpolation {
    provider: ProviderId,
    #[source]
    source:   fabro_types::settings::ResolveError,
},
```

Add the matching arm to `auth_issue_message` (**:155-176**, currently
exhaustive) — surface it as a loud, diagnosable misconfiguration, e.g.
`format!("{provider_name} header interpolation failed: {source}")`. The inner
`Display` names only namespace+name, never a value.

4b. **`CredentialResolver` / vault-backed source**
(`lib/crates/fabro-auth/src/resolve.rs`), `resolved_extra_headers_for_catalog`
(**:355-377**): rewrite the per-header match. For each `(name, source)`:

```
let value = InterpString::parse(source)
    .resolve_with(
        &mut ResolveCtx::new()
            .with_env(|n| self.lookup_env(n))
            .with_secrets(|n| vault_get_token(vault, n).ok().flatten()),
    )
    .map_err(|source| ResolveError::Interpolation {
        provider: provider.clone(),
        source,
    })?;
```

- This preserves the old behavior: `Literal` → itself, `Env` → env (now via an
  `env.*` token), `Vault` → vault (now via a `secrets.*` token) — and adds
  multi-segment support (e.g. a literal `Bearer ` prefix followed by a
  `secrets.TOKEN` token in one header value).
- `vault_get_token` is already imported at **:15**; `ResolveCtx`/`InterpString`
  need importing (see the InterpString API note; alias the interp error type to
  avoid clashing with the local `ResolveError`).
- **Token-only fail-closed:** wrapping `vault_get_token(...).ok().flatten()`
  mirrors the merged run-boundary lookup helper (`operations/start.rs:566`'s
  `vault_token_lookup`). A `File`/`Oauth` vault entry (schema mismatch) →
  `Err` → `None` → the interp resolver returns a `Missing` error → mapped to
  `Interpolation`. This fails closed. (Optional precision: if you want the exact
  "wrong schema" message instead of "not set", capture the `VaultLookupError`
  via a `Cell<Option<_>>` side-channel and prefer it after `resolve_with`
  returns — not required for correctness; the simple form is preferred for
  consistency with the boundary helper.)
- Drop `HeaderValueRef` from the import at **:5** (keep `ApiKeyHeaderPolicy`,
  `Catalog`, `CredentialRef`, `ProviderId`). The four call sites (**:393, :411,
  :428, :460**) are unchanged — the signature stays
  `Result<HashMap<String,String>, ResolveError>`.
- `provider_base_url_for_catalog` (**:349-353**) and every `base_url` copy stay
  exactly as-is (DD-5).

4c. **`EnvCredentialSource`** (`lib/crates/fabro-auth/src/env_source.rs`),
`resolved_extra_headers` (**:77-94**): rewrite the per-header match env-only:

```
let value = InterpString::parse(source)
    .resolve_with(&mut ResolveCtx::new().with_env(|n| self.lookup(n)))
    .map_err(|source| ResolveError::Interpolation {
        provider: provider.id.clone(),
        source,
    })?;
```

This source has no vault, so a `secrets.*` token → `Unavailable` → `Interpolation`
→ fail closed. Drop `HeaderValueRef` from the import at **:5**.

Because the error type changes from `NotConfigured` to `Interpolation`, broaden
the silent-drop guard in the `CredentialSource::resolve` match (**:120**) so an
auth provider whose header fails still drops silently (unchanged intent):

```
Err(ResolveError::NotConfigured(_) | ResolveError::Interpolation { .. })
    if provider.auth.is_some() => {}
```

`configured_providers` (**:139**) uses `self.resolved_extra_headers(provider).is_ok()`
and needs no change.

4d. **Sanity, no change:** `ApiCredential::from_api_key` (**:50-69**) sets
`extra_headers: HashMap::new()` and does not resolve headers.
`AdapterConfig.extra_headers` (`fabro-llm/src/adapter_registry.rs`) receives an
already-resolved `HashMap<String,String>`. No `fabro-llm` change.

### Step 5 — Fix test TOML and the catalog comment

5a. `lib/crates/fabro-llm/src/client.rs:1653`: change
`x-portkey-api-key = { literal = "pk-live" }` to `x-portkey-api-key = "pk-live"`
(this catalog TOML now deserializes header values as plain strings; the table
form would fail to parse). The test's assertion that the resolved header equals
`pk-live` still holds.

5b. `lib/crates/fabro-model/src/catalog/providers/openrouter.toml:18-19`: update
the commented example from `{ literal = .. }` to the plain-string / token form
(cosmetic).

### OPTIONAL — credential-shaped bare-literal lint (ship only if Scott flips DD-4)

> **This section is optional and OFF by default.** The default mitigation for
> the removed guardrail is docs-only (Docs section). Implement the lint only if
> Scott decides to warn on credential-shaped literals. It adds **one** new
> cross-crate edge (`fabro-config` → `fabro-redact`); if that edge is
> unwelcome, drop this section entirely — the migration still ships.

O-1. Add a public helper to `fabro-redact` (`lib/crates/fabro-redact/src/lib.rs`):

```
/// True when `s` contains a substring the redactor would treat as a secret
/// (high-entropy token or a known credential pattern). Used to warn when a
/// config literal looks like a pasted credential.
#[must_use]
pub fn looks_like_credential(s: &str) -> bool {
    !entropy::find_entropy_regions(s).is_empty()
        || !gitleaks::find_gitleaks_regions(s).is_empty()
}
```

`entropy::find_entropy_regions` (`entropy.rs:39`, `pub(super)`) and
`gitleaks::find_gitleaks_regions` (`gitleaks.rs:236`, `pub(super)`) are the same
finders `redact_string` uses (`lib.rs:46-47`) and are reachable from `lib.rs`.

O-2. In `provider_settings_to_catalog` (`builders.rs`, step 3), for each
`extra_headers` value where `InterpString::is_literal()` is true and
`fabro_redact::looks_like_credential(&value.as_source())`, emit
`tracing::warn!(header = %name, "provider extra header looks like a pasted
credential; use a secrets token instead of a literal")`. **Log the header name
and guidance only — never the value.**

O-3. Add `fabro-redact` to `lib/crates/fabro-config/Cargo.toml` `[dependencies]`
(it is **not** currently a dep — verified; `fabro-redact` is a leaf util, no
cycle). This is the one new cross-crate edge.

---

## Tests

Write each test **first** (red), then implement. Prefer the crate that owns the
behavior. Strip ambient provider keys; use temp-dir vaults.

### `fabro-config` — `layers/llm.rs`

Replace the `HeaderValueRef` test block (**:403-534**), the `extra_headers`
parse/reject tests (**:570-618**), and the merge tests (**:865-934**):

- `provider_extra_headers_parse_interp_tokens` — a provider block with
  `x-title = "My App"` (literal), `x-portkey-api-key = "env.PORTKEY_API_KEY"`,
  `x-team-secret = "secrets.gateway_team_secret"` (write the real double-brace
  form) parses; assert each stored `InterpString`'s `as_source()` round-trips to
  those exact strings.
- `provider_extra_headers_accepts_bare_string_literal` — **inverts** the deleted
  `provider_extra_headers_reject_bare_string_values`: a bare string value now
  parses as a literal `InterpString`. (Delete both
  `provider_extra_headers_reject_bare_string_values` at :603-618 and
  `header_value_ref_rejects_bare_string` at :459-478, plus the rest of the
  now-dead `HeaderValueRef` parse/format tests :405-534.)
- Update the merge tests, keeping their assertions, swapping the value type:
  `provider_extra_headers_map_replaces_wholesale` (**:865-896**) — proves
  **wholesale replace still works**; `provider_extra_headers_inherit_when_unset`
  (**:898-915**); `provider_extra_headers_empty_map_clears_lower_layer`
  (**:917-934**). Replace `HeaderValueRef::Env("PORTKEY_API_KEY")` etc. with
  `InterpString::from("env.PORTKEY_API_KEY")` (real double-brace form).
- **Keep unchanged:** `rejects_removed_provider_base_url_env_field` (:760-769),
  `parses_minimal_provider_entry` (:538, includes a literal `base_url`), and all
  `CredentialRef` tests.

### `fabro-model` — `catalog.rs`

Update or delete any test constructing `HeaderValueRef` or asserting its
`Display` (the catalog now holds plain strings). Confirm the builtin catalog
still builds via existing `from_builtin_toml` / `from_builtin_with_overrides`
coverage.

### `fabro-auth` — `env_source.rs` + `resolve.rs`

Rewrite the `{ env = .. }` TOML in `env_source.rs` tests (:283-330, :332-378),
and add resolver tests to `resolve.rs`:

- `env_source_resolves_literal_and_env_header_tokens` (rewrite of
  `resolve_registers_no_auth_provider_with_env_extra_headers`, :283-330) —
  no-auth provider with `x-portkey-api-key = "env.PORTKEY_API_KEY"` and
  `x-portkey-provider = "@bedrock-prod"` (literal); with the env var injected,
  `ApiCredential.extra_headers` holds the resolved values.
- `env_source_secrets_header_token_is_unavailable` — a `secrets.X` header on the
  env-only source fails closed (provider not registered / surfaced as an auth
  issue); the error does **not** contain any vault value.
- `env_source_reports_missing_env_header_for_no_auth_provider` (rewrite of
  `resolve_reports_missing_required_header_for_no_auth_provider`, :332-378) — a
  no-auth provider with a missing env-backed header surfaces an auth issue
  (now the `Interpolation` variant); assert it is surfaced (not silently
  dropped) and that no value leaks.
- `vault_source_resolves_secret_header_token` (new, `resolve.rs` tests) — build
  a temp vault with `vault_set_token(&mut vault, "gateway_team_secret", "s3cr3t")`;
  `catalog_with(...)` a **no-auth** provider whose
  `[providers.portkey.extra_headers]` has
  `x-team-secret = "secrets.gateway_team_secret"`;
  `test_resolver(vault, env_lookup)`; then
  `resolver.resolve(ProviderId::new("portkey"), CredentialUsage::ApiRequest, &catalog).await.unwrap()`
  (a no-auth provider dispatches through `api_credential_from_provider_auth`,
  resolve.rs:209-213) and assert `extra_headers["x-team-secret"] == "s3cr3t"`.
  This replaces the old `{ vault = .. }` behavior.
- `resolve_multi_segment_header_token` — a header value composed of the
  literal prefix `Bearer ` followed by a `secrets.TOKEN` token resolves to
  `Bearer <value>` (proves expressiveness beyond `HeaderValueRef`).
- `missing_secret_header_fails_without_echoing_value` — a `secrets.MISSING`
  header errors; assert the error string contains `MISSING` (the name) but plant
  a real value under a **different** vault name and assert that value is absent
  from the error.
- `header_with_file_or_oauth_vault_entry_fails_closed` — a `secrets.X` header
  where the vault entry `X` is an `Oauth` (or `File`) secret, not a `Token`,
  fails closed (header resolution errors / provider surfaces an auth issue) and
  no value leaks. Use `vault_set_oauth(...)` to seed a non-Token entry (as in
  `vault_ext.rs:131`).
- Leave literal-`base_url` assertions
  (`resolve_uses_catalog_credentials_and_base_url...`, env_source.rs:220-233;
  the resolver base_url assertions) unchanged — `base_url` is not migrated.

### `fabro-redact` — `lib.rs` (OPTIONAL, only with the lint)

- `looks_like_credential_detects_high_entropy_token` and
  `..._detects_gitleaks_pattern` (e.g. an `sk-...`/`AKIA...` shape) return true;
  `looks_like_credential_ignores_plain_text` (`"My App"`, `"@bedrock-prod"`, a
  plain URL) returns false.

### `fabro-config` lint — `builders.rs` (OPTIONAL, only with the lint)

- `warns_on_credential_shaped_bare_header_literal` — capture logs and assert a
  warning fires for a bare high-entropy literal header, **the value is not in
  the log**, and **no** warning fires for a `secrets.X` token or a benign
  literal.

---

## Docs

Update every surface to the token form; keep `cargo dev docs check` green.

1. **Generated** — `lib/crates/fabro-dev/src/commands/docs_options_reference.rs`:
   - TOML example (**:234-236**): change the `[llm.providers.proxy.extra_headers]`
     lines from `{ env = "PORTKEY_API_KEY" }` / `{ literal = "@bedrock-prod" }` /
     `{ vault = "gateway_team_secret" }` to the token form (real double braces):
     `x-portkey-api-key = "env.PORTKEY_API_KEY"`,
     `x-portkey-config = "@bedrock-prod"`,
     `x-team-secret = "secrets.gateway_team_secret"`.
   - `extra_headers` table row (**:249**): replace "Values must be typed refs:
     `{ literal = "..." }`, `{ env = "NAME" }`, or `{ vault = "NAME" }`." with:
     "Additional headers attached to provider requests. Values are interpolation
     strings: literal text, an `env.NAME` token, or a `secrets.NAME` token. Put
     credentials in a secret and reference them with a `secrets.NAME` token, not
     a bare literal."
   - Then run `cargo dev docs` to regenerate the fenced region in
     `user-configuration.mdx`, and `cargo dev docs check` to confirm it matches.
   - Leave the `base_url` row unchanged (DD-5).
2. **Hand-written** `docs/public/reference/user-configuration.mdx:96-98` (before
   the generated fence): change the two `{ env = .. }` / `{ literal = .. }`
   header lines to the token / literal form.
3. **Hand-written** `docs/public/integrations/openrouter.mdx:139-141`: change
   `"HTTP-Referer" = { literal = "https://your-site.example" }` /
   `"X-Title" = { literal = "Your App" }` to plain-string form
   (`"HTTP-Referer" = "https://your-site.example"`, `"X-Title" = "Your App"`).
4. **Hand-written** `docs/public/core-concepts/models.mdx:57-58`: change the
   `{ env = .. }` / `{ literal = .. }` header lines to the token / literal form.
5. **Add a new changelog entry** (do not edit the historical
   `2026-05-13.mdx`): note the breaking change plainly — provider
   `extra_headers` now use interpolation strings (literal text → plain text,
   `{ env = "X" }` → an `env.X` token, `{ vault = "X" }` → a `secrets.X` token,
   Token-only vault entries); multi-segment values like `Bearer <secret token>`
   now work; bare string header values are now accepted (were rejected), so put
   credentials in secrets and reference them with a `secrets.NAME` token, not a
   bare literal. If a `base_url` clarification is warranted anywhere, state in
   one sentence that `base_url` remains a plain literal (not interpolated) for
   now.

---

## Verify

Run the full gate; all must pass before done:

- `cargo +nightly-2026-04-14 fmt --check --all`
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`
- `cargo nextest run --workspace` (hermetic — strip ambient provider API keys;
  raise the FD limit with `ulimit -n 4096` on macOS if needed)
- `cargo dev docs check`
- `apps/fabro-web` + `lib/packages/fabro-api-client` typecheck (expected
  untouched — `extra_headers` is not on the wire and there is no OpenAPI change)
- Release build smoke: `cargo dev build -- -p fabro-cli --release`
- **Grep gate:** `rg -n "HeaderValueRef" lib/crates` returns **zero** hits.
  (`docs/superpowers/plans/...` may still mention it as historical plan text;
  scope the gate to `lib/crates`.)

---

## Dependencies & sequencing

- **No dependency on other unmerged interpolation PRs.** This builds on the
  already-merged foundation (`InterpString` v2, `ResolveCtx`, `resolve_with`,
  `vault_get_token`, the run-boundary secrets lookup) and the `fabro-auth`
  credential path as it exists on `main`.
- **New crate edge (optional-lint only):** `fabro-config` → `fabro-redact`. Not
  introduced unless the optional lint ships.
- **No `base_url` gate.** `base_url` is deferred, not blocked; this PR is fully
  self-contained.
- **Sequencing — land this before the interpolation-effort conformance
  registry.** The effort's final step adds a machine-readable registry of every
  interpolable field that a conformance test asserts against (single source of
  truth). Land this `extra_headers` migration **first**, so the registry records
  `extra_headers` as `InterpString` (its final type) rather than the
  soon-deleted `HeaderValueRef`. (Describe this in the PR in plain English; do
  not name internal phase identifiers.)
- **Not parallel-safe** with any other PR editing
  `fabro-model/src/catalog.rs`, `fabro-config/src/layers/llm.rs`, or the
  `fabro-auth` credential sources.


## Completed stages
- **toolchain**: succeeded
  - Script: `command -v cargo >/dev/null || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && sudo ln -sf $HOME/.cargo/bin/* /usr/local/bin/; }; cargo --version 2>&1`
  - Output:
    ```
    cargo 1.95.0 (f2d3ce0bd 2026-03-21)
    ```
- **preflight_compile**: succeeded
  - Script: `cargo check -q --workspace 2>&1`
  - Output: (empty)
- **preflight_lint**: succeeded
  - Script: `cargo +nightly-2026-04-14 clippy -q --workspace --all-targets -- -D warnings 2>&1`
  - Output: (empty)
- **implement**: succeeded
  - Model: gpt-5.5, 2.5m tokens in / 26.7k out
  - Files: /home/daytona/workspace/fabro/docs/public/changelog/2026-07-09.mdx
- **simplify_fable**: succeeded
  - Model: claude-fable-5, 122.2k tokens in / 36.9k out
  - Files: /home/daytona/workspace/fabro/lib/crates/fabro-auth/src/env_source.rs, /home/daytona/workspace/fabro/lib/crates/fabro-auth/src/lib.rs, /home/daytona/workspace/fabro/lib/crates/fabro-auth/src/resolve.rs, /home/daytona/workspace/fabro/lib/crates/fabro-auth/src/vault_ext.rs, /home/daytona/workspace/fabro/lib/crates/fabro-config/src/builders.rs, /home/daytona/workspace/fabro/lib/crates/fabro-model/src/catalog.rs, /home/daytona/workspace/fabro/lib/crates/fabro-workflow/src/operations/start.rs


# Simplify: Code Review and Cleanup

Review all changes for reuse, quality, and efficiency. Fix any issues found. Feel free to use any sub agents you need.

## Phase 1: Identify Changes

Run git diff (or git diff HEAD if there are staged changes) to see what changed. If there are no git changes, review the most recently modified files that the user mentioned or that you edited earlier in this conversation. (You may already have the changes in context, if so, feel free to skip this part)

## Phase 2: Launch Three Review Agents in Parallel

Use the Agent tool to launch all three agents concurrently in a single message. Pass each agent the full diff so it has the complete context.

### Agent 1: Code Reuse Review

For each change:

1. Search for existing utilities and helpers that could replace newly written code. Use Grep to find similar patterns elsewhere in the codebase — common locations are utility directories, shared modules, and files adjacent to the changed ones.
2. Flag any new function that duplicates existing functionality. Suggest the existing function to use instead.
3. Flag any inline logic that could use an existing utility — hand-rolled string manipulation, manual path handling, custom environment checks, ad-hoc type guards, and similar patterns are common candidates.

Note: This is a greenfield app, so focus on maximizing simplicity and don't worry about changing things to achieve it.

### Agent 2: Code Quality Review

Review the same changes for hacky patterns:

1. Redundant state: state that duplicates existing state, cached values that could be derived, observers/effects that could be direct calls
2. Parameter sprawl: adding new parameters to a function instead of generalizing or restructuring existing ones
3. Copy-paste with slight variation: near-duplicate code blocks that should be unified with a shared abstraction
4. Leaky abstractions: exposing internal details that should be encapsulated, or breaking existing abstraction boundaries
5. Stringly-typed code: using raw strings where constants, enums (string unions), or branded types already exist in the codebase

Note: This is a greenfield app, so be aggressive in optimizing quality.

### Agent 3: Efficiency Review

Review the same changes for efficiency:

1. Unnecessary work: redundant computations, repeated file reads, duplicate network/API calls, N+1 patterns
2. Missed concurrency: independent operations run sequentially when they could run in parallel
3. Hot-path bloat: new blocking work added to startup or per-request/per-render hot paths
4. Unnecessary existence checks: pre-checking file/resource existence before operating (TOCTOU anti-pattern) — operate directly and handle the error
5. Memory: unbounded data structures, missing cleanup, event listener leaks
6. Overly broad operations: reading entire files when only a portion is needed, loading all items when filtering for one

## Phase 3: Fix Issues

Wait for all three agents to complete. Aggregate their findings and fix each issue directly. If a finding is a false positive or not worth addressing, note it and move on — do not argue with the finding, just skip it.

When done, briefly summarize what was fixed (or confirm the code was already clean).
