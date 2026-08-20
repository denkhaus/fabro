# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and test commands

### Rust
- `cargo build --workspace` — build all crates
- `cargo nextest run --workspace` — run all unit tests
- `cargo nextest run -p fabro-server` — test a single crate
- `cargo nextest run -p fabro-workflow -- test_name` — run a single test
- `set -a && source .env && set +a && cargo nextest run --workspace --profile e2e --run-ignored only` — run all E2E live tests (requires credentials in `.env`, see `.env.example`)
- `set -a && source .env && set +a && cargo nextest run -p fabro-llm --profile e2e --run-ignored only` — run E2E tests for a single crate
- `cargo +nightly-2026-04-14 fmt --check --all` — check formatting (pinned nightly required for rustfmt config; CI uses the same date)
- `cargo +nightly-2026-04-14 fmt --all` — auto-format
- `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings` — lint (CI runs nightly clippy to match; install with `rustup toolchain install nightly-2026-04-14 --profile minimal --component clippy,rustfmt`)

macOS note: if `cargo nextest run` fails with `Too many open files (os error 24)` / `EMFILE`, raise the shell's soft FD limit before running tests, for example `ulimit -n 4096 && cargo nextest run --workspace`. Some terminals and inherited agent sessions start with `ulimit -n 256`, which is too low for the shared CLI test daemon under parallel nextest load.

### TypeScript (fabro-web)
- `cd apps/fabro-web && bun run dev` — rebuild web assets on change for the Rust server; refresh the browser manually
- `cd apps/fabro-web && bun test` — run tests
- `cd apps/fabro-web && bun run typecheck` — type check
- `cd apps/fabro-web && bun run build` — production build (writes to `apps/fabro-web/dist/` only; does NOT update the bundled SPA that ships in the Rust binary)
- `cargo dev build [-- <cargo args>]` — refreshes the embedded SPA assets from the production build, verifies SPA asset budgets, and then runs `cargo build` with forwarded args. The embedded assets are gitignored except for `.gitkeep`; use this when building a Rust binary that should include a populated SPA bundle. `bun run dev` for local development is unchanged because debug builds prefer `apps/fabro-web/dist/` on disk via the server fallback.

### Docker image
- `cargo dev docker-build` — builds the local Docker image from the current tree using the release pipeline's cargo-zigbuild approach. Honors `--arch amd64|arm64`, `--tag <name>` (default `fabro-sh/fabro`), `--compile-only` (stages `tmp/docker-context/<arch>/fabro` without `docker build`), and `--dry-run` (prints the Docker commands without running them). Prefer this over writing a throwaway Dockerfile; the release pipeline, `Dockerfile`, and this command share the same binary layout.

### Docker sandbox provider
- Docker is the default runtime sandbox provider from `defaults.toml`. The Fabro process must have a working Docker client environment (`DOCKER_HOST`, socket access, Docker Desktop behavior, TLS settings, groups/permissions, and any remote daemon policy are operator responsibilities).
- The packaged compose service mounts `/var/run/docker.sock` so the server can create sibling run containers on the host daemon. This is host-root-equivalent under Docker's security model; only use it in the trusted, single-tenant deployment model described by the sandbox code/docs.
- Docker and Daytona are clone-based providers. When a run manifest has a GitHub origin, they clone it into the provider workspace. Present non-GitHub origins fail unless the provider has `skip_clone = true`; absent origins or `skip_clone = true` create an empty workspace without repository files.

### Release automation
- `cargo dev release` — creates the next stable release tag. Use `cargo dev release --nightly` for a nightly prerelease. Use `--dry-run` to print planned commands without mutating git or running Cargo, `--skip-tests` only after running the release-mode smoke yourself, and `--release-date YYYY-MM-DD` or `FABRO_RELEASE_DATE` for deterministic version computation.

### Marketing site (apps/marketing)
- `cd apps/marketing && bun run dev` — start Astro dev server
- `cd apps/marketing && bun run build` — production build
- `cd apps/marketing && bunx vercel --prod` — deploy to Vercel (project: website, domain: fabro.sh)

