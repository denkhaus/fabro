# Revision — run 01M2Q9JJBYPN62NDN0DHHWNHT1

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2Q9JJBYPN62NDN0DHHWNHT1.md
- seeds filed: fabro-0d48 — Fix dup-run-check filed-only misclassification: revisor filing commits must not drive a duplicate verdict
- seeds filed: fabro-2ade — Fix planner-preflight.nu parse-time type_mismatch that dead-lands the preflight node
- basis: run 01M2Q9JJBYPN62NDN0DHHWNHT1, workflow version f451989b6dd39304d6e41db1810163069ec25f26057a791fc0c7397cc1e94c00, commit 6a422b807c8ea8b778354ff2aa1ff7cc0ea7ee66
- revised_at_commit: 6a422b807c8ea8b778354ff2aa1ff7cc0ea7ee66 (ADR-0015: engine drift signal for later judgement)

## Findings

### Fix dup-run-check filed-only misclassification (filed: fabro-0d48)

In `.fabro/scripts/dup-run-check.nu` the filed_only subject classifier (line 69)
missed the real filing-pass shape — commit 706d639 matched no regex branch (they
require 'file <count> seeds' or '; file fabro-'), so it landed among
implementations with closure=foreign and line 244 (foreign_impl > 0) forced
verdict=duplicate even though the true closing evidence 6805322 (PR #213) is the
target run's own self-closure. It hit the implementer in-run (events seq 89–100:
~19 s and $0.036 in override probes, near-Blocked) and this revisor pass's Step
3.5 check identically. Change: extend the classifier (diff-scope check or the
comma-separated file-seeds subject shape) and pin the shape in
`.fabro/scripts/dup-run-check-fixtures.nu`. Expected effect: no false Blocked
routes and no wrongful superseded-closes on freshly filed seeds. Not a
duplicate: closed fabro-8c75 (merge-commits-only matching) and closed fabro-e6a0
(closure identity) do not cover the subject-classifier gap; open fabro-a01f is
the opposite missed-duplicate direction.

### Fix planner-preflight.nu parse-time type_mismatch (filed: fabro-2ade)

In `.fabro/workflows/develop/scripts/planner-preflight.nu:135` the declaration
`mut closed = {seed: null, sha: null}` infers record<seed: nothing, sha:
nothing>, so the string-record assignment at line 152 raises
nu::shell::type_mismatch 1.3 s into every run (first-ever node execution, PR
#209); the node has never produced a verdict table, the planner re-derived
already-landed checks by hand (7 tool calls, 6 LLM rounds, $0.092
second-costliest stage), and every downstream prompt carried the failed-stage
dump. Change: initialize with empty strings (test non-empty at line 161) or
build the record as a let expression. Expected effect: the fabro-a32f saving
(93.6 s, $0.143 per stale-tracker run) materializes and preambles stop carrying
the dead node's error. Ordering note: land with or after the dup-run-check
filed-only fix so the revived node does not inherit the false-duplicate
verdict. Not a duplicate: no existing seed covers type_mismatch; fabro-a32f,
which introduced the node, is closed and did not cover this fix.
