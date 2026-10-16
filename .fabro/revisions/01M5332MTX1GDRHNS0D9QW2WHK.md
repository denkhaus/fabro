# Revision — run 01M5332MTX1GDRHNS0D9QW2WHK

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M5332MTX1GDRHNS0D9QW2WHK.md
- seeds filed: none — zero balance credit this pass (no same-pass stale/superseded closes), all surviving findings journal as overflow per ADR-0022
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M5332MTX1GDRHNS0D9QW2WHK, workflow version absent, commit 809b3ee6e73bd4e24121b7c5e3611ddf7712e03f
- revised_at_commit: 809b3ee6e73bd4e24121b7c5e3611ddf7712e03f (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Drop the tester gate transcript from the loop reviewer preamble** — overflow (no credit this pass).
- overflow: Drop the tester gate transcript from the loop reviewer preamble — in `.fabro/workflows/loop/workflow.fabro` reviewer node, extend `x.preamble_stages_ignore="planner,implementer,reviewer"` to also list `tester`; effect: frees ~12.3 KB of the 48 KB preamble budget so the reviewer sees the complete evidence capture (this run's preamble truncated the tail — exactly the two `reviewer.md` contract diffs under review, adjudicated from the loop-lane copy). Distinct arm of the loop-reviewer-preamble-budget theme already open as "Raise the loop reviewer node's preamble_inline_max_kb 16 → 32" (open in `.fabro/revisions/01M52ZMS6BR0ZSMP80PN5KQ233.md` — complementary mechanism: stage-scoping vs cap raise; next pass should consolidate both arms into one seed). Dedupe (analyst): closed budget lineage fabro-8d2c/fabro-1e9f; no open seed covers tester-stage scoping.

**2. Stop running the lints twice per gate + prompt-lint summary mode** — dropped: overflow-dup: delete the duplicated lint sections from the loop gate (open in `.fabro/revisions/01M43J0MSS6HK1N8396YQAADFT.md`) — same file (`.fabro/workflows/loop/scripts/loop-gate.nu`), same change class, same effect; the summary-mode half rides the same lint-noise theme that overflow already brackets.

**3. Byte-level per-line guard in evidence.nu caps** — overflow (no credit this pass).
- overflow: Add a byte-level per-line guard to `cap-changed-lines` — in `.fabro/workflows/develop/scripts/evidence.nu`, truncate any single changed line past ~512 bytes with a `…line truncated` marker (counted in the disclosure), extending the `cap-changed-lines` battery in `evidence-smoke.nu`; effect: churn/anomaly captures stay under the reviewer's 16 KB inline ceiling even when seed bodies ride single multi-KB lines (`.seeds/issues.jsonl` hunks), closing the residual of fabro-020b whose line-count cap does not bound bytes. Distinct from open overflows "Cap tracker-row diff context in evidence.nu" (`.fabro/revisions/01M531BQ0P3H99XHD5PTMNGXBQ.md`, `-U0` context hunks) and "Elide lockfile diffs" (`.fabro/revisions/01M2ZYVQ7CBSY3T0FEYBE9JK74.md`) — same file, different mechanism; next pass may consolidate as arms. Dedupe (analyst): fabro-020b closed with this residual journaled but unfiled; no open seed covers per-line byte capping.

**4. nu `$\"…\"` literal-paren trap: lint hint + conventions line** — overflow (no credit this pass).
- overflow: Surface the nu `$\"…\"` literal-paren trap as a lint hint and conventions rule — append a hint to the parse-failure diagnostic in `scripts/lint-nu.nu` (literal `(` inside interpolated strings is an expression opener) and add the one-line rule to the nu-conventions section of `.fabro/workflows/develop/prompts/implementer.md` plus the loop copy (one-unit rule); effect: the next implementer fixes paren-in-interpolation in one pass instead of a diagnosis detour (this run: 2 shell + 2 edit_file errors in the stage that was 79% of run cost, $0.45 of $0.57). Dedupe (analyst): closed fabro-88f2 added detection, no hint/convention; open fabro-2904 is bash double-quoting of `nu -c`, a different mechanism; distinct from the open probe-discipline overflows (shell quoting/pipeline recipes, different rules).

**5. Surface non-zero `git diff` exits as `diff-error` lines** — dropped: duplicate_of: fabro-41fa (filed by the target run's own closeout on its unmerged branch; `seeds show fabro-41fa` returns Issue not found in this tree because the filing PR is unmerged — analyst verified in `.seeds/issues.jsonl` on the branch).
