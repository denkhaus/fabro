# ADR-0029: Promotion criteria — script layer vs. engine core

Date: 2026-10-14
Status: accepted (user decision, evening session)

## Context

The fork's project layer grew 62 nu scripts (.fabro/scripts, scripts/,
per-lane scripts) over six weeks. The user asked whether established
script functionality should be promoted into the Rust core ("als Feature
statt als Script pflegen").

Three standing decisions bound the answer:

- ADR-0017: the engine is tool-agnostic; bootstrap lives in project
  artifacts. The .fabro layer is deliberately portable (ADR-0024/0027).
- Precedents exist in BOTH directions: `pin-toolchain` moved its core
  into the product (`fabro env update toolchain`) leaving the script as
  a manifest gate; `toolchain-guard.sh` stays a script BY NECESSITY
  (its failure mode is a sandbox without nu); fabro-e2d9 decided
  salvage detection server-side; ADR-0022 carries an explicit promotion
  condition for judgment.
- Upstream intake is frozen (ADR-0024/0027): a core promotion means a
  fork-only cell with the two-pin rule and a presence pin, forever.

## Decision

No bulk promotion. A script is promoted into the core ONLY when ALL
four criteria hold:

1. Data locality: the functionality needs server-owned state no agent
   or script can reach (the salvage case: only the server sees the run
   store).
2. Audience: it is a generic product promise (any Fabro installation
   with runs), not line discipline of this checkout.
3. Change tempo: the logic is stable enough for the release cadence
   (image build + tofu deploy + proof ceremony); weekly-iterated logic
   stays on the script layer.
4. Freeze cost accepted: a fork cell plus pins is a permanent
   maintenance contract under the frozen base.

Promotions are ADR-worthy decisions naming the cell, the pins, and the
API surface. Everything else stays on the project layer; the remedy for
script-layer growth is CONSOLIDATION there (fabro-53cc lib/battery.nu,
fabro-fea9 harness, fabro-7ce7 module family), never migration.

## Shortlist (as of this ADR)

- salvage detection (fabro-e2d9 option e1): GO — server-side detection
  at conclusion time; filing stays with the tracker-owning consumer.
- run-PR wait status (fabro-164b half): thin API exposure — the PR
  supervisor already knows; the script keeps orchestration only.
- judgment (ADR-0022): parked behind the fabro-d4c6 evaluation.

## Anti-shortlist (never promote; recorded so gates stop re-deriving)

- Gates, lints, tracker forensics (push-gate, qualitygate, prompt-lint,
  dup-run-check, claim-check, preflight): line discipline AND they
  change together with the prompts (ADR-0008 one-unit rule).
- Deploy/release wrappers (image-release, run-images, up, install-*):
  they already wrap product commands; the pipeline shape is ours.
- Session instruments (friction-score, loop-digest): iterate-local.
- line-tip-sha: denkhaus-specific (GitButler fork).

## Consequences

Architecture-gate scans apply these criteria before proposing splits or
promotions. Every promotion lands with its cell + two-pin plan in the
same change. The script layer remains the daily-iteration surface; its
growth is managed by consolidation seeds, not by moving code into the
core.
