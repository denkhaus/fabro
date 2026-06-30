Goal: # Implementation plan: MCP servers settings UI (`/settings/mcps`)

> **For the implement-plan workflow.** This is a self-contained spec for adding a
> web management UI for server-managed MCP servers. Implement **every** step.
> Use red/green TDD: write the failing `bun test` first, then the code.
>
> **Delivery note (for the human, not the agent):** the workflow's implement step
> reads "the plan file referenced in the goal," so this file must be visible to
> the cloned workspace — commit it at e.g. `docs/plans/mcp-settings-ui.md` on the
> branch the instance clones, or paste its contents into the run goal.

---

## 1. What this task is (and what is already done)

The **entire backend for server-managed MCP servers is already built, merged, and
live on `main`.** That includes the on-disk store, the OpenAPI spec, the HTTP CRUD
handlers with credential-omitting reads, and the workflow-side reference resolver
that lets a workflow enable a stored server by name. **None of that is in scope
here and none of it should be changed.**

This task is **frontend only**: a settings catalog page at `/settings/mcps` that
lets a user create, list, edit, and delete MCP server definitions through the
already-shipped HTTP API, following the exact patterns used by the existing
`/settings/environments` and `/settings/secrets` pages.

Everything you build lives under `apps/fabro-web/app/`. You consume the
**already-generated** TypeScript client `@qltysh/fabro-api-client` (the
`MCPServersApi` class and its models already exist — do not regenerate it).

---

## 2. Scope

**In scope (all under `apps/fabro-web/app/`):**

- A list page, a "new" page, and an "edit" page for MCP servers.
- A shared MCP form component with stdio / http / sandbox transport variants.
- Key/value editors for environment variables and HTTP headers.
- A credential-looking-value warning with a "use a secret instead" nudge
  (warn-but-allow — never block save).
- Query hooks + query keys + API client wiring.
- Route registration and (last) the settings nav link.
- `bun test` unit tests for every pure helper and a render smoke test per page.

**Out of scope — do NOT touch any of these (they are done or are future work):**

- ❌ Any Rust crate: `fabro-config`, `fabro-types`, `fabro-mcp-store`,
  `fabro-server`, `fabro-workflow`, `fabro-api`, etc. The resolver, store, and
  handlers are complete.
- ❌ The OpenAPI spec (`docs/public/api-reference/fabro-api.yaml`) and the
  generated clients. `MCPServersApi` is already generated and importable. Do not
  run `bun run generate`, do not edit `lib/packages/fabro-api-client`.
- ❌ Adding/altering API endpoints or request/response shapes. In particular, do
  **not** invent a PATCH/partial-update endpoint to work around the read model
  omitting env/header values (see §5) — that is deliberately future work.
- ❌ The reference-resolution behavior, the run-only `fabro exec` rejection, the
  reference TOML syntax, or anything about how runs consume the catalog.
- ❌ The SQLite migration of the store.
- ❌ Documentation (`mcp.mdx`) and end-to-end Rust tests — separate task.

If you find yourself editing a `.rs` file or `fabro-api.yaml`, stop — you have
left scope.

---

## 3. Locked decisions — do NOT relitigate

These are settled. Build to them; don't redesign them.

1. **Credentials: warn, don't block.** When an env-var or header looks
   credential-bearing, show a warning and offer a one-click path to store it as a
   secret and reference it with the secrets name rather than the value. **Still allow the user to
   save.** Do not hard-reject. Ordinary literals (`NODE_ENV=production`, ports,
   feature flags, non-sensitive headers) save without friction.
2. **Reads never expose secret values.** The API returns only env/header **names**
   (`env_keys` / `header_keys`), never values. The UI must never attempt to
   display a stored value and must not assume it can read one back.
3. **Settings-nested, not top-level.** This lives under `/settings/mcps`, mirroring
   `/settings/environments` and `/settings/secrets`. It is not a top-level nav
   destination.
