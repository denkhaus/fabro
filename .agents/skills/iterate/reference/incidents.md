# Incident index

The evidence layer: every incident, directive, and story behind a rule
in this skill, one line each, mapped to where the rule lives. Load on
demand — when a rule's WHY matters, or when auditing the skill against
drift. Incident ids are tracker legacy ids (fabro-xxxx) unless named
otherwise.

## Orientation / world (SKILL.md Phase 0, reference/orient.md)

- 2026-10-04 roster: a second /iterate alive in the same checkout found
  late -> sibling protocol runs FIRST. (orient.md)
- 2026-10-17 stash: a `-u` stash in the shared checkout grabbed a
  sibling session's WIP; pop restored someone else's dirty-set -> NEVER
  git stash. (orient.md)
- 2026-10-17 gh default: `gh repo view` resolves to fabro-sh/fabro in
  this checkout; a bare `gh pr merge 427` hit an unrelated 2026-05
  merge -> every gh call carries --repo denkhaus/fabro. (orient.md)
- PR #359 (2026-10-02): a red dogfood-gate run PR faked in-flight state
  and wedged the push gate -> PR sweep before selection. (orient.md)
- 2026-10-09 GitButler end: ~14-19h friction over 6 days, zero
  multi-agent use, permanent sha tax -> plain git; teardown v1000 same
  evening, machine 1 on 2026-10-16; the stale
  `origin/gitbutler/workspace` shadow branch verified gone 2026-10-16.
  (SKILL.md standing rules)
- fabro-cadd (2026-10-02): session bookkeeping proposed into line
  assets; PRs #359/#360 closed not-planned -> session/line boundary.
  (SKILL.md standing rules)
- 2026-10-17 rotation retro (brightctl port): 90 notes cost every
  session start the 50 KB read limit (134 KB file) -> rotate at >10
  sprints. (orient.md)
- 2026-10-02/10-26/10-29: the ledger shape, notes, and gate-marker
  directives. (orient.md)

## Selection (SKILL.md Phase 1, reference/select.md)

- 2026-10-12 Grundsatz-review + 2026-10-18 procedure: every decision
  point grilled via the ACTUAL grill-with-docs skill. (select.md)
- 2026-10-26: structured question-tool rounds replace free-text lists;
  options carry the Jev pre-screen distribution. (select.md)
- 2026-10-29: sprint batching (2-3 items, one review) and lane tranches
  (max five, hand-picked). (select.md)
- 2026-10-02 evening: local-first replaces the conductor-role split of
  2026-09-06. (select.md)
- 2026-10-27 sprint 58: no lane moves or line assignments even as
  recommendations until the re-armed line is proven reliable. (select.md)
