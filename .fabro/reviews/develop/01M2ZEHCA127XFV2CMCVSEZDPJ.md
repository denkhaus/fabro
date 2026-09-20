# Improve review — run 01M2ZEHCA127XFV2CMCVSEZDPJ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (23.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 13:31+0000 by revisor `fabro_ask`

---

## Recommendations for the develop workflow, from run 01M2ZEHCA127XFV2CMCVSEZDPJ (seed fabro-8e13)

Ordered by expected impact. Evidence per item is from this run's checkpoints/stage records, the worker warn log, and the journal; seed coverage was checked against `.seeds/issues.jsonl`.

---

### 1. Fix the stranded-delivery class: GitHub App lacks `workflows` permission, so this run's green work never left the sandbox
**What happened:** The run went green end-to-end (gate green in 107 s, reviewer approved, closeout closed fabro-8e13 at 13:25:15) — but every push of the run branch was rejected by GitHub, first at 13:22:12 through 13:25:20 (worker warn log): `refusing to allow a GitHub App to create or update workflow .github/workflows/release.yml without 'workflows' permission`. The seed legitimately touched `.github/workflows/release.yml` (nu staging), so the whole $2.09 / 23-minute deliverable is stranded on a local-only branch, `pull_request: null`, status `succeeded(publish_blocked)` — and the tracker now says the seed is **closed** as if delivered.
**Change:** Two-part: (a) grant the `fabro-the-factory[bot]` GitHub App `workflows: write` (one-time org setting); (b) add a deterministic pre-claim arm in `.fabro/workflows/develop/scripts/planner-preflight.nu` that flags seed bodies naming targets under `.github/workflows/**` while the App lacks the permission, and make `closeout.nu` append a `delivery pending push` note (or park) when the last lifecycle push failed, instead of a silent clean close.
**Expected effect:** eliminates the "green run, zero delivery, seed falsely closed" class entirely; this run's exact failure would surface in the ~12 s preflight instead of after 23 minutes.
**Seed:** NEW SEED — justification: no open seed covers the `workflows`-permission push rejection; closed fabro-696c was the generic PR-credential gap and was verified fixed (PRs #12–#20 published). The publish-blocked *taxonomy* exists engine-side (fork_terminal_taxonomy), but no seed covers the permission grant, the early preflight arm, or the closeout semantics.

### 2. Slim the evidence capture (fabro-020b) and fix the reviewer blob-read protocol (amend fabro-9837 — its remedy is now impossible)
**What happened:** The evidence capture was 48.9 KB — just over the 48 KB graph budget — because 9 loop-churn files rendered as **full diffs** (the `.seeds/issues.jsonl` hunks embed multi-KB seed bodies). It demoted to a blob ref; the reviewer (tools: `read_file`, `grep`, `glob` only, shell denied per fabro-269d) paged it with 4 `read_file` calls and still got an output-truncation warning that **clipped ~28 tokens mid-file inside `judgment_shadow.rs`** (reviewer journal painpoint) — an approval issued over silently clipped evidence.
**Change:** Adopt **fabro-020b**: in `.fabro/workflows/develop/scripts/evidence.nu`, render loop-churn/anomaly files as numstat + first ~30 changed lines, keeping full diffs for seed-work files only. Separately amend **fabro-9837**: its mandated first action (`nu -c "open --raw … | str substring"`) was written 2026-09-10, before the 2026-09-17 shell denial — replace it with (a) blob markers advertising byte size **and line count**, and (b) a reviewer-prompt prescription to page via `read_file` offset/limit computed from that line count.
**Expected effect:** typical captures land under the reviewer's 16 KB inline ceiling (no blob detour — fabro-020b measured a 68.8 s paging detour on the prior occurrence); when blobs are unavoidable, the reviewer pages deterministically and never approves clipped content.

### 3. Bridge the anomaly-file channel into HookContext — half of wave-1's question set is structurally dead in production
**What happened:** The implementer journal records it precisely: the `sandbox=false` host hook cannot read the run sandbox filesystem, so `judgment-shadow.nu` resolves anomaly files tiered (sidecar → host git walk → **empty residue group**). In the prod server container there is no checkout and the sidecar (written to the sandbox's `.fabro/evidence/`) never reaches `/storage/evidence` — so question group (a), the per-file residue adjudication that is the main novelty of fabro-8e13, can never fire in production; only the verdict pre-screen would.
**Change:** In `.fabro/workflows/develop/scripts/evidence.nu`, emit the anomaly file list as a small structured context key (inline-safe, a few hundred bytes) in addition to the sidecar; in `.fabro/scripts/judgment-shadow.nu`, add a tier that reads it from `$ctx.context_updates` (HookContext already carries context_updates, fabro-31b2). No engine change needed.
**Expected effect:** residue questions fire in production, so the wave-2 evaluation (fabro-d4c6) measures the real question set instead of validating a permanently-empty group.
**Seed:** NEW SEED — justification: fabro-d4c6 (evaluation) and fabro-deee (reviewer surfacing) consume the stream; neither fixes the delivery channel. No open seed covers it.

### 4. Filter preflight anchor false positives — both flags on the claimed seed were bogus
**What happened:** The preflight verdict table flagged fabro-8e13 with `missing_file` for `nu-0.115.0-x86_64/aarch64-unknown-linux-musl.tar.gz` (an external nushell release artifact named in prose) and `modules/fabro-app/main.tf` (lives in the separate fabro-tofu repo). The planner correctly adjudicated both as false positives (planner journal), but the flags add adjudication work to every planner lap, and a less careful pass could reject a valid top-priority seed over them.
**Change:** In `.fabro/workflows/develop/scripts/planner-preflight.nu` (anchor arm), stop flagging path tokens that (a) sit inside parenthetical example prose (lines containing `e.g.` / external-repo markers) or (b) carry archive/URL forms (`.tar.gz`, hostnames); flag only repo-relative source-path shapes.
**Expected effect:** anchor flags become trustworthy signal; planner laps stop re-adjudicating false `missing_file` rows (2 of 2 flags false on this run's claimed seed).
**Seed:** NEW SEED — justification: closed fabro-7daf (anchor content checks) and closed fabro-7611 (crate-relative resolution) don't cover the external/prose-path class; open fabro-e8ae is a different bug (one-char truncation).

### 5. Vendor a nu style-guide skill mirroring the rust-style-guide gate
**What happened:** The implementer was 87% of run cost ($1.831 of $2.094; 989 s inference vs 101 s tool time, 79 shell calls with 2 errors) and hit **four distinct nu-0.115 parse gotchas** while authoring `judgment-shadow.nu` (recorded as mx-513300; e.g. `describe` returning `record<...>`, parens-in-`$"…"` parsed as interpolation). The project enforces a mandatory prior read of the vendored rust-style-guide for `*.rs` diffs, but nothing equivalent exists for `*.nu` — the knowledge lives only in the just-created `loop-scripts` expertise domain.
**Change:** Vendor `.fabro/skills/nu-style-guide/SKILL.md` from mx-513300 plus the fabro-88f2 parse-vs-runtime regex class, and add one sentence to the guide-gate paragraph in `.fabro/workflows/develop/prompts/implementer.md` making it a prior read for `*.nu` diffs.
**Expected effect:** nu-heavy seeds stop re-discovering the same four parse gotchas per pass; fewer authoring/recovery shell rounds inside the most expensive stage.
**Seed:** NEW SEED — justification: closed fabro-88f2 is the gate-side lint tier, open fabro-2904 is prompt-embedded quoting; no seed covers a vendored authoring-time nu guide.

---

**What I could not inspect:** the remaining 144 warn lines (only the 60 newest returned) — the visible sample is homogeneous push failures, so the analysis is unaffected; and `sd` itself was not runnable from this read-only context, so seed coverage was verified by grepping `.seeds/issues.jsonl` directly.
