Goal: Repro: does the synth/merge node see branch outputs?

## Completed stages
- **fork**: succeeded
- **merge**: succeeded

## Context
- parallel.branch_count: 2
- parallel.fan_in.best_head_sha: 379d13d4f7218e30c9ec925d730f40194c63616b
- parallel.fan_in.best_id: a
- parallel.fan_in.best_outcome: succeeded
- parallel.results: [{"id":"a","status":"succeeded","head_sha":"379d13d4f7218e30c9ec925d730f40194c63616b"},{"id":"b","status":"succeeded","head_sha":"8ee19a3b6be176881b7063c1340d26477c997742"}]


You are the synthesis node after a parallel fan-in. Report VERBATIM every prior branch analysis you can see in your context/preamble. The branches emitted markers BRANCH_A_MARKER_7f3a and BRANCH_B_MARKER_91c2. If you can see those marker texts, quote them exactly. If you canNOT see any branch output text, respond with EXACTLY: NO_BRANCH_OUTPUT_IN_CONTEXT . Then list which context keys you were given.