4. **Optimistic concurrency via `revision` + `If-Match`.** Same as environments:
   capture `revision` from the read, pass it to replace/delete, surface a 409 as a
   "changed since you opened it — reload" message. No auto-retry.
5. **No form library, no `useEffect`.** Plain `useState` controlled inputs, pure
   validation functions, SWR for server state, `key={revision}` to reset the edit
   form. This is the house style and the repo enforces an effects policy
   (`docs/internal/react-effects-policy.md`).
6. **Transport type is chosen at create time and locked on edit** — mirroring how
   environments lock `provider` at creation. (You may revisit later; for v1, lock
   it. The id is likewise locked on edit.)

---

## 4. The API contract you consume

Already generated in `@qltysh/fabro-api-client`. Methods on `MCPServersApi`:

| Method | Signature | Returns |
|---|---|---|
| list | `listMcpServers()` | `McpServerListResponse` (`{ data: McpServer[], meta: { total } }`) |
| get | `retrieveMcpServer(id)` | `McpServer` |
| create | `createMcpServer(CreateMcpServerRequest)` | `McpServer` |
| replace | `replaceMcpServer(id, ifMatch, ReplaceMcpServerRequest)` | `McpServer` |
| delete | `deleteMcpServer(id, ifMatch)` | `void` |

The generated client automatically sets the `If-Match` header from the `ifMatch`
argument.

**Read model — `McpServer` (values omitted):**

```ts
interface McpServer {
  id: string;                  // stable id; also the runtime MCP server name
  revision: string;            // pass as If-Match on replace/delete
  display_name: string;
  description: string | null;
  transport: McpTransportView; // union by `type`; VALUES OMITTED
  startup_timeout_secs: number;
  tool_timeout_secs: number;
}

// McpTransportView = http | sandbox | stdio, each WITHOUT values:
//   stdio:   { type: 'stdio',   command: string[], env_keys: string[] }
//   http:    { type: 'http',    protocol?: McpHttpProtocol, url: string, header_keys: string[] }
//   sandbox: { type: 'sandbox', protocol?: McpHttpProtocol, command: string[], port: number, env_keys: string[] }
```

**Write model — `CreateMcpServerRequest` / `ReplaceMcpServerRequest` (values required):**

```ts
interface CreateMcpServerRequest {
  id: string;                  // create only; on replace the path id is authoritative
  display_name: string;
  description?: string | null;
  transport: McpTransport;     // union by `type`; VALUES INCLUDED
  startup_timeout_secs: number;
  tool_timeout_secs: number;
}
// ReplaceMcpServerRequest is the same minus `id`.

// McpTransport = http | sandbox | stdio, each WITH values:
//   stdio:   { type: 'stdio',   command: string[], env: Record<string,string> }
//   http:    { type: 'http',    protocol?: McpHttpProtocol, url: string, headers: Record<string,string> }
//   sandbox: { type: 'sandbox', protocol?: McpHttpProtocol, command: string[], port: number, env: Record<string,string> }

// McpHttpProtocol = 'streamable_http' | 'sse'  (default streamable_http; only on http/sandbox)
```

`replace` is a **full PUT** (whole transport replaced). There is no merge/preserve.

---

## 5. The one genuinely tricky UX decision: editing when values are write-only

Because reads return only `env_keys`/`header_keys` and replace is a full PUT, **the
edit form cannot read back env/header values, and saving overwrites the entire
transport.** Handle it explicitly — do not try to fix it server-side:

- On the **edit** form, pre-populate env/header rows with the existing **keys** and
  **empty values**.
- Show a clear banner on the edit form whenever the transport has any
  `env_keys`/`header_keys`:
  > "Existing environment variable and header values are write-only and are not
  > shown. Saving replaces the full set — re-enter every value you want to keep."
- On the edit form only, **block save** (with an inline row error) if any row has a
  non-empty key but an empty value, so the user can't silently blank out values.
  (On the create form, empty values are simply allowed/optional per the API.)

