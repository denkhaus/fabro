# Revision — run 01M2MKQNYA9WC10FSHX7G945DA

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2MKQNYA9WC10FSHX7G945DA.md
- seeds filed: fabro-a701 — Log absent optional memory files at info, not ERROR, in agent session init
- basis: run 01M2MKQNYA9WC10FSHX7G945DA, workflow version 5952f964d1fb60bb331471141f93441139394d5cb5926a93c600e54425d90f82, commit 2ae90727b845bd6ae5bd4c8e49ad5457708e3e96
- revised_at_commit: 2ae90727b845bd6ae5bd4c8e49ad5457708e3e96 (ADR-0015: engine drift signal for later judgement)

## Findings

### Stop logging absent optional memory files as ERROR in agent session init
- filed: fabro-a701
- Change: in the `coding_session_initialize` file-read path, log an absent optional memory file at info/debug, not ERROR. Evidence: this run's worker log shows 6 ERROR lines (2 per session for planner/implementer/reviewer) for `File "/workspace/fabro/.codex/instructions.md" was not found` despite a green gate. Expected effect: 6 fewer ERROR lines per green run; ERROR reserved for real failures. fabro-8275 covers only the preamble allow-key warn — different site.

### Close duplicate planner-batching seed fabro-2be2 (covered by fabro-55a7)
- filed: none — tracker hygiene; duplicate_of: fabro-55a7
- Change: fabro-2be2 and fabro-55a7 both open, both cover batching planner reconnaissance into one shell call. Closed fabro-2be2 as superseded by fabro-55a7 (same target `planner.md`, fabro-55a7 is the broader earlier seed), with the closure reason appended to fabro-2be2's description first (fabro-02c4). Expected effect: no double-implementation of the batching change.