### Dev servers
1. `fabro server start` — starts the Rust API server (demo mode is per-request via `X-Fabro-Demo: 1` header)
2. `cd apps/fabro-web && bun run dev` — rebuilds web assets on change; refresh the browser manually
3. Mintlify docs dev server (requires Docker — `mintlify dev` needs Node LTS which may not match the host):
   ```
   docker run --rm -d -p 3333:3333 -v $(pwd)/docs/public:/docs -w /docs --name mintlify-dev node:22-slim \
     bash -c "npx mintlify dev --host 0.0.0.0 --port 3333"
   ```
   Then open http://localhost:3333. Stop with `docker stop mintlify-dev`.

## API workflow

The OpenAPI spec at `docs/public/api-reference/fabro-api.yaml` is the source of truth for the fabro-api HTTP interface.

1. Edit `docs/public/api-reference/fabro-api.yaml`
2. `cargo build -p fabro-api` — build.rs regenerates Rust types and client via progenitor
3. Write/update handler in `lib/apps/fabro-server/src/server.rs`, add route to `build_router()`
4. `cargo nextest run -p fabro-server` — conformance test catches spec/router drift
5. `cd lib/packages/fabro-api-client && bun run generate` — regenerates TypeScript Axios client

### API type ownership

- Treat OpenAPI as the source of truth for the wire contract, not as the automatic owner of Rust types.
- Before adding or keeping a generated schema type, search the workspace for an existing hand-written Rust type with the same product meaning.
- If the schema and an existing Rust type have the same semantics and serde shape, reuse the existing type via `lib/foundation/fabro-api/build.rs` `with_replacement(...)` instead of generating a parallel API type.
- If two types are close but not identical, prefer proposing changes that align them into one canonical type rather than accepting small drift. It is usually better to iterate the API now than to create permanently split Rust/API types.
- Keep a separate API DTO only when the API is intentionally a projection, summary, or presentation-specific view of internal state. In that case, give it a distinct API-facing name instead of reusing the internal concept name.
- Treat `ApiFoo` aliases and `foo_to_api` / `foo_from_api` adapters as a smell unless they represent a real semantic boundary. They should not exist only to bridge accidental duplicate types.
- If a type is shared across crates and is part of the core product vocabulary, move it to a shared crate first, then make `fabro-api` reuse it.
- For every new `with_replacement(...)`, add a `fabro-api` test that proves type identity and JSON parity with the OpenAPI schema.

## Test support boundaries

Test-only helpers, fixture constructors, fake credentials, in-memory stores, panic-heavy setup code, and test environment shims must not be exposed from production modules or linked into normal builds.

Put shared test helpers in a dedicated `test_support` module gated behind tests or an explicit feature:

```rust
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
```

If another crate's tests need those helpers, enable the feature only through a dev-dependency using Cargo's dual-listing pattern:

```toml
[dependencies]
fabro-server = { path = "../fabro-server" }

[dev-dependencies]
fabro-server = { path = "../fabro-server", features = ["test-support"] }
```

Do not enable `test-support` in default features, production dependencies, release builds, or binaries.

Use names that make the boundary obvious: `test_app_state`, `test_store_bundle`, `test_auth_mode`, and similar. Avoid production-looking names such as `create_app_state` for test fixtures. `#[doc(hidden)]` is not a substitute for feature-gating; hidden public APIs still compile, link, and can be used accidentally.

Before merging changes that add or move shared test helpers, verify:

- `cargo build --workspace` succeeds without `test-support`
- relevant tests compile and run with `test-support`
- `rg -n "create_app_state|test-only-name"` does not show production call sites
- release/debug artifacts do not contain fake secrets, fixture tokens, or test helper symbols when built without `test-support`

## Architecture

Fabro is an AI-powered workflow orchestration platform. Workflows are defined as Graphviz graphs, where each node is a stage (agent, prompt, command, conditional, human, parallel, etc.) executed by the workflow engine.

