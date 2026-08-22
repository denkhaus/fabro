# Rule catalog

Severities: **error** = rule violation / broken contract, **warn** = likely
weakness or gap, **info** = hardening suggestion. Doc citations refer to the
snapshot in `docs/fabro/` of the scanned repository.

## AGNOS — project-agnosticism of workflow assets

| Rule | Sev | Checks |
|---|---|---|
| AGNOS-01 | error | Asset references project tooling from `.mise.toml` that is not in `allowed_tools` (binaries matched with aliases: nushell→nu, bun→bun/bunx, go→go/gofmt, …). Ambiguous English-word binaries (go) require command context (`go build`, `` `go` ``) to avoid prose false positives |
| AGNOS-02 | error | Asset leaks the current project name (`.seeds/config.yaml`, go.mod module, package.json, Cargo.toml). Platform uses of "fabro" (`.fabro/`, `Fabro-`, `fabro(`, `docs.fabro.sh`) are exempt |
| AGNOS-03 | error | Example seed id uses this repo's tracker prefix (`<project>-<hash>` from `.seeds/config.yaml`) |
| AGNOS-04 | warn | Asset references a repo file outside the workflow directory (couples workflow to repo layout) |
| AGNOS-05 | warn | Prompt hard-codes project-specific gate semantics (size limits, gofmt/go vet/go test, cargo, npm) instead of treating `just qualitygate` as opaque |
| AGNOS-06 | error | `script=` attribute invokes a project-tool binary (e.g. `nu …`) instead of an allowed command |
| AGNOS-07 | info | `script=` uses repo-root-relative path — works, but pins the workflow to the repo layout and duplicates the path per node |

## GRAPH — workflow.fabro correctness

| Rule | Sev | Checks | Docs |
|---|---|---|---|
| GRAPH-01 | warn | Unknown node shape | outcomes.md L28-35 |
| GRAPH-02 | error | `condition` uses unknown outcome value (valid: succeeded, failed, partially_succeeded, skipped) | outcomes.md L13-18, L121-134 |
| GRAPH-03 | error/warn | `preferred_next_label` in a prompt's JSON contract must match an outgoing edge label exactly; non-command nodes need an unconditional fallback edge | context.md (preferred_label), failures.md |
| GRAPH-04 | warn | Broad `outcome=failed` edge into exit swallows schema/infra failures and lets the run finish "successfully" | outcomes.md L108-119 (goal gates), failures.md |
| GRAPH-05 | warn/info | Missing `max_node_visits`, missing command `timeout`, default retries on deterministic gates, missing `output_retries`, no `goal_gate` anywhere | failures.md |
| GRAPH-06 | error | Dangling `@prompt` reference, missing script file, workflow.toml graph pointer broken | prompts.md L39-49 |
| GRAPH-07 | warn/info | `{{ goal }}` without graph goal attribute; `{{ inputs.X }}` without `[run.inputs]` | prompts.md L51-70 |
| GRAPH-08 | error | Unreachable node / unreachable exit | — |

## SCRIPT — deterministic bridge scripts

| Rule | Sev | Checks | Docs |
|---|---|---|---|
| SCRIPT-01 | warn | Checkpoint-base detection greps whole history (`--grep "Fabro-Completed:"`): merged prior run branches poison the base; empty-checkpoint fallback yields empty diff | checkpoints.md L38-52, L94-101 |
| SCRIPT-02 | info/pass | Evidence diff truncated by hand (100k) without disclosure — reviewer blind beyond the cut. When the prompt/script disclose the budget explicitly → pass | context.md (blob offloading) |
| SCRIPT-05 | warn | `--diff-filter=` without `D` drops deletions from the per-file diffs; deletion-criteria seeds cannot be verified | — |
| SCRIPT-03 | warn | Gate bridge assumes the project gate covers untracked files; Fabro stages all worktree changes into the next checkpoint, so untracked artifacts ride into the run branch | checkpoints.md L46-50 |
| SCRIPT-04 | error | `just <recipe>` referenced by an asset but not defined in the justfile | — |

## PROMPT — prompt contracts

| Rule | Sev | Checks |
|---|---|---|
| PROMPT-01 | warn | Prompt expands `{{ goal }}` without an injection guard ("user-provided data") |
| PROMPT-02 | error/warn | Prompt reads a context key no node writes, or before its producer can run |
| PROMPT-03 | warn | Planner shortcut closes seeds on visual inspection, bypassing gate and reviewer |
| PROMPT-04 | warn | Tool-less reviewer judges against the Planner's brief while the evidence capture lacks `sd show` (ground truth) |

## REPO — repo wiring

| Rule | Sev | Checks |
|---|---|---|
| REPO-01 | error | `run.prepare` references a `just` recipe that does not exist |
| REPO-02 | warn | Tool contract (just/ml/sd) not provisioned or documented anywhere in the repo |
