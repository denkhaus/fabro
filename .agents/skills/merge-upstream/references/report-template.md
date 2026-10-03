# Merge report template

Sections, in order. Keep it factual; every claim traces to evidence
(commits, test runs, smoke output).

1. **Merge identity**: upstream range (old -> new SHA), version, merge
   commit on denkhaus, pushed state.
2. **Verification**: build / fmt / tests (counts, thread setting) /
   clippy / web tests / deploy+smoke result (7 checks), proof the
   container runs merged code, and — when the merge carries data migrations —
   the activation/migration outcome on REAL production data (log lines,
   counts, marker written).
3. **Upstream changes by theme**: group commits; per theme 2-6 sentences:
   what changed, and "what it means for us".
4. **Our-code impact**: conflicts resolved (file, class, resolution),
   call-site adaptations, signature breaks.
5. **Regression status per feature**: for each touchpoint that overlapped
   this merge: green / adapted / at-risk + why. Explicit "no regression"
   statements for the big ones (taxonomy, boundary, retry, smoke/lock).
6. **Obsolescence & adaptation findings**: features/seeds upstream now
   supersedes or should reshape; concrete recommendation per finding
   (close as superseded / port / redesign), and whether a seed was filed
   or updated.
7. **Upstream code quality**: small-form commits, contract-first design,
   migration discipline — trends, not flattery.
8. **Overall direction**: 3-5 bullets where the platform is heading and
   how our fork should ride it.
9. **Workflow changes** (MANDATORY): do our lab workflows need changes
   from this merge? Currently only `develop` (canonical on
   meta/denkhaus-lab, synced to product worlds via sync-check). Cover:
   new engine capabilities the graph should adopt (exit kinds, cycle
   guards, preamble features), prompt/gate adjustments, and whether
   scripts (run_workflow.nu, qualitygate.nu) are affected. If nothing
   changes: say so explicitly with one sentence why.