### Rust crates (`lib/apps/`, `lib/components/`, and `lib/foundation/`)
- **fabro-cli** — CLI entry point. Commands: `run`, `exec`, `serve`, `validate`, `parse`, `cp`, `model`, `doctor`, `install`, `ps`, `system prune`
- **fabro-workflow** — Core workflow engine. Parses Graphviz graphs, runs stages, manages checkpoints/resume, hooks, and human-in-the-loop interactions
- **fabro-agent** — AI coding agent with tool use (Bash, Read, Write, Edit, Glob, Grep, WebFetch). `Sandbox` trait abstracts execution environments
- **fabro-sandbox** — Local, Docker, and Daytona sandbox providers. Docker is the default runtime provider and creates clone-based `/workspace` containers through the operator's Docker daemon; Daytona uses the same GitHub-only clone-source contract. Docker daemon access is host-root-equivalent and assumes trusted callers/payloads.
- **fabro-server** — Axum HTTP server. Routes for runs, sessions, models, completions, usage. SSE event streaming. Demo mode via header
- **fabro-llm** — Unified LLM client with providers: Anthropic, OpenAI, Gemini, OpenAI-compatible, plus retry/middleware/streaming
- **fabro-api** — Auto-generated Rust types and reqwest HTTP client from OpenAPI spec (build.rs + progenitor)
- **fabro-github** — GitHub App auth (JWT signing, installation tokens, PR creation)
- **fabro-mcp** — Model Context Protocol client/server
- **fabro-slack** — Slack integration (socket mode, blocks API)
- **fabro-checkpoint** — Git-based checkpoint storage with branch store and metadata branches
- **fabro-telemetry** — CLI analytics (Segment) and crash reporting (Sentry), with anonymous IDs, command sanitization, and detached subprocess delivery
- **fabro-util** — Shared utilities (redaction, terminal formatting)

### TypeScript (`apps/` and `lib/packages/`)
- **apps/fabro-web** — React 19 + React Router + Tailwind CSS frontend, bundled by a custom Bun script (`apps/fabro-web/scripts/build.ts`), not Vite
- **lib/packages/fabro-api-client** — Auto-generated TypeScript Axios client from OpenAPI spec

### Key design patterns
- **Sandbox trait** — Uniform interface for local, Docker, and Daytona execution environments. Clone-based providers use run-spec GitHub origin metadata rather than worker process cwd detection.
- **Graphviz graph workflows** — Stages and transitions defined as Graphviz graph attributes
- **OpenAPI-first** — `fabro-api.yaml` drives Rust type + client generation (progenitor) and TypeScript client generation (openapi-generator)
- **Checkpoint/resume** — Workflows can be paused, checkpointed, and resumed

## Strategy docs

When working in an area covered by a strategy doc, read the relevant document
**before** making changes:

- **`docs/internal/logging-strategy.md`** — read when adding `tracing` calls (`info!`, `debug!`, `warn!`, `error!`), working on error handling paths, or adding new operations that should be observable
- **`docs/internal/events-strategy.md`** — read when adding or modifying `Event` variants, touching `Emitter`/`emit()`, changing `progress.jsonl` output, or adding new workflow stage types
- **`docs/internal/testing-strategy.md`** — read when adding or reorganizing tests, choosing between unit vs `tests/it`, deciding whether a test belongs in `cmd` vs `workflow` vs `scenario`, or deciding how to structure snapshots and fixtures
- **`docs/internal/server-secrets-strategy.md`** — read when adding or changing server-level secrets, startup validation, install-time secret persistence, or subprocess env inheritance/scrubbing
- **`docs/internal/migrations-strategy.md`** — read when adding or changing temporary compatibility migrations, startup/file rewrites, migration runners, backups, or removal deadlines
- **`docs/internal/error-handling-strategy.md`** — read when changing error types, using `anyhow`/`thiserror`, adding `.map_err(...)`, converting errors to `String`, changing API error responses, or touching CLI/miette/log/telemetry error rendering
- **`docs/internal/react-effects-policy.md`** — read when adding or refactoring React effects in `apps/fabro-web`; direct `useEffect` calls should be avoided in component code

## Shell quoting in sandbox code

When interpolating values into shell command strings (in `fabro-workflow`), always use the `shell_quote()` helper (backed by `shlex::try_quote`). Never use manual `replace('\'', "'\\''")` or unquoted interpolation. This applies to file paths, branch names, URLs, env vars, image names, glob patterns, and any other user-controlled input assembled into a shell script.

