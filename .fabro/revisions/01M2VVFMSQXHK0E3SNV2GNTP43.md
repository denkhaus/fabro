# Revision — run 01M2VVFMSQXHK0E3SNV2GNTP43

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2VVFMSQXHK0E3SNV2GNTP43.md
- seeds filed: none — no credit this pass (ADR-0022), 1 surviving finding journaled as overflow
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2VVFMSQXHK0E3SNV2GNTP43, workflow version 2edfd65e52fc7543478409763d768a38506d2a554f690d1f98efed0a5be796b0, commit 95fd5fa619b95e2c1f3b2c1c63fb624d2fd98e3c
- revised_at_commit: 95fd5fa619b95e2c1f3b2c1c63fb624d2fd98e3c (ADR-0015: engine drift signal for later judgement)

## Findings

### Park upstream-PR-gated seeds mechanically in the planner preflight
- filed: none — overflow to journal (ADR-0022: 0 credit this pass, not capability-affecting, not needs-user)
- overflow: Park upstream-PR-gated seeds mechanically in the planner preflight — add an `externally_gated` arm to `.fabro/workflows/develop/scripts/planner-preflight.nu` that reads a `Gate: upstream PR <url>` line from seed bodies and parks them like `in_flight`, plus one `sd update fabro-af22` adding that line now. Expected effect: -60s and ~$0.13-0.15 per develop run until upstream PR fabro-sh/fabro#784 merges (basis: run 01M2VVFMSQXHK0E3SNV2GNTP43, planner burned ~60s / 8 tool calls at 03:31:37-03:32:37 re-deriving the gate on fabro-af22). Dedupe verified this pass: sd searches on "preflight" and "upstream" show no seed covering mechanical parking of upstream-PR-gated seeds (fabro-9372/fabro-d9f7/fabro-06e0/fabro-91ff/fabro-a285 govern different exclusion sources). Next pass may re-file against its own balance after re-running its own dedupe.
