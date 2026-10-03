---
name: merge-upstream
description: >-
  Merge fabro upstream commits into the fork branch denkhaus (never main):
  fetch, conflict resolution policy, regression testing, deploy with smoke,
  full upstream report, workflow-change assessment, and a self-reflection
  step that improves this skill. Use when upstream commits should be merged
  into our fork, when the user says "merge upstream", or after a fetch
  shows new upstream/main commits.
disable-model-invocation: true
---

# /merge-upstream (fabro only)

## Scope — local-only agent skill (user directive 2026-09-16)

This skill is a LOCAL session instrument for the human-side agent. It is
NEVER referenced from, loaded by, or wired into fabro workflows. Skills
that a fabro workflow's agent stages should use MUST be vendored into
`.fabro/skills/<name>/` in the repo (like `rust-style-guide` and
`improve-codebase-architecture`) — only there can a run's LLM agent
recognize and load them.


Project skill for the fabro repo at `~/dev/fabro` — never apply to other
projects.

Merge upstream `fabro-sh/fabro` commits into OUR fork branch `denkhaus`.
`main` is upstream-only and deliberately OUT OF SCOPE — never merge, push,
or rebase it.

## Preconditions

1. `cd ~/dev/fabro`, current branch is `denkhaus`, worktree clean.
   If not: stop and tell the user what to clean up.
2. `git fetch upstream --prune` — report the commit count and the one-line
   log of `denkhaus..upstream/main`. If zero: report "nothing to merge" and
   stop. Never touch `main`.
3. Deployed server state: if the instance was just built from the current
   tree, note it — the final `just up` will rebuild on the merged tree.

## Pre-merge diagnosis (mandatory — user directive 2026-09-19)

After a successful fetch and BEFORE running `git merge`, produce a short
diagnosis and present it to the user. Ideal merge conditions come from
knowing the overlap first, not from resolving blind conflicts; this is
how fork features are secured best. Cover all five points:

1. **Both-touched files**: for every file upstream changes
   (`git diff --name-only $(git merge-base HEAD upstream/main)..upstream/main`),
   compute our net fork delta (`git diff <merge-base>..HEAD -- <file>`)
   and predict per file: clean auto-merge / adjacent-hunk risk / real
   conflict. Report the counts.
2. **Features at risk**: map both-touched files against
   `references/touchpoints.md` rows; name the seeds whose code overlaps
   upstream changes and the fast verification to run post-merge.
3. **Semantic drift scan** (the class textual merges never show): when
   upstream changes a schema, settings key, loader, or event vocabulary,
   grep FORK-OWNED files and fork-added test fixtures for the OLD shape
   (fork files never conflict — they just break), and check operator
   data (prod settings.toml) whenever a loader turns strict. Scans are
   FULL-TREE and UNTRUNCATED — a `| head` cut hid a fork fixture in the
   2026-09-19 session and the test suite, not the diagnosis, caught it.
4. **Seam-shrink opportunities**: fork additions still living inline in
   upstream-owned files (consts, helpers, doc edits) move into
   fork-owned files BEFORE the merge (minimal seam: mod/import decl +
   one call), with a presence pin if the two-pin rule is unmet. The
   refactor is behavior-neutral and verified green (build + tests +
   clippy + fmt) on the pre-merge tree, then committed.
5. Present the diagnosis — conflict predictions, features at risk,
   drift findings, seam-shrink proposals — and get the user's go
   BEFORE merging.

## Resume after an interrupted session (2026-08-25 lesson)

If a previous merge session died (disk space, crash): the merge commit may
already exist and be committed while verification/push/deploy never ran.
Reconstruct state first — `git log`, `git status`, `ls .git/MERGE_HEAD`,
`git log origin/denkhaus..denkhaus` — instead of re-merging. Variant seen 2026-08-27: branch BEHIND
   origin/denkhaus with dirty files — compare worktree blobs against the
   pushed feature commit (`git hash-object <f>` vs `git rev-parse
   <rev>:<f>`) before stashing; if origin supersedes the drafts, stash as
   backup, `git pull --ff-only`, verify, drop. A wiped
`target/` means full rebuild (~10 min build + ~10 min test compile).

