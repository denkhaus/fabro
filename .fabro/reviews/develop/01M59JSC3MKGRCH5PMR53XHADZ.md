# Improve review — run 01M59JSC3MKGRCH5PMR53XHADZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (15m07s, revisor pass — reason and cost in run detail)
- generated: 2026-10-19 13:36+0000 by revisor `fabro_ask`

---

## Basis for this review

Run `01M59JSC3MKGRCH5PMR53XHADZ` (loop lane, seed `fabro-52b4`, succeeded, 15m07s wall / $0.0683, PR #437). All evidence below is from run events and stage transcripts (planner/implementer/reviewer/tester/closeout outputs, timings, tool stats), plus workspace files where a file to change is named.

**Tracker de-dup note:** every live `fabro_seeds_search` query failed with 503 `seeds_source_unconfigured` (queries attempted: `seed-spec-paths anecdote`, `newline`, `firehose`, `squash basis`, `battery implementer lint`, `loop project-facts`, `legend preflight`, `title stale seed`). I therefore de-duped against the same tracker data read-only from workspace file `.seeds/issues.jsonl` (greps: `seed-spec-paths`, `firehose|top-N`, `squash|basis_commit`, `merge_strategy|strand`, `set-title|stale title`, `legend`, `newline|prompt-lint`, `heartbeat`, plus the 58-row `seeds ready` dump captured in the planner transcript). Seed ids below are verified from that file.

---

### 1. Trim the planner's firehose inputs — seed **fabro-c3b4** (open, @loop; sibling **fabro-6b58**)
- **What happened:** planner's first tracker call returned all 58 ready seeds (~8.4KB inlined, planner transcript 08:00:47) just to pick one; its `fabro_run_search {workflow: loop}` in-flight check then returned 20 full run records (large JSON, truncated in the stream) when only status + recency matter. Planner stage: 44,513 input tokens, 155s.
- **Change:** `.fabro/workflows/loop/prompts/planner.md` — command table switches to a priority-bounded `seeds ready` view, and step 5 (IN-FLIGHT EXCLUSION) passes `created_since` + status filters to the run search, exactly per fabro-c3b4's 2026-10-11 rewrite (and fabro-6b58's self-exclusion).
- **Effect:** several KB less planner context every pass, faster first output, fewer distraction rows during tie-breaks.

### 2. Make "implementer never runs the battery" mechanical — **new seed**
- **What happened:** the implementer's recorded checks include `just lint-nu && nu .fabro/scripts/prompt-lint.nu` (67 scripts + 81 files, full output inlined into its context and the evidence capture), and the tester re-ran the *identical* battery green 14 seconds later (tester output 08:14:00–08:14:15). The planner brief itself prescribed the parse tier even though `planner.md` carries the prose rule ("the double battery run cost this lane 21.6s per run before this rule — run 01M550CAX").
- **Change:** teach `.fabro/workflows/develop/scripts/check-transcript.nu` to refuse battery-tier commands when `--by implementer` (or reject battery literals in the planner output schema).
- **Effect:** the existing rule stops depending on the brief author remembering it; saves the duplicated lint run plus its ~5KB token echo in implementer context and evidence on every implement pass.
- **New-seed justification:** no open or closed seed enforces this mechanically — the 01M550CAX-derived rule exists only as planner-prompt prose, and this run proves the prose gets bypassed.

### 3. Filter anecdotal path citations in seed-spec-paths — **new seed** (adjacent: closed **fabro-48f3**)
- **What happened:** implementer journal observation 1: the seed-spec-paths tier extracted `.seeds/issues.jsonl` from `fabro-52b4`'s body out of a stale PR-retirement anecdote ("422 on .seeds/issues.jsonl x3"), so any run of this seed REDs unless the tracker file is in the diff — this run passed only because the planner's claim/correction happened to write it (gate output: "1 loop-asset path(s) named, 0 missing … named .seeds/issues.jsonl").
- **Change:** in the shared spec-path extractor (fixtures in `.fabro/scripts/seed-spec-paths-fixtures.nu`), drop paths cited inside anecdote/provenance context (retired-PR narratives, Basis-line history paragraphs) rather than spec demands.
- **Effect:** removes a standing false-RED class for every future pass on seeds whose bodies cite the tracker anecdotally.
- **New-seed justification:** fabro-48f3 fixed the directory-token and creation-window false-REDs but explicitly accepted this class as out of scope ("a path quoted inside prescribed added CONTENT … stays a planner body-correction").

