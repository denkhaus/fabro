# Selection protocols (Phase 1)

Load at Phase 1. Sprint composition, the grilling format, lane rules,
vetting, and the local-first working mode — with the directives behind
each rule.

## What to select

- Pick the highest-value open items respecting family order (ADR-0009:
  900e -> 47b5 -> e47c -> e804 -> ba96), blockers (`!`), and the user's
  focus.
- Sprint composition (user directive 2026-10-29): a sprint carries TWO
  to THREE work items, selected together in this phase — the count
  scales down with the items' scope (a large item shares the sprint
  with at most one small companion; never a one-item sprint, whose
  review is pure overhead). The review cost is per sprint, so batches
  amortize it. Grill each item's design on its own; work the batch
  sequentially, closing each item with its own evidence.

## Lane tranches

- Lanes hold AT MOST FIVE items each — a hand-picked tranche selected
  for the CURRENT situation under the same discipline as the sprint
  batch (relevance-vetted per the lane-admission rule, family order,
  blockers, user focus; the user hand-picks with the session). When a
  tranche is worked through, select and assign the NEXT one; never
  backfill a lane wholesale from the backlog. The backlog lives
  unlaned; a lane is a tranche, not a pool.

## The grilling format (binding)

- FULL CONTEXT BEFORE THE GRILL (user directive 2026-11-01): every grill
  round is PRECEDED by a full-context pitch of the topic, written in the
  wait-what style — ASD-STE100 Simplified Technical English (short
  sentences, one topic per sentence, active voice, no idioms) over the
  repo's ubiquitous language (GLOSSARY.md; follow GLOSSARY-MAP.md to the
  right one when the repo has more than one). The pitch covers: what the
  mechanism is today, which incidents shaped it, what the fork changes,
  and who the callers are. A grill pitch the user cannot reconstruct the
  topic from is a process defect — the round is aborted and re-pitched
  before any options reach the question tool.
- GLOSSARY NAMES SURVIVE TRANSLATION (user directive 2026-11-01): in
  German chat framing, the English proper names of the ubiquitous
  language stay EXACTLY as GLOSSARY.md spells them — a Run is a Run
  (never a "Lauf"), a Stage a Stage, a Claim a Claim, the Adjudicator
  the Adjudicator (never a "Schlichter"), a Gear a Gear, merge-complete
  stays merge-complete. No ad-hoc translations and no invented German
  synonyms for glossary terms: the glossary IS the shared
  understanding, and a renamed term in prose is a fork of the
  vocabulary. A concept without a glossary entry gets named in English,
  used consistently, and carried into the glossary — never silently
  translated.
- EVERY decision point is grilled BEFORE implementation (user directive
  2026-10-12, Grundsatz-review; procedure directive 2026-10-18): run
  the ACTUAL grill-with-docs skill (grilling + domain-modeling;
  evidence = CONTEXT.md, ADRs, work items, code, guide) for EVERY
  design/mechanism/standing-rule choice — not only pivotal ones.
- Rounds over the design tree: the whole frontier per round, numbered
  questions each with a recommended answer, wait for answers, recompute
  the frontier. Domain-modeling sharpens terms against GLOSSARY and
  writes ADR/glossary entries as decisions crystallize.
- Rounds are DELIVERED through the `question` tool — one call per round;
  the options carry one-line descriptions WITH the Jev pre-screen
  distribution; the recommended option is listed first and marked
  "(Recommended)"; free-form answers go through the auto-added
  own-answer option (user directive 2026-10-26: structured selections
  instead of free-text question lists in chat; the answered record is
  what gets written into the design). The noul dimensions are their own
  question in the same call.
- Judgment pre-screen (ADR-0022, application point a): every
  grill-with-docs fires a pre-screen BEFORE the user grill — choice
  questions per fork plus noul for risk-appetite; present distributions
  WITH the alternatives; noul answers stay user-owned; record the run
  in the item/decision (framing-bias caveat named).
- No tacit decisions: an ungrilled fork that ships is a finding. The
  grill names the alternatives and picks the optimal path WITH the
  user; the chosen path and its rejected alternatives land in the item
  or ADR.
- Weichenstellende decisions belong to the user. Public-contract NAMES
  (graph attributes, API fields, tool names, CLI flags, settings keys)
  are pivotal too: name the capability, surface naming for user
  decision before the first commit.
- Plan against the guide: name the guideline pages the design must
  satisfy. A design that must deviate is itself a pivotal fork.

## Dispatch (the explicit-delegation case)

- Dispatch is one ceremony: ASSIGN, VERIFY, FIRE. An item named in a
  run goal must be claimable by the fail-closed picker BEFORE the fire
  — an unassigned item in a goal makes the planner silently substitute
  the next claimable item while the implementer still follows the goal
  text, closing the WRONG item (the claim/claim-check mismatch class).
  Verify with `work show <id>` (assignee/lane set) in the same breath
  as the fire; the goal text names the item id AND its one-line topic.

## Local-first working mode

- Invoking /iterate means the session works LOCALLY (user directive
  2026-10-02 evening; the conductor-role split of directive 2026-09-06
  is retired) — the sprint loop (claim, implement, review, reflect) IS
  this session's working mode. EVERYTHING is implemented by this
  session locally; ONLY loop-asset work (surfaces
  `.fabro/workflows/**`, `.fabro/scripts/**`, `scripts/**`, justfile,
  fabro-test rigs) is lane material for the loop workflow (which works
  it WITHOUT sprint framing). The session still orients, grills
  pivotal forks (writing the agreed design INTO the item before
  implementation), reviews, reflects, reports.
- EXTENSION (user directive 2026-10-27, sprint 58): until the re-armed
  line is PROVEN reliable (items verifiably worked by runs, the
  supervised laps green), propose NO lane moves and NO line assignments
  at all — not even as recommendations; the session works every item
  locally in sprint order, and the cycle report's ASSIGNMENT PENDING
  section recommends LOCAL work (next sprint candidates). Lane
  assignment resumes only on an explicit user decision after
  reliability is proven.

## Vet before working; lane admission

- VET BEFORE WORKING (user directive 2026-10-27 evening, born in the
  supervised laps): every item considered for work — lap, lane, or
  local sprint — gets a RELEVANCE TEST against the CURRENT codebase
  first: does the cited path/mechanism still exist, is the demand
  overtaken by landed work? Migration-era items are presumed stale
  until vetted (median open age ~50d; the laps caught two such
  artifacts live: a lane bug in PROJECT_FACTS and a zombie doc item).
  A vetting pass over the lane pools is standing backlog work, not
  ceremony.
- LANE ADMISSION (user directive 2026-10-29, extending the test): an
  item enters a lane ONLY after that check passes for THAT item,
  individually — never bulk-assign the migration-era pool, whose
  premises most restructures have overtaken. Unvetted pool entries are
  how a re-armed picker burns runs on dead premises. The backlog
  relevance pass executes this: close the dead items with evidence,
  keep only the hand-checked, high-quality ones.

## Ownership

- Ownership per ADR-0018: items are unassigned by default; design forks
  and user-decisions get `needs-user`. Emergencies bypass the picker:
  line down -> agent repairs directly with the user's knowledge, the
  item filed retroactively.