- 2026-10-27 evening (#990 laps): vet-before-working; migration-era
  pool presumed stale — the laps caught a project-facts lane bug and a
  zombie doc item live. (select.md)
- claim/claim-check mismatch class: an unassigned item in a run goal
  makes the planner substitute the next claimable item while the
  implementer follows the goal text, closing the WRONG item. (select.md)

## Build (SKILL.md Phase 2, reference/build.md)

- fabro-8615 (2026-10-03): pins must be mutation-proven RED (restore
  the PRE-FIX, not a nearby mutation); also the `commit -m "$MSG"` empty
  message. (build.md, integrate.md)
- sprint-54: reverse-sed restore hit a pattern present in THREE sibling
  commands -> uniqueness check before sed-both-ways. (build.md)
- sprint-60 run 01M60NX9W7N6: a bare nu block is a closure value, never
  executed; a lenient shim is position-blind — drive the real script
  with the real binary against a dead server. (build.md)
- 2026-10-26 #995: clap derives --item-type from the bare field;
  failure-shaped snapshots must be rewritten, never accepted. (build.md)
- fabro-e901: slice arithmetic — an index computed against the full
  text passed into a suffix walk hung every envelope test. (build.md)
- 3fce (2026-09-30): fork-dep fixes need every consuming lock pinned;
  patch BOTH crates of a multi-crate fork. (build.md)
- fabro-e71b (2026-10-07): machine 2's uncommitted petri changes
  stranded machine 1 -> deps finished only when committed and pushed;
  since ADR-0034 (2026-10-24) the forks are vendored in-tree. (SKILL.md
  standing rules, build.md)
- fabro-1b2a: three stacked gate-repair layers invisible through 16-min
  CI rounds -> full workspace program before a gate-repair push;
  fabro-9707 tracks the CI-side fix. (build.md)
- 2026-10-03 PR #378: a sibling's uncommitted red crate masks your
  clippy locally -> CI arbiter, say so in the commit. (build.md)
- fabro-091b (sprint 28): an edit cell anchored on a `fn` line landed
  between doc and fn, hijacking the doc comment. (build.md)
- f93b: a single `\` before a newline in a Python-written Rust literal
  glued the next line's indentation into the string — twice. (build.md)
- 2026-10-28: the three bench directives (bench-testable, single-aspect
  probes, log cross-check). (build.md)
- 2026-10-29: re-arm is a reliability chain, never a default. (build.md)
- fabro-2919 (2026-10-12 grill): a gate fix needing a deploy to verify
  is a 40-minute turnaround -> `just smoke-window-gate` answers in
  seconds locally. (integrate.md)

## Review (SKILL.md Phase 3, reference/review.md)

- 2026-10-08 (sprint 23): a fork feature shipped with its pin file but
  no touchpoints registry row; nothing noticed for days until the
  parity battery -> battery sweep before declaring a gate healthy.
  (review.md)
- 2026-10-23: an external disk cleanup wiped sandbox plugins and
  corrupted a dep cache — 160 "failed" petri tests were environmental;
  one `cargo clean -p syn` + one `plugins:build` made 1776/1776 green;
  same day a dep bump's 21 fabro-petri reds were proven environmental
  by the baseline-run isolation (filed fabro-4618). (review.md)
- 2026-10-21: a fabro-server unit interruption left a missing FakeTracker
  impl invisible until the explicit IT target ran -> --features
  test-support and count the tests. (review.md)
- 2026-09-30 salvage directive; PR #337/#339 set the dump-apply-land
  pattern. (review.md)
- 2026-10-08: shell-written tracker text executed backticked phrases as
  command substitution and created three empty files at the repo root.
  (integrate.md)
- The old `-- fork_seam` filter matches NOTHING since the suite split;
  a 0-test filter is a silent skip (nextest exits 4). (review.md)

## Architecture gate (SKILL.md Phase 4, reference/review.md)

- 2026-10-02: the architect WORKFLOW stays deactivated; the gate fires
  the LOCAL agent; its recommendations become work items worked
  locally. (review.md)

## Integration (SKILL.md Phase 5, reference/integrate.md)

- 2026-10-03 fabro-8615: `bash()` does not see the REPL's names —
  `commit -m "$MSG"` committed an empty message. (integrate.md)
- sprint 71 (2026-10-31): a stale leftover /tmp message file was
  consumed by `commit -F` — the commit carried the wrong message (amend
  caught it pre-push); same session a `git add -A` against the rule —
  write+verify+commit+rm as one ceremony; derive staging sets from the
  change. (integrate.md)
- 2026-10-06 (GB era): a hand-edited invalid `vcs_manager` value made
  sync wipe the config file to empty. (integrate.md)
- 2026-10-26 (three races): a cp backup raced its edit; the #1003 close
  cat'd a file landing a second later and closed the item with an empty
  reason (repaired via issuecomment-6424201611); an amend hit an
  already-pushed sha; two pushes left sed riders behind. (integrate.md)
- 2026-10-02 23:00: `gate | tail -1 && git push` pushed through a
  REFUSED verdict; 2026-10-03: `echo RC=$?` read ECHO's status —
  capture rc=$? on the direct next line. (integrate.md)
- 2026-10-05 (sprint 19): piping a create's JSON into a crashing
  consumer still landed the record — duplicate creates (d7e7+3523);
  write CLI json to a file, sweep the tracker diff. (integrate.md)
- 2026-10-04 fork.5: a repin riding its own push bypassed the gate —
  parity refusal + stale release clone -> repin ordering rule.
  (integrate.md)
- 2026-10-15: a `timeout`-wrapped gopass probe kills the unlock prompt
  mid-entry -> BLOCKING probe without timeout, warm the store EARLY.
  (integrate.md)
- 2026-10-12 fork.14: the 17:30 Berlin tick ran 13 min BEFORE the 17:43
  apply and faked a 'fix does not work' -> post-deploy proof ceremony +
  fast recipe. (integrate.md)
- 2026-09-30: a night deploy applied via `-var` override; the
  uncommitted variables.tf default bump was lost -> commit the pin in
  the same session. (integrate.md)
- 2026-10-24 fabro-6041 grill (user GO): the pre-deploy bench gate.

## Standing rules (SKILL.md)

- ADR-0028 (2026-10-12 Grundsatz-review): the complexity brake — a
  second patch on a mechanism within days triggers simplify-or-revert;
  a patch contradicting a standing decision (even a 2026-09-14
  code-comment one) stops for supersession.
- 2026-10-03 no-language-sprawl; fabro-c16a (2026-10-08): the 16-file
  python review engine RETIRED as never used, not ported.
- fabro-6538 + 591a (2026-10-03): the rust-style-guide canonical/mirror
  pair; loop runs never write `.agents/**`.
- ADR-0022/d4c6 (2026-10-16): judgment pre-screens everywhere or never
  get evaluation data; fabro-c14e's PR-triage application stays parked
  in its demand.
- fabro-c4be (2026-09-30, two envelope kills): loop-asset fix surfaces
  are unlandable by the develop line.
- 2026-10-01 e62d: direct-fix proposal duty ("sonst verlieren wir
  Zeit").
- ADR-0027 (2026-10-09): three read-only inventories — denkhaus-0 had
  all ten capability axes, ~74% of Petri-era closures were
  REBUILD/FIX of pre-existing capability, the host-hook gap exists in
  BOTH worlds; a return would have lost the record/projection run stack
  and the ~24% genuinely new machinery -> deps frozen, re-open
  condition recorded. fabro-d485: general, non-strategic fixes still
  offered upstream selectively.
