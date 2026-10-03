# Revision — run 01M2ZEHCA127XFV2CMCVSEZDPJ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2ZEHCA127XFV2CMCVSEZDPJ.md
- seeds filed: fabro-75a5 — Grant the develop GitHub App workflows:write so .github/workflows seed deliverables can publish (ADR-0019, needs-user); fabro-d774 — Blob markers must advertise byte size AND line count; reviewer pages via read_file offset/limit (supersedes fabro-9837)
- balance: 1 non-exempt seed filed (fabro-d774) / 1 same-pass superseded close: fabro-9837. fabro-75a5 is a needs-user (capability) filing and sits outside the balance per ADR-0022 exemption.
- basis: run 01M2ZEHCA127XFV2CMCVSEZDPJ, workflow version 8d3a780e5a0665da9cb87b36306f1ef7e10841bbc767996f563f828c08a277b6, commit 8431c97a4d525dafb07467d31d18c7cb2bf5f8bd
- revised_at_commit: 8431c97a4d525dafb07467d31d18c7cb2bf5f8bd (ADR-0015: engine drift signal for later judgement)

## Findings

### GitHub App workflows permission + pre-claim/park guards (multi-arm, capability split)
Run went green but every run-branch push was rejected 13:22-13:25 ("refusing to allow a GitHub App to create or update workflow .github/workflows/release.yml without workflows permission") — deliverable stranded, $2.09/23min lost, seed falsely closed in-sandbox.
- Capability arm (a): FILED as fabro-75a5 (needs-user,revision, ADR-0019 — implementation awaits explicit user approval).
- Non-capability arms (b)+(c): overflow this pass (no remaining balance credit), see below.

### Blob markers line count + reviewer read_file paging
FILED as fabro-d774 (priority 2). Open fabro-9837's mandated `nu -c 'open --raw | str substring'` first action predates the 2026-09-17 reviewer shell denial and is unexecutable; this run's reviewer approved over silently clipped evidence (~28 tokens lost mid-file). Supersession close of fabro-9837 executed with reason-before-close chaining. Not duplicates: fabro-9a43 (phrase checks), fabro-8cef (plain-text blob), fabro-cf3e (budget) — cross-referenced in the seed.

### Anomaly-file list to judgment-shadow via context key
No duplicate (fabro-8e13 is the hook itself, fabro-deee/fabro-d4c6 are consumers of the stream). Overflow this pass — no credit left.

### Prose/external/URL path-token filter in planner-preflight anchor flags
Both missing_file flags on this run's claimed seed were false positives (external nushell release artifact in prose; `modules/fabro-app/main.tf` living in the separate fabro-tofu repo). Not duplicates: fabro-e8ae (one-char truncation), fabro-4c81 (adds a check), closed fabro-7daf/fabro-7611. Overflow this pass — no credit left.

## Overflow (survived dedupe, no balance credit — ADR-0002/ADR-0022)

- overflow: Multi-arm preflight/publish guards for workflow-file seeds — arms: (b) pre-claim arm in `.fabro/workflows/develop/scripts/planner-preflight.nu` flagging seed bodies targeting `.github/workflows/**` while the App lacks the workflows permission; (c) make `.fabro/workflows/develop/scripts/closeout.nu` park with a 'delivery pending push' note when the last lifecycle push failed instead of a silent clean close; (d) in the anchor arm of `.fabro/workflows/develop/scripts/planner-preflight.nu`, stop flagging path tokens inside parenthetical example prose (lines with 'e.g.'/external-repo markers) or carrying archive/URL forms (.tar.gz, hostnames), flag only repo-relative source-path shapes; effect: this failure class surfaces in the ~12s preflight instead of after a 23-min green run, and anchor flags become trustworthy (2 of 2 false on this run). Consume together with the open planner-preflight overflows (externally_gated arm in 01M2VVFMSQXHK0E3SNV2GNTP43.md, suffix-resolve in 01M2XYE58D9210W751TTJT2P64.md, out-of-repo target in 01M2YMRJQZMC6SVTQJ3JC49CWC.md) as ONE multi-arm seed when balance allows. Pair with fabro-75a5 (the capability grant) for the full fix.
- overflow: Deliver the anomaly-file list to judgment-shadow via a context key — `.fabro/workflows/develop/scripts/evidence.nu` emits the anomaly file list as a small inline-safe context key in addition to the sidecar; `.fabro/scripts/judgment-shadow.nu` adds a resolution tier reading it from context_updates (HookContext has carried them since closed fabro-31b2); effect: fabro-8e13's per-file residue question group actually fires in production (with sandbox=false the sidecar in sandbox `.fabro/evidence/` never reaches the prod server container), so the wave-2 evaluation (open fabro-d4c6) measures the real question set.