This keeps the destructive-overwrite behavior visible and intentional.

---

## 6. Reference implementation to copy from

Mirror these existing files closely (same structure, naming, classes, helpers):

- Routes: `apps/fabro-web/app/router.tsx` (environments entries, ~lines 147–166)
- Nav: `apps/fabro-web/app/routes/settings.tsx` (`navSections`, `NavItem`/`NavSection`)
- List: `apps/fabro-web/app/routes/settings-environments.tsx`
- New: `apps/fabro-web/app/routes/settings-environments-new.tsx`
- Edit: `apps/fabro-web/app/routes/settings-environments-edit.tsx`
- Form + key/value editor: `apps/fabro-web/app/components/environment-form.tsx`
  (`EnvironmentFormFields`, `KeyValueEditor`, `entriesFromMap`/`mapFromEntries`)
- Panels: `apps/fabro-web/app/components/settings-panel.tsx`
  (`Panel`, `Row`, `SettingsPageIntro`, `PanelSkeleton`, `Badge`, `Muted`)
- Query keys: `apps/fabro-web/app/lib/query-keys.ts` (`environments.{list,detail}`)
- Query hooks: `apps/fabro-web/app/lib/queries.ts` (`useEnvironments`, `useEnvironment`)
- API client: `apps/fabro-web/app/lib/api-client.ts` (`environmentsApi`, `apiData`, `apiNullableData`, `ApiError`)
- Secrets create flow (for the nudge target): `apps/fabro-web/app/routes/settings-secrets-new.tsx` (reads `?name=`)
- Test convention: `apps/fabro-web/app/routes/settings-integrations.test.tsx`
  (`bun:test`, `react-test-renderer`, `setupReactTestEnv`, `mock.module("../lib/queries", ...)`)

---

## 7. Implementation steps (TDD; do them in this order)

Each step: write the failing test(s) first where a test is called for, then
implement until green. Run `cd apps/fabro-web && bun test` and `bun run typecheck`
frequently.

### Step 1 — Extract the shared key/value editor

`KeyValueEditor`, the `KeyValueEntry` type, and `entriesFromMap` / `mapFromEntries`
currently live inside `components/environment-form.tsx`. Move them verbatim (no
behavior change) into a new `apps/fabro-web/app/components/key-value-editor.tsx`
and re-import them in `environment-form.tsx`. Both the MCP form and the environment
form then share one implementation.

- Test: `components/key-value-editor.test.tsx` — `mapFromEntries` drops blank keys
  and trims, `entriesFromMap` round-trips; a render test adds/removes a row.
- Verify environments still typecheck and any existing environment tests pass.

### Step 2 — Query keys

In `lib/query-keys.ts`, add alongside `environments`:

```ts
mcpServers: {
  list: () => ["mcp-servers", "list"] as const,
  detail: (id: string) => ["mcp-servers", "detail", id] as const,
},
```

### Step 3 — API client wiring

In `lib/api-client.ts`, import `MCPServersApi` from `@qltysh/fabro-api-client` and
instantiate it exactly like `environmentsApi`:

```ts
export const mcpServersApi = new MCPServersApi(
  generatedApiConfiguration,
  "",
  generatedAxios,
);
```

### Step 4 — Query hooks

In `lib/queries.ts`, mirror the environments hooks:

```ts
export function useMcpServers() {
  return useSWR<McpServerListResponse>(
    queryKeys.mcpServers.list(),
    () => apiData(() => mcpServersApi.listMcpServers()),
  );
}

export function useMcpServer(id: string | undefined) {
  return useSWR<McpServer | null>(
    id ? queryKeys.mcpServers.detail(id) : null,
    id ? () => apiNullableData(() => mcpServersApi.retrieveMcpServer(id)) : null,
  );
}
```

### Step 5 — Credential detection helper (pure, TDD)

New `apps/fabro-web/app/lib/credential-heuristics.ts`. Pure functions, fully unit
tested first.

