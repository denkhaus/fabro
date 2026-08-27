---
name: iterate
description: Fabro dev lifecycle in one flow. Use when the user invokes /iterate or asks to start or continue a development cycle - select seed work, build, run code-review, deepen with improve-codebase-architecture when warranted, grill with docs at pivotal forks, and close EVERY cycle with the mandatory self-reflection step that optimizes this skill, records learnings to mulch, and files seeds for new demands.
---

# /iterate (fabro only)

One development cycle, end to end. The user starts a cycle by invoking
/iterate; the agent drives everything else. Chat replies in German,
all written artifacts in English.

## Phase 0 - Orient (always, cheap)

- Which world am I in? `git branch --show-current`. Platform work on
  `denkhaus`, product/lab worlds on `denkhaus-lab` (multi-context, see
  docs/agents/domain.md). NEVER git worktrees - branch switches happen
  in the main checkout.
- `sd ready` and the open-seed list for candidates. `ml prime <domain>`
  only when the cycle touches that domain.

## Phase 1 - Select

- Pick the highest-value open seed, respecting family order
  (stage envelope, ADR-0009: 900e -> 47b5 -> e47c -> e804 -> ba96),
  blockers (`!`), and the user's current focus.
- If the choice or the design is pivotal or unclear: run a
  grill-with-docs session FIRST (grilling + domain-modeling; evidence =
  CONTEXT.md, ADRs, seeds, code). Never guess at weichenstellende
  decisions - they belong to the user.
- Claim the seed: `sd update <id> --status in_progress`.

## Phase 2 - Build

- Platform change: implement directly on the current branch per
  AGENTS.md (build/test commands, API workflow, strategy docs - read
  the relevant doc before changing covered areas).
- Product/lab change or engine+workflow validation: run the develop
  workflow (`just run <workflow> ...`); quality gates run inside the run.
- ADR-0008: change graph, prompts, scripts, and settings as ONE unit
  in the same change.
- English everywhere written; no inline shell logic in the justfile
  (scripts/*.nu only); shell_quote for sandbox command strings.

## Phase 3 - Review

- Run a code-review skill session on the diff since the base point
  (standards axis + spec axis). Fix findings before continuing.

## Phase 4 - Deepen (conditional)

- When review surfaced structural smells or the touched area needs
  design sharpening: run improve-codebase-architecture. Skip when the
  cycle was mechanical - not every cycle needs this.

## Phase 5 - Integrate

- Commit and push. Deploy/smoke where the domain requires it
  (platform: `just up`; lab: run_workflow integration).
- Close or update seeds (`sd close` / `sd update`), write an ADR when a
  decision crystallized, `sd sync` + push.

## Phase 6 - Reflect (MANDATORY - never skip, every cycle)

1. **Cost review**: what took longer than it should, what needed
   retries, what was missing at decision time? Adjust THIS skill file
   directly - self-edit is intended and expected.
2. **Learnings -> mulch**: `ml record <domain> --type
   <convention|pattern|failure|decision> --description ...` (+ evidence
   flags), then `ml sync`. Real insights only - no ritual filler.
3. **New demands -> seeds**: feature requests, bug demand, and gaps
   observed on the way are filed autonomously with `sd create` -
   never parked in chat.
4. **Open forks ahead**: note uncertainties and upcoming pivotal
   decisions for the next grill-with-docs; the user makes weichenstellende
   calls.
5. **Cycle report**: compact summary - outcome, verification evidence,
   seeds filed/closed, skill changes made. Then end the turn; the user
   starts the next cycle with /iterate.

## Standing rules

- Upstream posture: we offer nothing until upstream reacts to our open
  issues/PRs (fabro-f251 parked, no priority).
- The two worlds are temporary (fabro-a9bb): the envelope family feeds
  the retirement. The move timing is decided jointly - when close,
  propose a dedicated grilling session instead of drifting.
- Boundaries: mulch = expertise, seeds = actionable work, ADRs =
  decisions, this skill = process. Nothing stays in chat that belongs
  in one of them.
- Test-only helpers behind the test-support feature; strum for enum
  string/int conversions; follow AGENTS.md Rust import style.
