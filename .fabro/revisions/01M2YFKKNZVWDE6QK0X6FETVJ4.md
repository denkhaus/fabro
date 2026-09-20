# Revision — run 01M2YFKKNZVWDE6QK0X6FETVJ4

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2YFKKNZVWDE6QK0X6FETVJ4.md
- seeds filed: none — zero balance credit this pass (no same-pass stale/superseded closes); all surviving findings journaled below
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M2YFKKNZVWDE6QK0X6FETVJ4, workflow version 13a6957f6e164d2beeb8885df642782f7271c4c, commit 071cc1a9a3d5759d9bf1e1e1183484200f5a6656
- revised_at_commit: 071cc1a9a3d5759d9bf1e1e1183484200f5a6656 (ADR-0015: engine drift signal for later judgement)

## Findings

### 1. Briefs must annotate quoted insert-text (exact bytes incl. case); copyedit the lowercase 'a' at planner.md line 30

Not filed — no balance credit. Journaled as overflow below for the next pass.

- overflow: Briefs must annotate quoted insert-text (exact bytes incl. case) + copyedit lowercase 'a' at `.fabro/workflows/develop/prompts/planner.md` line 30 — add a rule to planner.md step 7 (contradiction check): when a brief quotes text for verbatim insertion, normalize the quote or annotate the exact-bytes requirement including capitalization, and fix the merged broken prose ('…fewest blockers. a needs-user seed…') at line 30; effect: no more wasted capitalize-then-re-edit cycles and every future planner pass reads grammatical instructions. Dedupe (this pass, analyzer-verified): no existing seed covers quoted-insert grammar (closed fabro-cf76/fabro-c0ca = anchors/insertion points, open fabro-d20f = heading citations, fabro-7773 = behavior contradictions; reviewer-note part covered by open fabro-89dd).

### 2. Demote the by-design `context_update_dropped: output.planner` warn to info in fabro-workflow

Not filed — same theme already open in the overflow ledger.

- overflow-dup: add `output.planner` to the planner node's context_allow_keys / skip self-node drops (open in `.fabro/revisions/01M2WCNY3WGP4QFZ24KPACBTND.md`) — the engine-side demote-to-info in `lib/components/fabro-workflow` proposed here targets the same per-pass warn noise via a different mechanism; next pass should consolidate both arms into one seed and re-run dedupe (adjacent: open fabro-7028 same warn class on gatebounce; closed fabro-b907 fixed the dedup mechanism, not the warn level).

### 3. Make prompt-lint warnings decision-backed: date-pin review status and allowlisted routing-named schema properties

Not filed — same theme already open in the overflow ledger.

- overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in `.fabro/revisions/01M2XQJC3TWRMJ1128WRXRQYXH.md`) — that entry already covers allowlisting routing-intent schemas (`planner-output.schema.json`, conductor schema per fabro-9ec3) and the date-pin "older than 45 days" nag for the pinned toolchain (`2026-04-14`, `.fabro/scripts/prompt-lint.nu` line 142); this finding's decision-backed date-pin framing (reviewed-on date or escalation) should merge as an arm of that entry when the next pass files it.