```ts
// True when a key/value pair looks credential-bearing and should be nudged
// toward a secret. Never flags an already-templated value.
export function looksLikeCredential(key: string, value: string): boolean;

// Suggest a secret name derived from the key (UPPER_SNAKE_CASE, alnum + _).
export function secretNameForKey(key: string): string;

// The interpolation reference to store in place of a literal secret value.
export function secretReference(name: string): string; // `\{\{ secrets.NAME \}\}`
```

Rules for `looksLikeCredential`:
- Return **false** if `value` is empty, or already a template
  (`/\{\{\s*(secrets|env|vars)\./` matches) — references are fine as-is.
- Return **true** if the key (case-insensitive) matches any of:
  `authorization`, `password`, `passwd`, `secret`, `token`, `api[-_]?key`,
  or ends in `_key` / `_token` / `_secret`.
- Otherwise return **true** if the value looks high-entropy: length ≥ 20, contains
  a mix of at least two of {lowercase, uppercase, digit}, and has no spaces.
- Otherwise **false**.

`secretNameForKey`: uppercase, replace non-`[A-Za-z0-9]` with `_`, collapse
repeats, trim leading/trailing `_`; if empty, fall back to `SECRET`.

Tests must cover: `Authorization`/`Bearer abc...` → flagged; `API_KEY` → flagged;
`NODE_ENV`/`production` → not flagged; a `\{\{ secrets.X \}\}` value → not flagged;
empty value → not flagged; a long random-looking value under a benign key →
flagged; `secretNameForKey("x-api-key") === "X_API_KEY"`.

### Step 6 — Form model: types, mappers, validation (pure, TDD)

New `apps/fabro-web/app/components/mcp-server-form.tsx` (form component) plus a
small co-located or `lib/`-level module of pure helpers (tested first):

Form value shape (flat, transport-discriminated):

```ts
export type McpTransportKind = "stdio" | "http" | "sandbox";

export interface McpServerFormValues {
  id: string;
  displayName: string;
  description: string;
  startupTimeoutSecs: number;
  toolTimeoutSecs: number;
  transport: McpTransportKind;
  // stdio + sandbox
  command: string;            // shell-style words; split on whitespace -> string[]
  // http + sandbox
  protocol: McpHttpProtocol;  // default 'streamable_http'
  // http
  url: string;
  headers: KeyValueEntry[];
  // sandbox
  port: number;
  // stdio + sandbox env
  env: KeyValueEntry[];
}
```

Pure helpers + tests:

- `defaultMcpServerFormValues(kind): McpServerFormValues` — sensible defaults
  (timeouts mirror `McpServerSettings::default()`: startup 10, tool 60).
- `mcpServerToFormValues(server: McpServer): McpServerFormValues` — for edit.
  Maps `transport` **view** into the form. **Env/header values are not available**,
  so populate `env`/`headers` entries from `env_keys`/`header_keys` with **empty
  values** (see §5).
- `createRequestFromForm(values): CreateMcpServerRequest` and
  `replaceRequestFromForm(values): ReplaceMcpServerRequest` — build the
  `McpTransport` (values included), splitting `command` on whitespace, converting
  entry arrays via `mapFromEntries`, omitting `protocol` when default.
- `isMcpServerFormValid(values, { isEdit }): boolean` — id matches
  `/^[a-z0-9][a-z0-9-]{0,62}$/` (create only), `displayName` non-empty, and per
  transport: stdio needs `command`; http needs `url`; sandbox needs `command` and a
  valid `port` (1–65535). On edit, additionally require a value for every row with
  a non-empty key (the §5 guard).
- `credentialWarnings(values): { field: 'env'|'headers'; index: number }[]` —
  derived during render from `looksLikeCredential`, used to render per-row warnings.

Round-trip test: `mcpServerToFormValues` → `replaceRequestFromForm` preserves id,
display name, transport type, url/command/port; and (documented) drops values that
weren't re-entered.

