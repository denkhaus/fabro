# Revision — run 01M0QTNF6GWHGPKHSQ2Y2PB8CQ

- status reviewed: failed
- review: .fabro/reviews/develop/01M0QTNF6GWHGPKHSQ2Y2PB8CQ.md
- seeds filed: fabro-4601 — Tell the implementer to edit one file sequentially, never as parallel batches

## Findings

### Tell the implementer to edit one file sequentially, never as parallel batches
- filed: fabro-4601
- Change: add one line to the output-hygiene section of `prompts/implementer.md` requiring sequential edits to the same file (or a single `write_file` of final content).
- Expected effect: fewer wasted tool round trips per implementer stage; concurrent-write lock warnings disappear from run logs.
- Evidence: run 01M0QTNF6GWHGPKHSQ2Y2PB8CQ worker log 17:31:51 — six `WARN fabro_agent::write_locks: concurrent write to the same file in one batch; serializing` events from ~6 parallel `edit_file` calls at `fib_test.go`.
- Distinct from fabro-0bfd (engine log rendering) and fabro-facd (harness clobber fix).
