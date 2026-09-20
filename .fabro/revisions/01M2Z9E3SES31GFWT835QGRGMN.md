# Revision — run 01M2Z9E3SES31GFWT835QGRGMN

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2Z9E3SES31GFWT835QGRGMN.md
- seeds filed: none — healthy run
- balance: 0 non-exempt seeds filed / 0 — no credit this pass (no same-pass stale/superseded closes)
- basis: run 01M2Z9E3SES31GFWT835QGRGMN, workflow version 8d3a780e5a0665da9cb87b36306f1ef7e10841bbc767996f563f828c08a277b6, commit 49f9a94df2174fddc21dbda94758e62ec44b3bf8
- revised_at_commit: 49f9a94df2174fddc21dbda94758e62ec44b3bf8 (ADR-0015: engine drift signal for later judgement)

## Findings

- overflow: Prebuild the graph-render binary in `scripts/verify.nu` before nextest — mirror `scripts/qualitygate.nu:206-216` build-renderer-if-needed in `scripts/verify.nu` (~line 132, before the nextest step): run `cargo build -p fabro-cli --bin fabro` whenever fabro-server is in the touched set; effect: no phantom environment-artifact graph-render failures in the implementer lane (`get_graph_returns_svg` / `render_graph_from_manifest_*` forced a git-stash verification plus a duplicated full fabro-server suite run, ~3-4 min + tokens, in run 01M2Z9E3SES31GFWT835QGRGMN); dedupe verified: closed fabro-febd landed only a gate-side skip-guard, sd searches on verify/renderer/prebuild return no seed covering the verify.nu-side prebuild arm. Not filed: zero filing credit this pass (ADR-0022).