Disk guards before/during `just up`: check `df -h /`. Safe reclaim:
`docker builder prune -f`, unused `fabro-runner-<sha12>` images (verify no
running container uses them first). NEVER `docker volume prune` — the
release build cache lives in the `fabro-docker-cargo-target-<arch>` volume
(a prune turns the 8-min cached build into a ~40-min cold one).

## Merge phase

Optional judgment pre-screens (ADR-0022): for diagnosis point 1, one
fan-out call over both-touched files can rank clean / adjacent-risk /
real-conflict per file (choice; state = both hunks); for the semantic
drift scan it is the second net for the non-greppable case ("does the
stricter loader/schema make this fork file semantically invalid?",
noul); for smart adaptation, per fork feature obsolescence (noul).
Grep stays primary (full-tree, untruncated). Advisory only, fail-open
(`.fabro/scripts/judgment.nu`); judgments log automatically to the
canonical session log `~/.local/state/fabro-judgments/<YYYY-MM-DD>.jsonl` (script default; --log-file only for
overrides) with --skill merge-upstream.

Run `git merge upstream/main --no-commit` (a background watcher on this
host auto-pushes new commits within ~30s; `--no-commit` prevents it from
publishing a default-message merge before adaptations and the real
message exist). On conflicts, apply the resolution policy
from `references/conflict-policy.md` (core rule: our features AND upstream
changes both survive; adapt our call sites to upstream's new signatures
instead of reverting either side). Conflict classes seen so far and their
resolutions are listed there — check for a match before improvising.
After resolving, run `cargo nextest run -p <touched-packages> --no-run`
BEFORE the full pipeline: upstream struct-variant refactors (e.g.
RunTarget::Git -> Git(GitRunTarget)) pass `cargo build` but break OUR
test initializers with E0063/E0061 (2026-08-27 lesson).

While resolving, watch for upstream code that SUPERSEDES our features
(see "Smart adaptation" below).

Style-guide sync: when upstream changes `.fabro/skills/rust-style-guide/`,
copy `.fabro/skills/rust-style-guide/SKILL.md` byte-identical over
`.agents/skills/rust-style-guide/SKILL.md` so the agent-facing coding
policy (see /iterate) never drifts from upstream's; note it in the report.

## Verification phase

Order matters; a red earlier step means fix before continuing.

0. FORK-ONLY PRESENCE SUITES (user directive 2026-09-13): immediately
   after conflict resolution, run `cargo nextest run -p fabro-workflow --
   fork_seam` (plus any newer fork-only test files). A red fork-only test
   = a fork feature was dropped in the resolution — restore it or stop;
   never relax the test. Also grep fork-guard markers that have no
   presence test yet (touchpoints.md rows) before trusting the gate.
1. `cargo build --workspace` — zero errors.
2. `cargo +nightly-2026-04-14 fmt --all` then `--check --all` green.
3. Tests (reduced threads avoids load timeouts on this host):
   `ulimit -n 8192 && unset FABRO_SERVER && cargo nextest run -p fabro-sandbox -p fabro-workflow -p fabro-api -p fabro-server -p fabro-cli --no-fail-fast --test-threads 4`
   A test that fails only under full parallelism but passes isolated is a
   LOAD signal, not a regression — note it, do not chase it endlessly.
   (2026-08-25, v0.336.0: `cmd::config::create_explicit_workflow_path_…`
   failed SQLite seeding under load, passed isolated — known flake.)
   (2026-08-26, v0.337.0: `cmd::server_start::start_already_running_exits_
   with_error` failed at --test-threads 4, passed isolated — same class.)
4. `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings` — zero.
5. Web: `cd apps/fabro-web && bun run typecheck && bun run test` — 0 fail.

## Commit + deploy phase

1. One merge commit, message `merge: upstream/main (<old> -> <new>) —
   <version>` listing conflicts resolved and call-site adaptations.
