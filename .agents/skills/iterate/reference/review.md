# Review and the architecture gate (Phases 3–4)

Load at Phase 3. Review execution, the language gate, presence suites,
prompt fact-checks, the battery sweep, salvage, environment-repair
classes, and the architecture gate.

## Two-axis review

- ONE review per sprint covers the batch's whole accumulated diff since
  the previous sprint's tip — not each item separately; per-item discipline (language gate, batteries,
  verification commands) still runs at every close.
- Freeze the diff to a patch file FIRST; both axes review the frozen
  snapshot. Standards axis = rust-style-guide + the strategy docs; spec
  axis = the tracker's design bodies. Fixpoint derivation: the tip at
  the end of the sprint named in `last_code_review_at_sprint`, derived
  with `git log`, never HEAD~N.
- Proof of work: the cycle report NAMES the guideline pages loaded for
  the diff and the verification commands actually run.
- Judgment pre-screen (ADR-0022, application point b): cycle review
  fires a verdict pre-screen per evaluated run and per-file residue
  adjudication, compared against the reviewer's own verdict — mismatches
  are findings.

## Language gate (ADR-0033)

- Every reviewed artifact is English — docs, comments, workflow
  prompts, work-item notes, commit messages, PR bodies, review notes. A
  German paragraph or a mixed-language file in the diff is a FINDING,
  not a nit: fix it in the same cycle, translating the whole file at
  once. Only test data that deliberately probes non-English model
  output stays non-English and is named as such. The mechanical net
  lives in `reference/integrate.md` (the staged-diff regex).

## Fork-feature regression check

- Run the fork-only presence suites of every crate you touched: for
  fabro-workflow the current files are `--test fork_ask_tool_registry
  --test fork_publish_gate --test fork_run_tool_pins` (the old
  `-- fork_seam` filter matches NOTHING since the suite split; a filter
  that runs 0 tests is a silent skip and nextest exits 4), plus every
  newer `fork_*` test file of the crates you touched.
- Every NEW fork feature carries a presence pin (fork-only test file +
  TOUCHPOINTS row); a pin-less fork feature ships only with a filed
  follow-up item. A RED fork-only test = a landed fork feature
  regressed: fix or revert, never relax the test.

## Prompt fact-check and hygiene

- Landed prompt diffs get a FACT-CHECK against repo reality: claimed
  branches, paths, command behavior.
- Prompt hygiene: workflow prompts land WITHOUT seed-id literals, run
  ids, PR numbers, commit shas, dated cost narratives, machine-specific
  paths, or decision-record references (ADR numbers, flip dates,
  attribute names) — prompts are runtime agent instructions; provenance
  lives in the graph comments (the maintainer layer) and the tracker,
  never mixed into the prompt layer (`.fabro/workflows/**`);
  branch/merge facts belong in PROJECT_FACTS. The mechanical net is the
  prompt-lint evidence ban.

## Loop-asset battery sweep

- BEFORE declaring a gate healthy, run EVERY battery named in
  `scripts/qualitygate.nu` (grep its `smokes` + `batteries` lists, run
  each with `nu`, read each exit code). Cargo rounds and the workflow
  smokes can be fully green while a registered parity battery stays
  red: one fork feature shipped with its pin file but no touchpoints
  registry row, and nothing noticed for days (found by
  `touchpoints-parity-fixtures.nu`). Seconds of work; the only net for
  the missing-row / missing-registration class.

## Environment artifact vs regression

- Local full-crate verification runs with `--profile ci` timeouts: a
  default-profile timeout on a Docker/sandbox test under dev-host
  conditions is an environment artifact (known class), not a regression
  — rerun isolated with the ci profile before treating it red.
- After an external interruption (server restart mid-build, user disk
  cleanup): a corrupt target cache shows as compile errors on untouched
  deps (`could not compile syn ... No such file or directory` — fix
  with `cargo clean -p <dep>`); deleted sandbox plugins show as instant
  expect-panics across whole blackbox suites (fix with petri's
  `mise run plugins:build`; fabro tests resolve the plugin from PATH).
  Repair the environment BEFORE reading any of it as regression
  (160 "failed" petri tests were once the wiped plugins; one
  clean -p + one plugins:build made 1776/1776 green).