### Step 7 — The form component

`McpServerFormFields({ values, onChange, lockId, lockTransport })` in
`components/mcp-server-form.tsx`, built from `Panel` / `Row` and a local `patch()`
helper (copy the environment-form idiom):

- **General panel:** `id` (locked on edit), `display_name`, `description`,
  transport `<select>` (locked on edit), startup/tool timeout numeric inputs.
- **Transport panel** — render fields by `values.transport` using the discriminated
  conditional idiom (see `image_source` switching in `environment-form.tsx`):
  - `stdio`: `command` text input; env `KeyValueEditor`.
  - `http`: `protocol` select (`streamable_http`/`sse`), `url` text input; headers
    `KeyValueEditor` (key placeholder `Authorization`, value `Bearer token`).
  - `sandbox`: `protocol` select, `command` text input, `port` numeric input; env
    `KeyValueEditor`.
- **Credential nudge:** for each `KeyValueEditor` row flagged by
  `looksLikeCredential`, render an inline warning (`text-amber`/warning tone) with a
  "Store as secret" action. The action: set that row's value to
  `secretReference(secretNameForKey(key))` and open
  `/settings/secrets/new?name=<secretNameForKey(key)>` in a new tab (the user
  finishes creating the secret there). This is warn-but-allow: saving with the
  literal still works.

Keep markup, class names, and helper conventions identical to the environment form
so it reads as the same codebase.

### Step 8 — List page

`apps/fabro-web/app/routes/settings-mcps.tsx`, mirroring
`settings-environments.tsx`:

- `meta()` title.
- Default component calls `useMcpServers()`, renders `SettingsPageIntro`
  (description + a "New MCP server" control), and a panel listing servers
  (display name, id, transport type badge, a row menu with Edit / Delete).
- A "New MCP server" dropdown offering stdio / http / sandbox →
  `/settings/mcps/new?type=<kind>` (mirror the environments provider dropdown).
- Loading → `PanelSkeleton`; error → error panel; empty → friendly empty state.
- **Delete:** confirm, then `apiData(() => mcpServersApi.deleteMcpServer(id, revision))`,
  then `mutate(queryKeys.mcpServers.list())`, then success toast. 409 → toast
  telling the user it changed; refresh the list.

### Step 9 — New page

`apps/fabro-web/app/routes/settings-mcps-new.tsx`, mirroring
`settings-environments-new.tsx`:

- Breadcrumb + `McpServerFormFields` (no locks).
- Initial transport from `?type=` (`useSearchParams`), default `stdio`.
- Submit: `createRequestFromForm`, `apiData(() => mcpServersApi.createMcpServer(req))`,
  `mutate(list)`, success toast, navigate to `/settings/mcps`.
- Errors via `ErrorMessage`; submit disabled unless `isMcpServerFormValid(values,{isEdit:false})`.

### Step 10 — Edit page

`apps/fabro-web/app/routes/settings-mcps-edit.tsx`, mirroring
`settings-environments-edit.tsx`:

- Read `:id` via `useParams`, fetch with `useMcpServer(id)`.
- Render `<McpServerFormFields lockId lockTransport ... key={server.revision} />`
  (the `key` remounts on revision change — copy this exactly).
- Initialize from `mcpServerToFormValues(server)`.
- Render the §5 write-only-values banner when keys exist.
- Submit: `replaceRequestFromForm`, `apiData(() => mcpServersApi.replaceMcpServer(id, server.revision, req))`,
  `mutate(list)` + `mutate(detail(id))`, success toast, navigate back.
- 409: copy the environments `staleAwareMessage` pattern — show "this MCP server
  changed since you opened it; reload and reapply," do not auto-retry.

### Step 11 — Route registration

In `app/router.tsx`, import the three modules and add under the `settings` children
(next to the environments entries):

```ts
route("mcps", SettingsMcps),
route("mcps/new", SettingsMcpsNew),
route("mcps/:id/edit", SettingsMcpsEdit),
```

