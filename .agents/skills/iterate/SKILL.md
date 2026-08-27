---
name: iterate
description: Fabro dev lifecycle in one flow. Use when the user invokes /iterate or asks to start or continue a development cycle - select seed work, build, run code-review, deepen with improve-codebase-architecture when warranted, grill with docs at pivotal forks, and close EVERY cycle with the mandatory self-reflection step that optimizes this skill, records learnings to mulch, and files seeds for new demands. The rust-style-guide skill is the binding Rust coding policy in every phase.
---

# /iterate (fabro only)

One development cycle, end to end. The user starts a cycle by invoking
/iterate; the agent drives everything else. Chat replies in German,
all written artifacts in English.

## Coding policy (Rust)

The `rust-style-guide` skill is THE binding Rust coding policy for all
Rust work in this repo - upstream code, our platform changes, and local
features share ONE style. It is the same guide the upstream workflow
agents use (supporting files in `.fabro/skills/rust-style-guide/`). It is
central in every phase below: grill against it, plan against it, build
with it, review by it, deepen with it. A deviation from the guide is a
decision, not an accident - it needs the user plus an ADR.

## Phase 0 - Orient (always, cheap)

- Which world am I in? `git branch --show-current`. Platform work on
  `denkhaus`, product/lab worlds on `denkhaus-lab` (multi-context, see
  docs/agents/domain.md). NEVER git worktrees - branch switches happen
  in the main checkout.
- Interrupted cycle? Reconstruct BEFORE selecting: `git status` (uncommitted
  diff is the interrupted operation) + `sd list --status in_progress` tell
  you what was mid-flight; continue that work instead of picking a new seed.
- `sd ready` and the open-seed list for candidates. `ml prime <domain>`
  only when the cycle touches that domain.

## Phase 1 - Select

- Pick the highest-value open seed, respecting family order
  (stage envelope, ADR-0009: 900e -> 47b5 -> e47c -> e804 -> ba96),
  blockers (`!`), and the user's current focus.
- If the choice or the design is pivotal or unclear: run a
  grill-with-docs session FIRST (grilling + domain-modeling; evidence =
  CONTEXT.md, ADRs, seeds, code, rust-style-guide). Never guess at
  weichenstellende decisions - they belong to the user.
- Plan against the guide: the plan names the guideline pages the design
  must satisfy (newtypes, enums-vs-traits, error taxonomy, async task
  lifecycle, ...). A design that must deviate is itself a pivotal fork -
  put it to the user in the grill session, never drift silently.
- Claim the seed: `sd update <id> --status in_progress`.

## Phase 2 - Build

- Seeds BEFORE implementation (user directive 2026-08-27): file the seed
  or update the existing one with the agreed design BEFORE writing code.
  If implementation goes wrong, the plan must already be durable in the
  tracker — design decisions never live only in chat or in the diff.
  Close/update the seed with evidence after the work lands.
- Rust changes (platform AND local features): load the rust-style-guide
  skill FIRST - `guidelines.md` plus only the pages the task needs, and
  `workflows/new-rust-project.md` when creating or configuring a crate.
  Write code that conforms from the start; do not retrofit style after
  review.
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
- The standards axis IS rust-style-guide (plus the AGENTS.md strategy
  docs): reviewers load `workflows/code-review-refactor.md` and the
  guideline pages covering the diff. Guide findings are findings, not
  opinions.

## Phase 4 - Deepen (conditional)

- When review surfaced structural smells or the touched area needs
  design sharpening: run improve-codebase-architecture. Skip when the
  cycle was mechanical - not every cycle needs this.
- Judge deepening proposals against the guide as well: enums-vs-traits,
  newtype vs primitives, error taxonomy and layer boundaries, public
  API evolution, module visibility. codebase-design supplies the
  vocabulary (depth, seam); rust-style-guide names the target shape.

## Phase 5 - Integrate

- Commit and push. Deploy/smoke where the domain requires it
  (platform: `just up`; lab: run_workflow integration).
- Close or update seeds (`sd close` / `sd update`), write an ADR when a
  decision crystallized, `sd sync` + push.

## Phase 6 - Reflect (MANDATORY - never skip, every cycle)

1. **Cost review**: what took longer than it should, what needed
   retries, what was missing at decision time? EVERY finding that
   implies a behavior change MUST become an edit to THIS skill file
   (user directive 2026-08-27: a reflection that does not refine the
   workflow is ineffective). Domain knowledge goes to mulch; durable
   preferences to memory; process lessons HERE. No finding may stay
   unrecorded. Deployment on this repo is ALWAYS `just up` (cached
   loop: SPA, binary, image, CLI, compose) - `docker compose up
   --build` builds nothing here and `cargo build -p <name>` drifts with
   upstream package renames.
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
   seeds filed/closed, skill changes made. Every seed listed with a
   ONE-LINE DESCRIPTION, never a bare id (user directive 2026-08-27).
   Then end the turn; the user starts the next cycle with /iterate.

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
  string/int conversions; AGENTS.md Rust import style - these
  fabro-specific rules sit ON TOP of rust-style-guide, never against it.
