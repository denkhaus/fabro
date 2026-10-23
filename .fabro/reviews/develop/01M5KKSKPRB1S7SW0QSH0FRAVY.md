# Improve review — run 01M5KKSKPRB1S7SW0QSH0FRAVY

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (11m40s wall / $0.855, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 07:52+0200 by revisor `fabro_ask`

---

## What actually happened in this run (basis for every recommendation)

From run events and stage records: run succeeded in 11m40s wall / $0.855 total. Implementer = 398s and $0.597 (70% of cost); planner = 164s/$0.179 with only 1.5s of tool time — the rest was adjudication churn; tester = 54s producing 8.8KB output; evidence capture = 59KB; reviewer = 45s/$0.079. Two of the planner's three `fabro_run_set_title` calls were rejected with **"run title must be at most 100 characters"** (events seq 233–242) before a 97-char title landed. The preflight returned a false **`duplicate` verdict on fabro-4653** (trigger: no-changes verification commit a3f0834) plus a false `missing_file` anchor on `loop/workflow.fabro`, forcing a planner body-correction cycle. The reviewer had to re-read all 6 changed files itself because the 48KB preamble ceiling cut the planner.md diff out of the evidence capture (reviewer journal, from stage transcript).

Tracker searches run: `fabro-c03a`, `fabro-a827`, `fabro-17fb`, `fabro-9337`, `title`, `preamble`, `evidence capture`, `stale comment`.

## Recommendations (by expected impact)

**1. Fix dup-run-check's no-op-commit misclassification — item fabro-c03a** (open, @loop; the implementer's journal painpoint this run already assigns it here).
Change: in `.fabro/scripts/dup-run-check.nu`, classify commits whose diff vs base is empty (or subject declares "no changes") as mention-only, not implementation matches; same unit should fix the anchor resolver that flagged the resolvable `.fabro/workflows/loop/workflow.fabro` as `missing_file` for `loop/workflow.fabro`.
Effect: eliminates the false `duplicate` verdict and the pre-claim `--set-description` correction cycle the planner ran this run — directly cuts planner adjudication time (162s inference for what should be a short pass) and removes a recurring per-run trap for every previously-verified seed.

**2. Diff-scope prompt-lint output in the loop gate — item fabro-a827** (open, @loop; its title literally names this run's 8.8KB tester output).
Change: `.fabro/workflows/loop/scripts/loop-gate.nu` (via the shared battery-runner) should pass the run's changed-file list to prompt-lint and print only warnings for diff-touched files, plus a one-line "N pre-existing warnings suppressed" summary.
Effect: tester output drops from 8.8KB/37 all-repo warnings to a few hundred bytes for a loop-only diff; the same lint block is also re-inlined in the evidence capture, so this shrinks two downstream contexts at once.

**3. Dedupe tester section vs evidence recorded checks — item fabro-17fb** (open, @loop).
Change: in `.fabro/workflows/develop/scripts/evidence.nu --lane loop`, drop the recorded-checks re-inline of any check whose output already appears verbatim in the tester stage section (this run: the full prompt-lint output appears twice in the reviewer's preamble).
Effect: ~10KB less reviewer preamble per review; reviewer reads each check result once instead of twice — cheaper reviews and less truncation pressure on the diff sections the reviewer actually needs.

**4. Stop the evidence capture from losing seed-work diffs — new item needed.**
Justification: no open item covers the loop lane's byte-ceiling truncation (fabro-31f7 fixed the *line* cap, now closed; fabro-3c9d was develop-side, closed).
Change: in `.fabro/workflows/loop/workflow.fabro`, raise the reviewer's effective preamble byte budget (graph `x.preamble_budget_kb=48` / reviewer `x.preamble_inline_max_kb=16`) or reorder `evidence.nu` so complete seed-work diffs precede recorded-checks noise — and move the loop graph-contract smoke pin (contract 10) with it.
Effect: the reviewer judges from the capture instead of re-reading all 6 files (its 4 `read_file` calls this run were purely compensating for the truncated planner.md/project-facts/reviewer diffs it named in its journal).

**5. Add the 100-char limit to the loop planner's title instruction — new item needed** (fabro-b529 covers the same rejection class but only proposes an engine-side cap change for revisor select).
Justification: no open item covers the missing length guard in the loop lane's planner prompt.
Change: one line in `.fabro/workflows/loop/prompts/planner.md` RUN TITLE paragraph: "keep the title under 100 characters — the tool rejects longer."
Effect: removes the two rejected `fabro_run_set_title` round-trips observed in events (seq 233–242), saving 1–2 planner turns per run.

**6. Reword the stale planner-node graph comment — new item needed** (fabro-ae57 fixed a different stale comment and is closed).
Justification: no open item tracks this comment, flagged by this run's reviewer as hygiene to fix "when that unit lands."
Change: `.fabro/workflows/loop/workflow.fabro` lines ~71–74 ("the tracker claim is the node's only write (fs_write pins the tracker file)") now contradicts the just-landed `fabro_work_next` claim flow — reword when fabro-c03a lands, or now with a transition note.
Effect: the next loop implementer/reviewer doesn't waste cycles adjudicating a graph comment that describes the pre-ADR-0032 mechanism (this run's reviewer spent journal space on exactly that).

Not recommended as new filings: fabro-9337 (prompt-lint check-9 scope extension) was already filed by this run's closeout and is owned by fabro-c03a — it's covered.
