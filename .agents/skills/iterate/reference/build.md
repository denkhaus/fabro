# Build discipline (Phase 2)

Load before the first edit cell of any implementation work. Mechanical
gates, pin-power proofs, edit traps, Rust rules, vendoring, bench
directives, and verification recipes. Edit mechanics also live in
`mulch prime rust`.

## Design before code

- File the item's design (the grill outcome) into the tracker BEFORE
  any code exists — design decisions live in the tracker, never only in
  chat or the diff.

## Local build discipline

- Claim (`work update --status in_progress`), implement under the
  mechanical gates of the touched surface (Rust: rust-style-guide +
  the guideline pages covering the diff, loaded and NAMED before the
  first edit cell), verify with real commands, close with evidence.
- Run the touched tests IMMEDIATELY after each refactor edit, not at
  the end of the batch: three self-inflicted scanner bugs (two byte
  offsets, chain legs) each cost a hung 120s/20s test round; the unit
  test catches them in 0.02s.
- Slice arithmetic is the recurring trap: never pass an index computed
  against the FULL text into a walk that slices a SUFFIX — carry the
  absolute cursor beside the relative offset.

## Pin power is PROVEN, not assumed — RED FIRST

- RED FIRST is the standard loop (user directive 2026-11-01; the @tdd
  skill is binding for the loop's order and shape): write the
  test/pin BEFORE the behavior exists and run it RED against the
  genuine absent-or-broken state — for a fix, the pre-fix tree IS the
  red state; for new functionality, stub only what must compile
  (types, enum fields, initializers) and leave the BEHAVIOR out so the
  test fails for the missing behavior, not for a compile error. Then
  implement minimally to green. The observed red run IS the pin's
  proof — no mutation needed.
- The mutation/restore proof (restore the PRE-FIX implementation, or
  mutate the new behavior, confirm RED, restore) is the FALLBACK for a
  test written AFTER its implementation already existed (inherited
  code, a late-added pin): it reconstructs the red the red-first loop
  would have shown. A "nearby" mutation of the new code can pass while
  the pin is blind — two variant mutations of one scanner survived;
  only the HEAD restore failed the run-level pin with the real
  symptom.
- When unsure about the loop's order or shape, LOAD the @tdd skill and
  follow it — do not improvise a variant (implement-then-test-then-
  mutate is the anti-pattern the 2026-11-01 directive named).
- A test added through an EXISTING harness must prove the input reaches
  the seam: one projection harness never wrote the run spec's
  `graph_source`, so the first engine-level pin was trivially green
  until the harness carried it.
- A sed-both-ways mutation restore must verify the pattern is UNIQUE in
  the file first (grep -c == 1, or anchor on surrounding context): one
  reverse sed rewrote a flag line in THREE sibling commands while
  restoring one and reddened unrelated ITs — caught only because the
  test filter matched them.
- Two sprint-60 pin traps: (a) a bare nu block `{ ... }` as a statement
  is a CLOSURE VALUE, never executed — a pin arm written that way reads
  as green while it never ran; executable arms need `do { ... }`.
  (b) A lenient shim (nu parses declared flags wherever they appear) is
  POSITION-BLIND and can never pin an argv grammar — pin CLI invocation
  shapes by driving the real script END TO END with the REAL binary
  against a dead server and asserting the failure is transport, not a
  clap parse error.

## Known failure-shape classes

- A NEW-flag IT failing with clap's "unexpected argument" is a FLAG-NAME
  mismatch until proven otherwise: clap derives `--<snake_case_field>`
  when no explicit `long =` is set, so a field `item_type` ships
  `--item-type` while tests, docs, and the agreed surface say `--type`.
  Compare the bare field name against the agreed flag BEFORE suspecting
  the harness or a stale binary.
- A pending snapshot for a NEW feature whose captured output is the
  FAILURE itself (error text, nonzero exit) means `cargo insta accept`
  baked the bug in: rewrite the snapshot to the intended behavior,
  never accept a failure-shaped snapshot of a new feature.

## Direct-fix verification recipe (before any push)

- Lint first (`just lint-nu` for nu), then DRY-RUNS — positive AND
  negative — each as its own shell call, exit codes read from the
  process (never behind a pipe: `$?` reads the last pipeline member,
  not the script). Nu interpolated strings treat `word:` before a `(`
  as a command call — reword such phrases; parse-clean does not mean
  run-clean.

## Fork and dependency wiring

- Delegated builds (the explicit-choice case): feed implementation items
  with pointers — files, trait seams, guideline pages.
- A fix on a forked dep repo is NOT shipped until every consuming
  workspace's Cargo.lock pins the new rev — `branch =` patch pins move
  only on `cargo update`; bump the lock in the SAME change. Patch BOTH
  crates of a multi-crate fork repo (a single entry splits shared
  traits into two copies and fails unification).
- SINCE ADR-0034 (2026-10-24) petri and pebble are VENDORED in-tree:
  the fix lives in `vendor/<name>`, the `[patch]` block resolves it by
  path, and the vendor tree IS the pin (`vendor/PINS.toml`,
  `just vendor-pins`) — no lock rev to bump. The tree is exported to
  the fork branch in the same change. The branch-pin/lock-bump half
  still governs the remaining git deps (lithos-llm, sandbox-driver,
  twins).
- DEPS ARE FINISHED ONLY WHEN COMMITTED AND PUSHED: fork-dep work left
  uncommitted or unpushed at session end strands every other machine
  that needs it (machine 1 had to park a takeover because machine 2's
  petri changes never left the local tree). See the dependency-finish
  standing rule and `reference/integrate.md`.

## Rust gates

- Rust work: read rust-style-guide's SKILL.md AND the guideline pages
  covering the diff in the SAME turn, before the first Rust edit cell;
  name the pages in the cycle report.
- Gate-repair pushes (fixing a RED dogfood-gate): verify with the FULL
  workspace program locally BEFORE the push — pinned-toolchain
  `cargo clippy --locked --workspace --all-targets -- -D warnings` plus
  the touched crates' `cargo nextest run --profile ci --no-fail-fast`.
  The gate aborts at the first failing crate and nextest at the first
  failing test, so later-crate breakage and further snapshot drift stay
  invisible through any number of 16-min CI rounds.
- Multi-agent limit: `cargo clippy` lints EVERY workspace member, so a
  sibling agent's uncommitted red crate blocks or masks your OWN lint
  verification locally. Then: message the sibling the exact lint + fix,
  fix yours by reading, and accept CI as the arbiter for the round —
  say so in the commit message.
- Edit-cell mechanics: Rust edit cells that INSERT an item before a
  documented function anchor on the item ABOVE (or the previous item's
  closing brace), never on the target's `#[must_use]`/`fn` line — the
  doc comment sits above that line and the insert lands BETWEEN doc and
  fn, hijacking the doc. Rust line-continuations written THROUGH
  Python MUST use raw strings or doubled backslashes: a single `\`
  before the newline glues the next line's indentation INTO the literal
  as mid-sentence spaces. Pin full user-facing refusal strings in tests,
  not just their prefixes.

## Loop assets

- Deterministic-script-first: a prompt clause requiring judgment over
  mechanical data becomes a script that prints a verdict
  (`.fabro/scripts/`, `just lint-nu` for new nu scripts; prompts keep
  only the call + verdict routing). Never write and execute an edited
  script in one shell call.
- Every `.fabro/workflows/**` change lands with a bench surface in the
  fabro-test repo that exercises it (user directive 2026-10-28) — a
  dedicated probe/scenario, or an existing row that provably drives the
  changed asset. The cycle report names the surface; a change without
  one files the fabro-test follow-up in the same session.
- BENCH PHILOSOPHY (user directive 2026-10-28): targeted single-aspect
  probes that compose — the real lane graphs are deliberately NOT bench
  material (they need items + LLM stages). Compose aspects rather than
  replaying whole lanes: a graph shape against its fold, an environment
  property against the arms that depend on it, an invocation grammar
  against the runtime CLI. NEVER rerun the full bench for one facet's
  red: the single-probe mode is the default; a red facet gets its
  targeted probe plus, for integration legs, a direct manual drive of
  the leg's script.
- BENCH CROSS-CHECK VIA PLATFORM LOGS (user directive 2026-10-28):
  after every test round — bench, probe, or supervised lap — query the
  log platform for the window's WARN/ERROR and reconcile the patterns
  against the green table. Green tests are only one side of the coin;
  the log side names what the asserts never saw. Query mechanics live
  in mulch and the rootprint skill.

## Re-arm and laps

- RE-ARM IS A RELIABILITY CHAIN, NEVER A DEFAULT (user directive
  2026-10-29): a line re-arm (scheduler/automation re-enable) is the
  user's explicit call, following the same chain every time — the named
  blocker items are landed, ONE manual supervised lap per lane has run
  green (each log-cross-checked), and only then the user decides. The
  concrete blocker list of the moment lives in the tracker/handoff,
  never in this skill.
- Probe rigs: workspace-relative hook/stage scripts resolve against the
  RUN TARGET checkout — land the probe on the target branch first or
  create with `--target OWNER/REPO@<probe-branch>`. Nu verify scripts
  get positive AND negative dry-runs before any run.
- `just up` ships the working tree into the local image: run it from a
  landed/clean state. A handoff's stack claim is verified
  (`.../install/session`: 401 = unconfigured, 404 = configured).