- A dep-rev bump under suspicion gets a BASELINE RUN, not a revert:
  restore Cargo.lock to HEAD, move a new pin file aside when it cannot
  compile against the old revs, run the same suite — an identical
  failure SET means pre-existing (file it with the evidence, keep the
  bump); a GROWN set means the bump owns it (a 21 fabro-petri
  reds proven environmental by exactly this isolation).

## Suite-specific traps

- fabro-server's IT suite (`tests/it`, incl. the OpenAPI conformance
  test) requires `--features test-support` and is NOT built by a plain
  `cargo nextest run -p fabro-server` — the run stays green while the
  it target silently sits out. Verify with
  `cargo nextest run -p fabro-server --test it --features test-support
  --profile ci`, and count the tests against the handoff's expected
  number.
- Direct pushes to the line branch skip dogfood-gate (it runs on PRs
  only) and reds accumulate invisibly until the next PR pays for them:
  run the touched crates' tests before a direct denkhaus push, or use a
  PR vehicle for test-affecting work. Never switch branches while a
  background cargo runs.
- After any non-PR integration (emergency squash, manual land): sweep
  for the platform-created run PR and the run branch — a leftover open
  PR blocks the push gate and fakes in-flight state. Close the PR as
  already-integrated citing the landed sha; delete the branch.

## Salvage

- When an evaluated run FAILED or is a green-lie `succeeded` and its
  implementer/tester stages produced noteworthy work (non-trivial
  diff), SALVAGE it: `fabro dump --output
  <dir> <run>` (run state is durable on mirtuell), apply the stage
  diff, verify, land via PR citing the run, or file a salvage-pointer
  item citing run id + dump command + stranded checkpoints when direct
  landing does not fit. Never let stranded work vanish silently; never
  let a failed run close or lose an item (closure follows the publish
  path — failed runs publish nothing).

## Truth-checking

- Verify a reviewer's factual premise in code before fixing; the same
  for auto-filed item citations (check the cited item's premise AND
  implementation status). A closed item whose demand is invisible in
  any diff: grep `.fabro/journal` + `.fabro/revisions` for the id
  before reporting it lost; an undocumented closure is itself a
  finding.
- Tool-call misuse by agents is BOUNDARY evidence, not prompt material:
  fix the boundary (validation naming property paths, contract
  ergonomics), file the boundary item in the same session.
- Revise the autonomous workflows after every delegated cycle: inspect
  the runs' journals and stage outcomes; every painpoint lands as a work
  item, a skill edit, or an explicit no-action note in the cycle report.
  `fabro ask <run-id>` improve-review occasionally (~every 5th cycle).
- Verification discipline (unpiped commands, full reruns, sequential
  load, failure-set diffs, clean-tree controls, simulation safety,
  e2e-profile traps): `mulch prime testing`.

## Phase 4 — the architecture gate

- Deepen when review surfaced structural smells OR the Phase 0 verdict
  is `architecture-due`; `grind` deepens only on smells; `normal` never
  deepens on score alone. Judge proposals against the guide and
  codebase-design vocabulary.
- Standing path: the architect
  WORKFLOW is DEACTIVATED on the line — the gate fires the LOCAL
  improve-codebase-architecture agent; ONLY its STRONG recommendations
  become work items, worked LOCALLY. The
  autonomous architect workflow returns only on an explicit future
  user order (then: self-gated by friction, 48h cooldown, files its
  own items).
- Procedure: delegate the scan to a sub-agent with an output contract
  (candidates to a file: files, scope, problem, solution, benefits,
  deletion-test verdict, strength), curate with the fork-scope filter,
  then file every non-Speculative STRONG candidate IMMEDIATELY with
  owner-based assignment — nothing survives only in the report. Medium
  and Weak candidates are NOT filed: they
  land in the sprint note's gate paragraph, one line each — the
  tracker stays signal, the gate note stays the archive. Render
  the interactive skill's HTML report on LOCALHOST only — anything
  beyond this machine is an exposure decision that belongs to the user.
  Mark the ledger (`last_arch_review_at_sprint`) and offer the grilling
  loop for the picked candidate.
- Architect scope is binding: findings restructure ONLY the fork's own
  surface (`.fabro/workflows/**`, `.fabro/scripts/**`, fork-only files,
  our tooling). Upstream-owned `lib/**`/`apps/**` is observable, never
  restructurable. Apply the same filter when reviewing architect
  findings.
