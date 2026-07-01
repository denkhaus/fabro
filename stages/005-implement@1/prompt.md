Goal: # Plan: make `Sandbox::glob` semantics consistent across providers

## Summary

`Sandbox::glob(pattern, path)` returns correct results on the **Local** provider
but silently wrong/empty results on the **Docker** and **Daytona** providers.
The remote providers translate the glob pattern into `find <base> -name
<pattern>`, and `find -name` is not a glob matcher — it matches the basename
only and cannot match a pattern containing `/`. So any pattern with a slash or
`**` returns nothing (or the wrong set) on remote sandboxes.

Fix it by separating the two things globbing actually does — **traversal**
(enumerate files, requires filesystem access) and **matching** (pure path
logic) — so that only traversal happens inside the sandbox, and matching is done
once in shared Rust code using the `glob` crate's `Pattern` matcher. This makes
Local, Docker, and Daytona semantically identical by construction.

## Root cause (verified)

- Trait method: `async fn glob(&self, pattern: &str, path: Option<&str>) -> crate::Result<Vec<String>>`.
- **Local** — `lib/crates/fabro-sandbox/src/local.rs:628`: uses `glob::glob("<base>/<pattern>")` (the `glob` crate). Correct glob semantics.
- **Docker** — `lib/crates/fabro-sandbox/src/docker.rs:1845`: `find <base> -name <pattern> -type f`. **Broken.**
- **Daytona** — `lib/crates/fabro-sandbox/src/daytona/mod.rs:1853`: `find <base> -name <pattern> -type f`. **Broken (identical bug).**

`find -name` matches only the file's basename and refuses a pattern containing
`/`, so e.g. `find <base> -name "*/SKILL.md"` exits 0 with **empty** output.
Reproduced locally:

```
find <base> -name "*/SKILL.md" -type f   → (empty), exit 0     # what remote runs
find <base> -name "SKILL.md"     -type f   → <base>/eng-patch-cves/SKILL.md
find <base> -path "*/SKILL.md"    -type f   → <base>/eng-patch-cves/SKILL.md
```

## Blast radius / why it matters

There are two real callers of `sandbox.glob`, and both are broken on every
remote (clone-based) sandbox — which are the production providers:

1. **Skill discovery** — `lib/crates/fabro-agent/src/skills.rs:247`:
   `env.glob("*/SKILL.md", Some(dir))`. Result: skills are **never discovered**
   in Docker/Daytona runs. `/skill-name` prompt syntax silently no-ops and the
   agent runs without the skill loaded.
2. **The agent's `Glob` tool** — `lib/crates/fabro-agent/src/tools.rs:369`:
   `env.glob(pattern, path)` with arbitrary LLM-supplied patterns
   (`**/*.rs`, `src/**/*.ts`, etc.). Result: the agent's file-search returns
   wrong/empty results for any pattern with `/` or `**` on remote sandboxes — a
   quiet, general correctness hole, not just a skills problem.

This was discovered while trying to run a skill-based workflow on the Daytona
testing server: `agent.skills.discovered` reported `skills:[]` even though the
branch cloned correctly and the `SKILL.md` was present in the workspace.

## Reasoning: why Local works and remote can't just call the crate

Globbing is two jobs fused together:

- **Traversal** — walking directories (`readdir`/`stat`) to enumerate which
  files exist. Requires real filesystem access.
- **Matching** — deciding whether a path string matches the pattern. Pure
  string/path computation; touches no filesystem.

`glob::glob()` does both at once against the local filesystem.

- On **Local**, the sandbox *is* the machine running the code, so `glob::glob`
  walks the real files and works.
- On **Docker/Daytona**, the files live **inside the container / remote VM**,
  not on the host running the Rust code. `glob::glob()` would walk the *host's*
  filesystem and never see the sandbox's files. Having the `glob` crate as a
  dependency doesn't help: its traversal is hard-wired to `std::fs` on the local
  machine, and those files aren't local. The only way to observe a remote
  sandbox's filesystem is across its boundary (exec a command, or the provider's
  file API) — which is why the remote impls shell out to `find` at all.

The bug is that the remote impls delegated **matching** to `find` too
(`-name`), whose semantics differ from a glob matcher. Only traversal actually
needs to cross the sandbox boundary; matching does not. So the fix is to keep
traversal in the provider (list files) and move matching into shared host-side
code using the crate's `Pattern` matcher.

Alternatives considered and rejected:
- **Translate glob → a faithful `find` expression** (e.g. `-path`, `-regex`):
  fragile — `-path`'s `*` crosses `/` (so `*/SKILL.md` would wrongly match any
  depth), and `**`, `[!a-z]`, brace expansion, `?` don't map cleanly; also
  varies GNU vs BSD `find`.
- **Ship a real glob matcher binary into every sandbox image**: heavier —
  image dependencies and version coupling — versus `find`, which is universal.

## Proposed fix (design)

Introduce one shared matcher and have each provider supply only a file listing.