## Rust import style

- **Types** (structs, enums, traits): import by name — `use crate::outcome::Outcome;`
- **Functions**: import the parent module, call as `module::function()` — `use fabro_workflow::operations; operations::create(...)`
- **No glob imports** in production code (`use foo::*`). Globs are acceptable in test modules and preludes. Enforced by clippy `wildcard_imports` lint.

## Enum string/int conversions (strum)

For any enum where a variant maps to a fixed string or integer, derive it with `strum` instead of hand-writing `impl Display`, `impl FromStr`, `as_str()`, `fn all()`, or `const ALL: &[Self]`. Hand-written variant→string maps drift across the three impls on every rename.

- `strum::Display` replaces hand-written `impl fmt::Display` whose body is a match over string literals.
- `strum::EnumString` replaces hand-written `impl FromStr`. The `Err` type becomes `strum::ParseError` — adjust callers that assumed `Err = String`.
- `strum::IntoStaticStr` replaces `impl From<E> for &'static str`. When an existing `as_str(self) -> &'static str` is on the public API, keep it as a one-line wrapper: `pub fn as_str(self) -> &'static str { self.into() }`.
- `strum::EnumIter`, `strum::VariantArray`, `strum::VariantNames` replace hand-written `fn all()` / `const ALL` / `&[&'static str]` arrays. Do NOT use these if the hand-written list intentionally excludes variants (e.g. `Provider::ALL` skips `OpenAiCompatible`).
- `strum::FromRepr` replaces `fn from_u8`/`from_i32`. Note: it returns `Option<Self>`, so don't adopt it when the existing conversion has a `_ => default` fallback — that's a behavior change, not a cleanup.

Align strum with serde. When the enum also derives `Serialize`/`Deserialize` with `#[serde(rename_all = "...")]`, add the matching `#[strum(serialize_all = "...")]`. For variant aliases, use `#[strum(to_string = "canonical", serialize = "alias")]` — strum picks the last `serialize` for `Display`/`IntoStaticStr` otherwise, so `to_string` is needed to pin the canonical form.

Skip strum when parsing is fuzzy (URL/path detection, structured IDs, multi-token formats), when a variant carries a `String` catch-all, or when `Display` does dynamic formatting.

## Snapshot tests (insta)

Many CLI tests use `insta` inline snapshots. When a snapshot needs updating:

1. Run `cargo insta pending-snapshots` to list what changed
2. Verify each pending snapshot is expected
3. Run `cargo insta accept` to accept all, or `cargo insta accept --snapshot <path>` for a specific one

Never run `cargo insta accept` without first checking what's pending — it accepts *all* pending snapshots, which may include unrelated changes.

## Testing workflows

- `fabro run <name>` — run a workflow by name (resolves `.fabro/workflows/<name>/workflow.toml`), e.g. `fabro run repl`
- `#[e2e_test(twin, live("VAR"))]` — dual-mode test that runs against twin-openai or real API. `#[e2e_test(twin)]` for twin-only tests (e.g., scripted failures). `#[e2e_test(live("VAR"))]` for live-only tests requiring secrets. `#[e2e_test()]` for sandbox tests with no API deps. Behavior is controlled by `FABRO_TEST_MODE` (`live`, `strict`; default is `twin`), and `cargo nextest run --profile e2e ...` implies `strict`. Use `fabro_test::e2e_openai!()` in twin/dual-mode tests to get `(base_url, api_key)`.
- Local test HTTP clients must use `.no_proxy()`. Prefer shared helpers like `fabro_test::test_http_client()` or crate-local equivalents instead of `reqwest::Client::new()`, bare `Client::builder().build()`, or `reqwest::get(...)`.
- This is not cosmetic: macOS proxy discovery adds hidden startup overhead to repeated localhost reqwest clients and can surface as misleading nextest timeouts.

<!-- seeds:start -->
## Issue Tracking (Seeds)
<!-- seeds-onboard:v0.5.15 -->
<!-- seeds-onboard-schema:7 -->

