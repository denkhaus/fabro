# Revision — run 01M1YTVK73YEJXW4542MWDX4BJ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1YTVK73YEJXW4542MWDX4BJ.md
- seeds filed: fabro-6f6e — Closeout: warn when the run diff touches a Dockerfile — the fix is inert until the toolchain image rebuilds; fabro-c0a8 — Develop planner: record stale-spec corrections via sd update before claiming; fabro-96bd — Implementer prompt: document the ml record flag contract per record type
- basis: run 01M1YTVK73YEJXW4542MWDX4BJ, workflow version 5603b5e729f24c8e4ab21d9e7599599d18c4c910b381de3682f7a9a6b8628a21, commit 54612515b569d36aa68872cb119b12ae7f9c4eaf
- revised_at_commit: 54612515b569d36aa68872cb119b12ae7f9c4eaf (ADR-0015: engine drift signal for later judgement)

## Findings

### Closeout: warn when the run diff touches a Dockerfile — the fix is inert until the toolchain image rebuilds
- filed: fabro-6f6e
- Change: in `.fabro/workflows/develop/scripts/closeout.nu`, detect a diff touching `Dockerfile*` and emit a run-level warning ("seed closes, but takes effect only after `fabro-toolchain:noble` rebuild") surfaced in the run summary and PR body (the channel open seed fabro-5b0a proposes — complementary, cross-referenced, not superseded). Expected effect: eliminates the recurring class of seeds closed green while the promised behavior stays broken for N subsequent runs.

### Develop planner: record stale-spec corrections via sd update before claiming
- filed: fabro-c0a8
- Change: `.fabro/workflows/develop/prompts/planner.md` step 3 (stale-basis check): when the basis resolves but the spec's named path is wrong, the planner must record the correction via `sd update <id> --description` before claiming. Expected effect: ~40% planner time/token cut on platform-path seeds and the contradiction stops recurring in every future reader's context.

### Implementer prompt: document the ml record flag contract per record type
- filed: fabro-96bd
- Change: document the exact flag set per record type (pattern requires `--name`) in `.fabro/workflows/develop/prompts/implementer.md` step 6, and note the mulch-cli exit-0-on-error bug for upstream filing. Expected effect: one fewer failed tool call + LLM round per lesson capture and deterministic error detection.
