# ADR-0038: Painpoints are platform records — the transitive closure chain

## Status

Accepted (2026-10-30). Supersedes ADR-0037 (whose sub-issue carrier and
direct parent/child link this decision replaces).

Amended 2026-10-30 (user decision, sprint 69): the GENUS is the
observation, not the painpoint — an observation can contain painpoints,
never the reverse. The record kind is `observation.recorded` with
categories `painpoint | note`; the tool is `fabro_observation_record`;
the graph attribute is `x.observations="required"`. The chain and every
mechanism below are unchanged; only the vocabulary turned.

## Context

ADR-0037 linked painpoint work items to their parent item through
GitHub sub-issues. The grill continued one round further and found the
stronger architecture: painpoints become first-class PLATFORM RECORDS
in fabro's database, and the link chain gains an intermediate node.
The anchor problem (claimless runs), the recursion (who revises the
revisor), and the duplicate-flood (text-similarity refiling) all
dissolve structurally instead of by convention.

## Decision

1. Observations (painpoints and notes alike) are platform records,
   persisted per project/run/stage through an agent tool available to
   every stage — conductor, revisor, and every claimless run included.
   The records carry their run context natively; no anchor convention
   is needed.
2. THE TOOL CALL IS A PRESENCE CONTRACT (user decision): every agent
   stage under `x.observations="required"` must call the observation
   tool at least once; an empty entry is the explicit "nothing durable"
   signal; a stage that never called it is INVALID. The engine enforces
   this beside the output-schema validation.
3. The closeout sweeps the journal's painpoint array into records as
   the mechanical floor during the transition — stages that predate
   the tool lose nothing.
4. Work items link to OBSERVATIONS (not to parent items), and the link
   lives ONLY in fabro's database — the platform owns both ends. The
   closure radius derives transitively: an item's radius is the open
   work items of the observations discovered in its runs. No GitHub
   mirror (a mirror is a drift surface); the board's radius view
   renders the chain.
5. A bird's-eye LLM agent runs as a PLATFORM HOOK at every run's
   terminal state: it reviews that run's observation records, files one
   work item per painpoint observation, and DEDUPES AT THE LINK — if a
   work item for an observation already exists, it is linked, never
   recreated. The revisor workflow's "revise the freshest run" role
   dissolves into this hook.
6. The work close verb keeps the hard gate from ADR-0037's grill: a
   claimed item with an open (transitive) radius is refused;
   `--force` overrides with a mandatory reason that lists the open
   observation-linked work items.
7. The existing pool: new mechanism forward, opportunistic backfill
   where a theme is still open. Tranche selection keeps the
   PREFERENCE rule: close open radii before opening new themes.

## Consequences

- Every run — conductor, revisor, develop, probe — produces its radius
  mechanically; the revisor needs no revisor because its own capture
  is a validated tool call and its own observations are records of its
  own run.
- Duplicate work items get a structural answer: the observation record
  is the identity; filing dedupes at the link.
- The radius view, flood metrics, and friction inputs become platform
  queries; the GitHub issue tree stops being the link carrier (the
  tracker keeps items, fabro keeps the chain).
- The engine gains a stage-validity rule (tool-call presence) and a
  terminal hook with LLM work; both need pins and a bench surface.

## Evidence

- Stage 1 landed (sprint 69, denkhaus/work#1082): record kind
  `observation.recorded` (`fabro-store`), fold onto
  `StageProjection.observations` (`fabro-petri` projection), tool
  `fabro_observation_record` riding `x.fabro_tools` with the stage
  position bound per session (`fabro-workflow` services + host-tools
  seam), and the engine's presence validation in the vendored petri
  (`x.observations` lowers into the agent config; completed non-error
  tool calls counted per session; a required stage without a call
  fails `bad_output` with a teaching reason) — fork export
  `denkhaus/petri#5361eb31a41f` (review-fix re-export; first export
  3d3f3fbac35e). Pins: vendored
  `tests/observation_contract.rs` (4 arms, mutation-proven),
  frontend-attractor lowering pin (mutation-proven), fabro-petri
  projection fold pin (mutation-proven), fabro-workflow
  `fork_observation_tool.rs` + dispatch pins, envelope lint pins.
  TOUCHPOINTS row added.
