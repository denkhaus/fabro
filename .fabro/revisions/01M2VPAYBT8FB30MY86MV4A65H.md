# Revision — run 01M2VPAYBT8FB30MY86MV4A65H

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2VPAYBT8FB30MY86MV4A65H.md
- seeds filed: none — 0 credit this pass, both findings journaled as overflow
- balance: 0 non-exempt seeds filed / 0 same-pass stale-superseded closes — no credit this pass (ADR-0022)
- basis: run 01M2VPAYBT8FB30MY86MV4A65H, workflow version 2edfd65e52fc7543478409763d768a38506d2a554f690d1f98efed0a5be796b0, commit fc5d76dfed0ced51d5388586e5d7723629b65f48
- revised_at_commit: fc5d76dfed0ced51d5388586e5d7723629b65f48 (ADR-0015: engine drift signal for later judgement)

## Findings

### Cover the implementer's direct nextest calls with a just mtime-touch recipe named by the brief
- filed id: none — overflow to journal (zero credit this pass, ADR-0022)
- Concrete change: extend open fabro-7d98 (touch only at verify.nu start) and complement fabro-a1ef; expose the mtime normalization as a `just` recipe or pre-step and name it in the implementer brief's verification bullets so every nextest invocation runs on fresh binaries, per fabro-9ec3 standing policy (in a script, not prose). Expected effect: eliminates misleading-red diagnosis cycles in the costliest stage ($2.05 of $2.26 run cost). This run's implementer hit the stale-nextest-binary trap on direct cheapest-first `cargo nextest` calls (after editing `test_support.rs` and `initialize.rs`) — journal painpoint, fixed only by touch.

### Record a durable WAITING marker on upstream-blocked seeds for the next planner
- filed id: none — overflow to journal (zero credit this pass, ADR-0022)
- Concrete change: when a planner adjudicates a seed as waiting-on-external, append a `WAITING: upstream #<n>, recheck before planning` line via `sd update <id> --description`, and/or have `planner-preflight.nu` annotate a waiting_upstream flag parsed from that marker in the verdict table. Expected effect: subsequent planners skip re-derivation (~30-40 s and 3 tool calls saved per run while fabro-sh/fabro#784 pends). Evidence: this run's planner spent ~half its 85 s (2 LLM rounds + 3 shell calls, events seq 45-56) re-deriving that fabro-af22's fix lives in upstream #784. Not duplicated by fabro-06e0/91ff/6b58 or fabro-d9f7 (run/PR claim hygiene, not externally-blocked seeds).