This project uses [Seeds](https://github.com/jayminwest/seeds) v0.5.15 for git-native issue tracking.

**At the start of every session**, run:
```
sd prime
```

This injects session context: rules, command reference, and workflows. Pass `--format json|compact|markdown|plain|ids` on any command for agent-friendly output.

**Quick reference:**
- `sd ready` — Find unblocked work
- `sd search <query>` — Full-text search across titles + descriptions
- `sd create --title "..." --type task --priority 2` — Create issue
- `sd update <id> --status in_progress` — Claim work
- `sd close <id>` — Complete work
- `sd dep add <id> <depends-on>` — Add dependency between issues
- `sd sync` — Sync with git (run before pushing)

### Planning
Use `sd plan` when work is large or ambiguous enough that an LLM benefits from structured decomposition. Submit spawns one child seed per step; `step.blocks` uses forward semantics (step i with `blocks: [j]` means step i blocks step j, and step j gets step i's id in its `blockedBy`).

- `sd plan templates` — List built-ins (`feature`, `bug`, `refactor`) plus custom templates
- `sd plan prompt <seed-id>` — Emit a structured prompt the LLM fills in
- `sd plan submit <seed-id> --plan <file>` — Validate + spawn child seeds
- `sd plan show <pl-id>` — View sections, children, sub-plans
- `sd plan edit <id> [--name | --section <name> <text> | --step <i> --title/--priority/--type]` — In-place field edits; bumps revision
- `sd plan outcome <pl-id> --result success|partial|failure` — Record outcome (storage-only)
- `sd plan review <pl-id> --by <name>` — Record reviewer (informational)

### Before You Finish
1. Close completed issues: `sd close <id>`
2. File issues for remaining work: `sd create --title "..."`
3. Sync and push: `sd sync && git push`
<!-- seeds:end -->

<!-- mulch:start -->
## Project Expertise (Mulch)
<!-- mulch-onboard:v0.10.7 -->

This project uses [Mulch](https://github.com/jayminwest/mulch) v0.10.7 for structured expertise management.

**At the start of every session**, run:
```bash
ml prime
```

Injects project-specific conventions, patterns, decisions, failures, references, and guides into
your context. Run `ml prime --files src/foo.ts` before editing a file to load only records
relevant to that path (per-file framing, classification age, and confirmation scores included).

For monolith projects where dumping every record wastes context, set
`prime.default_mode: manifest` in `.mulch/mulch.config.yaml` (or pass `--manifest`) to emit a
quick reference + domain index. Agents then scope-load with `ml prime <domain>` or
`ml prime --files <path>`.

**Before completing your task**, record insights worth preserving — conventions discovered,
patterns applied, failures encountered, or decisions made:
```bash
ml record <domain> --type <convention|pattern|failure|decision|reference|guide> --description "..."
```

Evidence auto-populates from git (current commit + changed files). Link explicitly with
`--evidence-seeds <id>` / `--evidence-gh <id>` / `--evidence-linear <id>` / `--evidence-bead <id>`,
`--evidence-commit <sha>`, or `--relates-to <mx-id>`. Upserts of named records merge outcomes
instead of replacing them; validation failures print a copy-paste retry hint with missing fields
pre-filled.

Run `ml status` for domain health, `ml doctor` to check record integrity (add `--fix` to strip
broken file anchors), `ml --help` for the full command list. Write commands use file locking and
atomic writes, so multiple agents can record concurrently. Expertise survives `git worktree`
cleanup — `.mulch/` resolves to the main repo.

`ml prune` soft-archives stale records to `.mulch/archive/` instead of deleting them; pass
`--hard` for true deletion. Restore an archived record with `ml restore <id>`. Do not read
`.mulch/archive/` directly — those records are stale by definition. If you need historical
context, run `ml search --archived <query>`.

### Before You Finish

If you discovered conventions, patterns, decisions, or failures worth preserving during
this session, record them before closing:

```bash
ml learn                                                                    # see what files changed
ml record <domain> --type <convention|pattern|failure|decision|reference|guide> --description "..."
ml sync                                                                     # validate, stage, commit
```

Skip if no insight surfaced. Unrecorded learnings are lost; ritual filler records are also noise.
<!-- mulch:end -->
