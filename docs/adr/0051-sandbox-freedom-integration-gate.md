# ADR-0051: Sandbox freedom, integration gate — the trust boundary moves to the checkpoint

Date: 2026-11-22. Status: accepted. Supersedes the enforcement posture of
ADR-0019 for write scopes; ADR-0019's secrets/redaction class is unchanged.
Amends ADR-0049's `fs_hide=".agents/**"` read hiding (the read hide is
retired; the delivery exclusion stays).

## Context

The line's rules sharpened under friction. The 2026-11-22 envelope
incident (work#1552's lap): an implementer fulfilled an item criterion
that names a file inside `.agents/**` — the one hidden surface — wrote
it through the shell (the documented, taught affordance: policy governs
the shell), and the stage checkpoint detected the out-of-scope paths and
ended the run before committing. Thirty-three minutes of correct-intent
work (an OpenAPI contract, fixtures, tests) died at `+0/−0`. The friction
history says this is systemic, not singular: the reviewer's total
read-only posture made the binding style guide mechanically unreadable at
review time (19 friction records), the evidence pipeline pushed truncated
prints instead of letting the reviewer pull the data (the largest journal
friction cluster, ~90 records), and prompts accumulated recitals,
carve-outs, and footgun one-liners teaching agents around the very rules
that later killed their runs.

The pattern: rules enforced at the agent's tools against work whose
quality is judged later by dedicated layers (reviewer, tester gate,
adjudicator, checkpoint). Prevention at the source for harm that
detection could own, at the cost of the run, the work, and the prompt
readability.

## Decision

**The trust boundary sits at the integration, not in the sandbox.**

1. **Sandbox freedom.** Inside a run's sandbox the agent reads and writes
   what its work needs. `fs_hide` no longer hides surfaces from stage
   agents' reads, and no checkpoint violation ends a run. The shell
   remains deliberately ungoverned — now with nothing to bypass.
2. **Integration gate.** The stage checkpoint commits only in-scope
   paths. Out-of-scope paths are excluded from the commit, named in the
   stage's outcome (journal painpoint), and never leave the disposable
   sandbox. The stage succeeds; the run continues; the reviewer sees the
   named deferral and judges it — accept with a follow-up item, or
   refuse with direction into the graph's re-work edges.
3. **The irreversible class stays hard-gated at the agent.** Secrets and
   redaction keep their present enforcement: a secret that reached a log
   or a prompt is not recoverable downstream, so that gate does not move.
4. **Review data is pull-based.** The engine's native checkpoint diffs
   (already stored as patch blobs) are enumerated for the reviewer; the
   reviewer pulls what it judges from. The evidence stage shrinks to
   metadata (integrity, stat map, churn classification) — the capture,
   inline-budget, and demote machinery is deleted with the prompt rules
   that compensated for it.
5. **Deletion contract.** Every prompt rule that re-teaches what a
   mechanical layer enforces is deleted in the same change that makes the
   layer mechanical. A rule without a mechanical owner states why the
   owning layer cannot carry it.

Least privilege survives with its substance moved: what an agent may
DELIVER is enforced at the only place that matters — what lands. The
sandbox is disposable; the trunk is the asset.

## Consequences

- The develop line loses the run-fatal envelope kill: a scope surprise
  costs a named deferral, not the run.
- The reviewer regains native reads (style guide, prompts, evidence) —
  the review-time readability class closes.
- The evidence capture stack, its budgets, and the reviewer's
  truncation-recovery protocol are deleted.
- The prompt surface shrinks by every recital that duplicated an
  enforcement and every carve-out teaching a bypass of a barrier that no
  longer exists.
- New deliverable surfaces are added by extending the checkpoint's
  write scope — a one-line graph attribute — never by hiding reads or
  killing runs.
- Enforcement changes in the engine (petri checkpoint) are vendored-fork
  changes: the edit, the consuming change, and the fork export are one
  change (ADR-0034), riding the deploy window that carries the binary.
- Workflow/graph/prompt assets ride the run checkout immediately — so
  the assets and the binary MUST land atomically in the same window,
  with the manual laps held until the deploy proves the new semantics.
