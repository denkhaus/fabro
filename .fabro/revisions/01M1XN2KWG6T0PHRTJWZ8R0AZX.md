# Revision — run 01M1XN2KWG6T0PHRTJWZ8R0AZX

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1XN2KWG6T0PHRTJWZ8R0AZX.md
- seeds filed: fabro-56f4 (match gate-red failure tails against open known-bug seeds on the tester→implementer bounce), fabro-4815 (qualitygate exit status distinguishing deterministic vs transient infra), fabro-93a7 (classify seed-spec-cited paths as seed-work in evidence.nu), fabro-cf3e (raise develop reviewer preamble_inline_max_kb 16→32)
- basis: run 01M1XN2KWG6T0PHRTJWZ8R0AZX, workflow version 8afe037fc8c61680097f808a27f66ee20da8b546373f6e6e69ad4a09aee94a98, commit 5ff61b59afe4f2afa5fbcf28bb4730bc8ccfb83d
- revised_at_commit: 5ff61b59afe4f2afa5fbcf28bb4730bc8ccfb83d (ADR-0015: engine drift signal for later judgement)

## Findings

### Match gate-red failure tails against open known-bug seeds on the tester→implementer bounce — filed fabro-56f4
Deterministic pre-step on the `Gate red` edge matching the failure tail against open workflows-labelled seeds, attaching matching seed bodies to the implementer brief. Run evidence: tester@1 red at 10:34 on the fabro-febd failure the planner had already read at 10:02, yet implementer@2 (events 672–910) re-derived it from a 186 KB log blob (704 s, $0.38; same as run 01M1VZTJSZ55). Expected: bounce passes start at the root cause; eliminates the ~12 min/~$0.38 whole-revisit class. Complements fabro-febd; not a duplicate.

### Exit qualitygate with a status that distinguishes deterministic failures from transient infra — filed fabro-4815
`scripts/qualitygate.nu` red exits become distinguishable (deterministic miss vs infra flake) so checkpoint classification stops recording deterministic failures as `failure_class: "transient_infra"` (as happened with the fabro-febd failure this run). Expected: deterministic bugs not treated as retryable noise; Gate-red bounce can prefer known-bug matching. Distinct from fabro-e988 (summary printing only).

### Classify seed-spec-cited paths as seed-work in evidence.nu — filed fabro-93a7
`.fabro/workflows/develop/scripts/evidence.nu` classifies a changed file as seed-work when its path is named in the seed spec/brief, even under `.fabro/` or `docs/`. Run evidence: `docs/public/api-reference/fabro-api.yaml` and ` .fabro/workflows/conductor/prompts/` legs — three of eight acceptance criteria — were bucketed as loop churn, forcing a manual `git diff 422fbb0..HEAD` by the reviewer. Expected: reviewer verifies all criteria from the capture. Broader than closed fabro-2992 (seed target paths only).

### Raise the develop reviewer node's preamble_inline_max_kb from 16 to 32 — filed fabro-cf3e
Per-node cap 16→32 on the reviewer node. Run evidence: 25.8 KB evidence capture demoted to a blob ref at 2.1% context usage — one extra `read_file` round trip and unread-blob rejection risk. Complements open fabro-8d2c (graph budget half); knob implemented by closed fabro-9467. No duplication found (fabro-9467 is closed and this is a new value change).
