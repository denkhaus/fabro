# ADR-0027: Dep-fork freeze — the line stays on Petri and stops tracking upstream dependencies

- Status: Accepted (2026-10-09)
- Deciders: user (decision: stay on the Petri line, dep freeze, no measurement run), agent (analysis: two read-only inventories)
- Evidence: `docs/lab/return-analysis/denkhaus0-inventory.md`,
  `docs/lab/return-analysis/petri-era-classification.md` (both 2026-10-09),
  `docs/lab/petri-integration-analysis.md` (the 2026-09-20 migration plan)

## Context

The user asked whether the archived pre-Petri line (`origin/denkhaus-0`,
tip `85440fac6`, 2026-09-21) would be the better base — the impression was
that weeks went into rebuilding pre-Petri capabilities on Petri/Pebble.
Two read-only inventories measured the question.

Facts that shaped the decision:

1. Scale: `denkhaus-0` -> `denkhaus` is 703 commits over 1268 files
   (+119k / -238k lines), but the tree is ~89% identical (3501 of 3940 old
   paths are common).
2. Capability parity: all ten axes we care about are PRESENT at
   `denkhaus-0` (run hooks, fs envelopes, preamble budget, stage journal,
   exit kinds, quota park incl. the provider window gate, sandbox
   providers, resume/rewind/fork/retry, web+API, CLI surface).
3. Cost already paid: of 101 classified closed Petri-era seeds, 50 (51%)
   REBUILD a pre-Petri capability, 23 (23%) FIX carried-over code, 24
   (24%) are NEW. NEW work clusters in enforcement/machinery (readiness
   and window gates, release/line-tip integrity, salvage sweep,
   gate/registry machinery, tracker claim checks), not product surface.
4. Residual: ~20 port-class seeds remain open; the epic fabro-9930 is
   still open while every W0-W5 child is closed.
5. The gap that prompted the discussion — host-side run hooks cannot read
   repo files (fabro-091b) — is NOT a Petri regression. `denkhaus-0`'s
   host hook was cwd'd at `host_source_dir` but runs inside the
   containerized server with no host checkout (fabro-8e13). Returning
   would not fix it; it is a deployment/closure problem.
6. The recurring friction is not the engine's feature set but the
   maintenance of external fork repos (`denkhaus/petri`,
   `denkhaus/pebble`, plus `lithoscomputer/lithos-llm`,
   `sandbox-driver`, `twins`): pins, lock bumps, cross-repo pushes.

## Decision

1. The line STAYS on the Petri base (`denkhaus`). `origin/denkhaus-0`
   remains archived history, not a development base.
2. ADR-0024 point 2 ("dep forks stay tracked ... continue to merge
   upstream") is REVERSED: dep forks stop tracking upstream. No routine
   dep intake; every dep bump is a deliberate, recorded decision with a
   stated reason.
3. The freeze is enforced by a recorded rev list and a battery, not by
   manifest rewriting. `Cargo.lock` already pins every engine dependency
   to an exact rev (the build uses the lock); the new
   `.fabro/scripts/dep-pins-fixtures.nu` asserts the locked revs equal
   the recorded list (petri `6cf8ba06...`, pebble `208f391c...`,
   lithos-llm `fd42e6b2...`, sandbox-driver `90b0d825...` /
   `236196ed...`, twin-openai `19bf6ae2...`, daytona-sdk `0e69058c...`)
   and fails when any of them moves — a bump is then a deliberate edit of
   that list plus a recorded reason.
   Manifest-level `rev =` pins were tried and reverted (2026-10-09):
   cargo's git-source unification splits the petri/pebble crate sets
   across duplicate lock entries and leaves mixed `branch=` specs behind,
   so the pin would have introduced churn without adding enforcement.
4. The fork repos stay OURS: patching them is allowed and expected. The
   dependency-finish rule stands (a session that patches a fork commits
   AND pushes it, then bumps our pin in the same change).
5. Re-open condition: if the engine structurally blocks a change on our
   side (a needed change is impossible in the fork), the return option is
   re-opened on the basis of the two inventories.

## Consequences

- Positive: no involuntary upstream intake on the engine; the engine
  version becomes a decision instead of a drift; one class of moving
  parts disappears.
- Named cost: upstream engine improvements do not arrive automatically
  (90 commits at the frozen base alone, incl. network policies, model
  overrides, the simulation/Lean wave). Each becomes a deliberate
  cherry-pick decision.
- The ~20 port-class residuals and the host-hook transport gap
  (fabro-091b) continue to be worked on the line; the post-2026-10-07
  "dependency bumps are the only external updates" wording is superseded
  by this ADR.
- The two inventories are landed as durable evidence under
  `docs/lab/return-analysis/`.
