# Revision — run 01M2K8TECWAHRR79PQ63V2C0ZP

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2K8TECWAHRR79PQ63V2C0ZP.md
- seeds filed: fabro-96e7 (evidence.nu spec-path check before loop-asset anomaly), fabro-2be2 (planner: batch claim reconnaissance probes), fabro-66bc (planner: top-N priority-sorted sd ready), fabro-2b1b (drop hand fmt/clippy triplet, rely on just verify implementer)
- basis: run 01M2K8TECWAHRR79PQ63V2C0ZP, workflow version 33dfa66ad1d4e13738b9797918d1ac298a029fa4fcd7e02edddc0fef394bb445, commit ef4cb924b4a73fec65d75474b06b4574ae565689
- revised_at_commit: ef4cb924b4a73fec65d75474b06b4574ae565689 (ADR-0015: engine drift signal for later judgement)

## Findings

- Evidence.nu: check seed-spec-cited paths before flagging loop-asset anomalies — filed fabro-96e7. Diff the loop-path set against paths cited in the claimed seed spec before emitting the anomaly section; spec-compliant loop-asset seeds stop generating false residue alarms and false "Changes requested" cycles.

- Planner: batch claim reconnaissance probes into one shell call — filed fabro-2be2. planner.md steps 3-4 prescribe one combined probe shell (anchors + cases + git-log grep) before the claim; ~5 fewer LLM round trips per run, est. -30-40 s wall, -25-35% planner cost.

- Planner: use a top-N priority-sorted sd ready invocation — filed fabro-66bc. Replace the 200-seed `sd ready --assignee fabro --limit 200` pour with a top-N priority-sorted invocation; smaller planner context, cheaper turns, less truncation noise.

- Drop the hand fmt/clippy triplet from briefs; rely on `just verify implementer` — filed fabro-2b1b. implementer.md step 4 and planner.md step 7 prescribe `just verify implementer` plus only genuine supplements; one fewer compile pass per implementer stage, shorter briefs, no verify-vs-prose conflict.