2. `git push origin denkhaus`.
3. Deploy + verify (user directive 2026-09-13, anchored in iterate):
   PRODUCTION is https://mirtuell.net — after an upstream merge (engine
   changes by definition), build the release image with `just
   image-release` (ghcr.io push), pin the new digest in
   ~/dev/fabro-tofu/variables.tf (fabro_image_ref), and deploy with
   fabro-tofu. TOOLCHAIN IMAGE (2026-09-19 lesson, run
   01M2XP626TS9): `just image-release` also pushes a new
   fabro-toolchain tag via run-images.nu; after PUTting it into the
   server-managed `toolchain` environment, ALSO docker-pull that tag on
   the prod host (SSH) — API-initiated sandbox pulls carry no registry
   auth, so a tag missing on the host 401s and fails the run in
   <1s (product gap seeded; see touchpoints 2026-09-19).
   PRE-DEPLOY ERA CHECK (2026-09-18 lesson): if the new binary changes
   projection semantics vs the deployed one (status taxonomy, summary
   shape), validate startup against a prod DB snapshot first — copy the
   SQLite (WAL-checkpoint before cp!), run the new image isolated
   (--network none, dummy SESSION_SECRET, prod settings.toml), and
   require "Activated SQLite run history" before deploying. Terminal
   rows written by older binaries otherwise crash-loop production
   (fabro-eec6). `TF_VAR_state_passphrase` from gopass +
   `TF_DATA_DIR=.terraform-prod tofu plan -var-file=envs/prod.tfvars`
   (verify the plan touches only image/container/ghcr-auth), then apply.
   Smoke on https://mirtuell.net: /health, HTTP 200, auth probe, fresh
   uptime. `just up` is the LOCAL test stack only — never production.
   Deploy windows: only while no conductor pass runs (check
   `fabro ps --server https://mirtuell.net`).
4. Prove the container runs the merged code (e.g. strings of the binary
   for a marker of the new upstream change) when cheap.

## Smart adaptation (regression watch + obsolescence)

For EACH upstream commit, ask three questions; the answers feed the report
and, when yes, become follow-up actions:

1. **Regression?** Does it touch code our features build on (publish path,
   terminal taxonomy, preamble/pipeline, sandbox contract, ask/attach,
   spa_refresh, smoke/just plumbing)? If yes: name the feature seeds at
   risk and prove via targeted tests that behavior still holds.
2. **Obsoleted?** Does upstream now provide what one of our open seeds or
   local features implements (same or better)? If yes: do NOT silently
   keep ours — flag it in the report with a concrete recommendation
   (close the seed as superseded / port our improvement onto upstream's
   concept / merge approaches), and file or update the seed accordingly.
3. **Better concept?** Did upstream introduce a mechanism (e.g. the sandbox
   runtime directory, SQLite stores, run targets) that our planned features
   should be REBUILT on instead of extended locally? If yes: name the
   affected open seeds and propose the rebase of their design.

Known touchpoints to check every time are listed in
`references/touchpoints.md`.

## Report phase

Produce the final report (structure in `references/report-template.md`):
merge identity, verification results, the upstream commits grouped by
theme with "what it means for us", our-code impact (conflicts +
adaptations), regression status per feature, obsolescence/adaptation
findings, upstream code-quality assessment, overall direction — AND
explicitly a **Workflow changes** section: whether our lab workflows
(currently only `develop`, canonical on meta/denkhaus-lab, synced to the
product worlds) need changes driven by this merge (e.g. new engine
capabilities the develop graph should adopt; changes to prompts, gates,
or the run_workflow script).

## Self-reflection phase (mandatory, last)

After the report, reflect on THIS merge session and improve this skill:

1. What took longer than it should / needed retries? (e.g. marker
   stripping that broke delimiters, wrong test-thread settings)
2. Which conflict classes, touchpoints, or adaptation patterns were NEW?
   Add them to `references/conflict-policy.md` and
   `references/touchpoints.md` (dated, one line each).
3. Was any verification step's cost/benefit wrong (skippable, missing)?
   Adjust the phase lists.
4. Did the report miss a section the user asked for? Update
   `references/report-template.md`.

Editing THESE FILES is allowed and intended (the skill improves itself);
knowledge that belongs to the project (bug causes, project expertise)
still goes to mulch, and actionable work to seeds — never into this skill.
State in one short paragraph what was learned and changed.