### 4. Stop stranding Basis commits: merge instead of squash — **new seed** (adjacent: closed **fabro-d95c**)
- **What happened:** this run's preflight shows 3 of 5 candidates (`fabro-11d3`, `fabro-7f58`, `fabro-b6f9`) with `basis_commit_resolves=false`; the legend's own text names the cause — "squash-merged run branches strand their basis commits" — and this run's spec sets `pull_request.auto_merge=true, merge_strategy="squash"`, so it manufactures more of them (this run's PR #437 will strand its Basis shas next).
- **Change:** `.fabro/workflows/loop/workflow.toml` → `merge_strategy = "merge"` (or `rebase`) for the loop lane.
- **Effect:** Basis-line shas resolve in the object DB; the planner stops adjudicating every basis via files/worktree workaround on each pass.
- **New-seed justification:** fabro-d95c only made the dead sha advisory (flag + legend); no seed changes the merge strategy (greps `merge_strategy|strand`, `squash|basis_commit`).

### 5. Mirror the per-role command-table split into the loop lane — **new seed** (mechanism from this run's `fabro-52b4`; adjacent open **fabro-aeb7**)
- **What happened:** implementer journal observation 3: `.fabro/workflows/loop/prompts/project-facts.md` still ships the full claim/close/re-status table to all three roles — visible live in this run's planner preamble — i.e., the loop reviewer/implementer still get advertised the write commands develop just stopped showing (measured there: reviewer −1692B, implementer −1335B).
- **Change:** port the `tracker-commands-planner.md` / `tracker-commands-implementer.md` include pattern into `.fabro/workflows/loop/prompts/` (fabro-aeb7's assignee-parameterized fragment is the right vehicle).
- **Effect:** read-only roles stop seeing claim/close forms (correctness hygiene), ~1–1.7KB less prompt per role per stage in this lane too.
- **New-seed justification:** fabro-52b4 was deliberately develop-scope; fabro-aeb7 covers cross-workflow *dedup* of the table, not per-role subsets — no seed demands the loop-lane split.

### 6. Trailing-newline check in prompt-lint — seed **fabro-6b0f** (open, @loop, filed by this run's closeout)
- **What happened:** both new includes (`tracker-commands-planner.md`, `tracker-commands-implementer.md`) shipped with `\ No newline at end of file` (visible in the evidence diff); the reviewer flagged it non-blocking; prompt-lint passed both.
- **Change:** add an EOF-newline check to `.fabro/scripts/prompt-lint.nu` + fixture (per fabro-6b0f).
- **Effect:** this exact nit class dies at gate time instead of surviving to reviewer nit / residual seed.

### 7. Long-stage heartbeat — seed **fabro-b769** (open, @loop)
- **What happened:** the implementer stage ran 640s — 70.5% of the 907s run wall — with `fidelity=summary:high` and no interim output surfaced; planner + reviewer add another 217s. Nothing is visible for ~11 minutes of a scheduled pass.
- **Change:** adopt fabro-b769 (heartbeat notification for stages exceeding a threshold), leveraging the per-stage timing already in run events.
- **Effect:** run visibility during the silent implementer stretch; less "is it stuck?" polling of the UI.

### 8. Seed-title correction path — **new seed**
- **What happened:** the planner corrected `fabro-52b4`'s body pre-claim (`sd`→`seeds`, dropped dead `memory/AGENTS.md` citation) but the title still reads "…sd-command table … sd show only…" — the stale CLI name the body just fixed (implementer journal observation 2). Titles feed `seeds ready` matching, preflight, and briefs, so they rot visibly every pass.
- **Change:** extend the planner's stale-spec correction arm (`.fabro/workflows/loop/prompts/planner.md` step 3 / `seeds` CLI) with a title-correction form, or have `claim-check.nu` flag stale title terms alongside stale body paths.
- **Effect:** titles stop contradicting corrected bodies; candidate matching and briefs stay truthful.
- **New-seed justification:** no seed covers seed-title staleness (greps `set-title`, `title rot`, `stale title` empty; ADR-0030 is about *run* titles).

### 9. Slim the preflight legend's triple serialization — **new seed** (adjacent: closed **fabro-ead4**, closed **fabro-bd55**)
- **What happened:** the static verdict legend (~2.3KB) is the majority of the preflight's 3.8KB stdout, and it rides three times per run — raw stdout, `command.output`, and the `output.preflight` context update — plus once more inlined in the planner's preamble (planner UserInput transcript).
- **Change:** in `.fabro/workflows/develop/scripts/planner-preflight.nu`, emit a one-line legend pointer (or version-hash) instead of the full text in per-run output; keep the full legend in the script header/docs.
- **Effect:** ~2KB× fewer bytes into planner context every pass at zero information loss (legend text is static per lane).
- **New-seed justification:** ead4 *added* the legend and bd55 made it lane-scoped; nothing proposes slimming its per-run serialization.

---

**Not recommended despite being observable:** model change or reasoning-effort tuning (cost is already $0.068/run and inference-dominated — the levers above cut tokens/latency without touching the model), and the `fabro_run_set_title` retry (2 calls, 1 error — the 100-char cap is already documented in the prompt; one cheap retry is below the noise floor).
