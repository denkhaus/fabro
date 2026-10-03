# Revision — run 01M2Q2Q2NY211PVV0E44YDAAQN

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2Q2Q2NY211PVV0E44YDAAQN.md
- seeds filed:
  - fabro-a32f — Deterministic already-landed preflight before the planner LLM lap
  - fabro-a81d — Chain the superseded-close write pair into one shell call
  - fabro-23d4 — Add an `sd note` append command so closure notes stop rewriting whole seed bodies
  - fabro-8c8a — One-line needs-user label semantics in planner step 2
- basis: run 01M2Q2Q2NY211PVV0E44YDAAQN, workflow version 1e95a3bfe93d5d70d9b6aa6a844359b97607388445120dab1286674bd42e4a86, commit cebd6e682c3c5a66dc1ecb041abbe77cf792660f
- revised_at_commit: cebd6e682c3c5a66dc1ecb041abbe77cf792660f (ADR-0015: engine drift signal for later judgement)

## Findings

### Deterministic already-landed preflight before the planner LLM lap — filed fabro-a32f (priority 1)
Extend `.fabro/scripts/dup-run-check.nu` (or a pre-planner script node in `workflow.fabro`) to grep merge-target base history for each top `sd ready` candidate's seed id and route 'Already landed' mechanically, so the planner adjudicates only the ambiguous residue. Expected effect: sub-second greppable facts stop costing a full run (93.6 s / $0.143 / PR #207 re-derived across three planner probe rounds). Existing seeds cover adjacent segments only (fabro-0da8 tracker-emptiness, fabro-ab38 post-merge sweep, closed fabro-8c75/7280/5656 implementer-side); cross-referenced fabro-a01f.

### Chain the superseded-close write pair into one shell call — filed fabro-a81d (priority 2)
In `planner.md`'s ALREADY LANDED arm, emit `sd update --description && sd close` as one shell call. Expected effect: 1–2 LLM round trips (~15–30 s) removed per closure pass (this run: seq 61 and seq 67 as separate turns with a 20.9 s stall). Extends open fabro-55a7's batching rule from reconnaissance to the close sequence — thematic overlap, not supersession.

### Add an `sd note` append command — filed fabro-23d4 (priority 2)
`sd` gains `sd note <id> <text>`; `planner.md`'s superseded-close procedure switches to it. Expected effect: closure notes stop re-emitting entire seed bodies via `mktemp` heredoc under wholesale `--description` replacement, eliminating silent tracker-history loss from a dropped line.

### One-line needs-user label semantics in planner step 2 — filed fabro-8c8a (priority 2)
Add the claimability rule for `needs-user` seeds (USER DECISION note in body = claimable; the label records the approval gate, not a block). Expected effect: removes a hesitation turn per needs-user candidate (seq 34 this run) and prevents skipping an approved seed as untouchable.