### Step 12 — Nav link (LAST)

Only after steps 1–11 are green, add a `NavItem` to `navSections` in
`routes/settings.tsx`, in the same section as Environments:

```ts
{
  name: "MCP servers",
  href: "/settings/mcps",
  icon: PuzzlePieceIcon, // or another existing @heroicons/react/24/outline icon
  description: "Server-managed MCP servers you can enable by name in workflows.",
  match: (p) => p.startsWith("/settings/mcps"),
},
```

---

## 8. Tests to write (`bun test`, from `apps/fabro-web`)

Follow `settings-integrations.test.tsx` conventions (`bun:test`,
`react-test-renderer` + `act`, `setupReactTestEnv`, `mock.module("../lib/queries",
...)` to feed fixture data).

Required:

1. `lib/credential-heuristics.test.ts` — all rules in §5/Step 5 (this is the
   highest-value, purely-logical test surface).
2. `components/key-value-editor.test.tsx` — map round-trip + add/remove row.
3. `components/mcp-server-form.test.tsx` (or a `lib` test) — `mcpServerToFormValues`
   populates keys with empty values; `createRequestFromForm`/`replaceRequestFromForm`
   build correct discriminated transports; `isMcpServerFormValid` accepts/rejects
   per transport and enforces the edit value-required guard.
4. `routes/settings-mcps.test.tsx` — list renders rows from mocked
   `useMcpServers`; empty state renders.
5. `routes/settings-mcps-edit.test.tsx` — with a mocked `useMcpServer` returning a
   transport that has `env_keys`, the write-only banner renders and env rows show
   the keys with empty values.

---

## 9. Definition of done

- `cd apps/fabro-web && bun run typecheck` — clean.
- `cd apps/fabro-web && bun test` — all new + existing tests pass.
- `cd apps/fabro-web && bun run build` — production build succeeds and stays within
  the SPA asset budgets (the workflow's `cargo dev build` re-checks these; keep the
  pages lightweight, reuse existing components, add no new heavy dependencies).
- `cd lib/packages/fabro-api-client && bun run typecheck` — clean (you only consume
  it; don't change it).
- No Rust files changed; `cargo` steps in the verify gate pass unchanged.
- Create → list → edit → delete all work against a live server; the edit form never
  displays a stored secret value; credential-looking values warn but still save.
- The nav link appears and routes correctly.

---

## 10. Known rough edges (acknowledge; don't "fix" out of scope)

- **Edit requires re-entering env/header values** — a direct, intended consequence
  of the credential-omitting read model + full-PUT replace. Handle it per §5; do
  not add a server-side partial-update endpoint here.
- **Credential heuristic false positives/negatives** — it's a *nudge*, not a gate.
  Keep it cheap and explainable; never block save on it.
- **Workflow-fit note (for the human):** the implement-plan workflow's preflight is
  Rust-centric, but its verify gate runs `bun run typecheck` and `bun run test` for
  `fabro-web`, plus `cargo dev build` (which runs the SPA production build + asset
  budget check). This frontend-only change passes the Rust preflight untouched and
  is genuinely enforced by the TypeScript portions of verify.


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
  - Model: gpt-5.5, 1.7m tokens in / 37.7k out
  - Files: /home/daytona/workspace/fabro/apps/fabro-web/app/components/key-value-editor.test.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/components/key-value-editor.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/components/mcp-server-form.test.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/components/mcp-server-form.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/lib/credential-heuristics.test.ts, /home/daytona/workspace/fabro/apps/fabro-web/app/lib/credential-heuristics.ts, /home/daytona/workspace/fabro/apps/fabro-web/app/routes/settings-mcps-edit.test.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/routes/settings-mcps-edit.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/routes/settings-mcps-new.test.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/routes/settings-mcps-new.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/routes/settings-mcps.test.tsx, /home/daytona/workspace/fabro/apps/fabro-web/app/routes/settings-mcps.tsx


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