1. **Shared matcher helper** in `fabro-sandbox` (new module, e.g.
   `src/glob_match.rs`), reusing the already-present `glob` crate
   (`fabro-sandbox/Cargo.toml:43`, `glob = "0.3"`):
   - `pub(crate) fn match_glob(base: &str, pattern: &str, candidate_paths: &[String]) -> Vec<String>`
   - Build the full pattern from `base` + `pattern` (mirror what Local passes to
     `glob::glob`), compile with `glob::Pattern::new`, and match each candidate
     absolute path with `glob::MatchOptions`.
   - Use `require_literal_separator: true` so `*` matches within a single path
     segment (i.e. `*/SKILL.md` = exactly one directory level), matching the
     intent of the skill pattern. Confirm the chosen `MatchOptions` reproduce the
     Local `glob::glob` result set via tests (see below).
   - Sort results (keep existing ordering behavior; Local sorts by mtime, remote
     sorted lexically — preserve per-provider ordering or standardize and update
     any order-sensitive callers/tests).

2. **Provider file listing** — each remote provider enumerates candidate files
   under `base` *without* letting `find` do the matching:
   - Docker: `find <base> -type f` via the existing exec path (keep
     `shell_quote` on `base`), then `match_glob`.
   - Daytona: prefer the **filesystem list API** over shelling `find` if one is
     available (more robust — file reads work on Daytona even when the shell is
     fail-closed on a GitHub-token mint failure, which is a real failure mode).
     Otherwise `find <base> -type f`, then `match_glob`.
   - Performance: to avoid listing an entire large tree, derive the longest
     literal (non-wildcard) leading directory of `pattern` and append it to the
     `find` root; only the remaining wildcard portion goes to `match_glob`. For
     patterns with no literal prefix (like `*/SKILL.md`) this lists `base` as
     today. This is an optimization, not required for correctness — implement it
     if straightforward, otherwise note as a follow-up.

3. **Route Local through the same matcher too** (recommended, one source of
   truth): have Local enumerate files under `base` and filter through
   `match_glob`, so all three providers share identical matching. This is the
   one judgment call — if reproducing Local's exact current results proves
   fiddly, the acceptable fallback is to leave Local on `glob::glob` and add a
   test proving `match_glob` agrees with `glob::glob` on shared fixtures. Do not
   change Local's observable behavior without a test locking it.

4. Update the delegating wrappers if the trait shape changes
   (`lib/crates/fabro-sandbox/src/sandbox.rs:141` delegation macro,
   `worktree.rs:327`, `read_guard.rs`). If `glob` stays a trait method with the
   same signature and only the impl bodies change, these need no changes.

## Implementation steps (red/green TDD)

1. Write failing unit tests for `match_glob` first:
   - `*/SKILL.md` matches `<base>/a/SKILL.md`; does **not** match
     `<base>/SKILL.md` (needs a directory) and does **not** match
     `<base>/a/b/SKILL.md` (exactly one level under `require_literal_separator`).
   - `*.rs` matches only top-level `.rs` files under `base`.
   - `**/*.rs` matches `.rs` files at any depth (verify the crate's `**`
     handling with the chosen `MatchOptions`).
2. Implement `match_glob` (`src/glob_match.rs`), export `pub(crate)`.
3. Rewrite `Docker::glob` (`docker.rs:1845`) to list (`find -type f`) + `match_glob`.
4. Rewrite `Daytona::glob` (`daytona/mod.rs:1853`) to list (file API preferred,
   else `find -type f`) + `match_glob`.
5. (Recommended) route `Local::glob` (`local.rs:628`) through `match_glob`; keep
   a test asserting parity with the previous behavior.
6. Add a regression test for the original symptom: `glob("*/SKILL.md", base)`
   returns a nested `<base>/<name>/SKILL.md`. Where a live Docker/Daytona env
   isn't available in unit tests, test the `list + match_glob` composition
   directly against a temp fixture tree.
7. Confirm the two real callers work end to end: skill discovery
   (`skills.rs:247`) and the agent `Glob` tool (`tools.rs:369`).

## Acceptance criteria

- `sandbox.glob("*/SKILL.md", Some(dir))` returns the nested `SKILL.md` on Local,
  Docker, and Daytona.
- `match_glob` semantics match Local's prior `glob::glob` results on shared
  fixtures (locked by test).
- No regression to the agent `Glob` tool for common patterns (`*.rs`,
  `**/*.rs`, `src/*.ts`).
- All `implement-plan` verify gates pass: `cargo check`, nightly `clippy -D
  warnings`, `fmt --check`, `cargo nextest run --workspace`, docs check, web
  `typecheck`/`test`, and release build.

## Out of scope

- The GitHub App / Dependabot-alerts permission issue that fail-closes the
  agent shell on the testing server (separate, server-side; not a code change).
- The `patch-cves` workflow and `eng-patch-cves` skill themselves.
- Adding new glob features beyond parity with the Local provider.

## Conventions to follow (repo)

- Use the `shell_quote()` helper for any value interpolated into a shell string
  (the `find` base). Never hand-roll quoting.
- No wildcard imports in production code (`use foo::*`).
- Errors via `crate::Error`; follow the error-handling strategy doc.
- Keep any test-only helpers behind `#[cfg(test)]` / `test-support`; do not
  expose fixture helpers from production modules.
- Match surrounding code style in `fabro-sandbox`.


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


Read the plan file referenced in the goal and implement every step. Make all the code changes described in the plan. Use red/green TDD.