# Integration mechanics (Phase 5)

Load at Phase 5. Commit ceremony, tracker-write traps, push policy,
deploy windows, toolchain pins, fork releases, and bench landing.

## Commit ceremony

- Commit code (plain `git add <files>` + `git commit -F <msg-file>`)
  BEFORE tracker mutations land. The message travels through a FILE,
  never through a shell variable — and the file is written and read
  back THIS session (a stale leftover message file once committed under
  the wrong message; amend caught it pre-push). Write /tmp/msg.txt,
  commit with -F, read the message back (`git log -1 --format=%B`),
  then delete the file — before pushing.
- Stage NAMED files only (never `git add -A`) so a sibling session's
  uncommitted work cannot ride the commit — and after any bulk staging,
  verify the staged list against the expected path prefixes (the change
  itself, `git diff --name-only`, is the staging set for mechanical
  rewrites: two consecutive pushes once left riders behind because the
  sed batch touched six files and the hand-typed add list had four; an
  EMPTY `git status` is part of the push gate, not an afterthought).
  A `git rm` STAGES the deletion itself: leave deleted paths out of
  later `git add` lists — a staged deletion has no worktree file, and
  `git add` on it fails with a pathspec error.
- Language gate before commit (ADR-0033): re-read the staged diff —
  every written line is English. Cheap mechanical catch:
  `git diff --cached | rg 'ä|ö|ü|ß|Zweck|Regeln|Facette|Pflege|wird|nicht|muss'`.
  Hits are inspected, not silently kept; deliberately non-English test
  data is the named exemption. A sibling repo with its own landing
  dance (fabro-test: `just land <subject>`) needs the same look at its
  docs, not only at its scripts.

## Shared-checkout and tracker-write traps

- A SHARED checkout means shared FILES: whole-file commits can sweep a
  sibling's tracker lines into your commit. Check the per-id diff for
  foreign ids before committing a shared file, land your own lines
  promptly, and re-read `iterate-state.json` before writing — the
  sibling session counts its closures into the same ledger.
- CONFIG TRAP (2026-10-06, GB era): a hand-edited INVALID
  `vcs_manager` value made sync wipe a config file to EMPTY; the repo
  now carries NO `vcs_manager` key (plain default). Valid keys when one
  is ever needed: project/version/max_plan_depth/vcs_manager.
- Shell-written tracker text NEVER carries backticks or `$(` through a
  `bash()` command line (2026-10-08): bash executes them as command
  substitution — a close reason containing backticked phrases ran the
  command inside and created three empty files at the repo root. Write
  the payload to a file and pass `--reason "$(cat /tmp/reason.txt)"`;
  after any shell-based tracker write, sweep `git status` for stray
  untracked entries and delete them before committing.
- NEVER a file-WRITE and its file-CONSUME in one PARALLEL tool block
  (2026-10-26, three times: a cp backup raced its own edit and held the
  PRE-FIX; a close cat'd a file landing a second later and CLOSED THE
  ITEM WITH AN EMPTY reason — repaired by a comment, but the close
  comment is the evidence record). Sequence them: write, observe the
  result, then consume.
- NEVER amend a PUSHED commit: the branch refuses force-push and
  local/remote diverge. Check `git status` against the remote tip
  before any amend; recovery is `git reset --soft <pushed-sha>` + a
  fresh commit — correctable, but a wasted gate round.
- A create whose post-processing dies still created the record: capture
  exactly one id per intended create and `fabro rm --force` duplicates
  immediately — submitted ghosts count as active runs and wedge the
  push gate. The same class hits tracker creates: piping a create's
  JSON into a consumer that crashes still lands the record — write CLI
  json to a FILE, parse the file, and sweep the tracker diff for
  surprise new ids before committing.

## Push policy

- During the cycle: branch updates and read-only integration stay
  allowed; the PUSH direction is gated to one mechanical decision at
  the END: `nu .fabro/scripts/push-gate.nu` (exit 0 = open: no running
  pass AND no open run PR). Check and push share one cell; the gate
  runs UNPIPED; the push itself is `git push origin denkhaus`.
- The RC traps, all live incidents: `gate | tail -1 && git push` pushed
  straight through a REFUSED verdict while a loop pass ran (`&&` behind
  a pipe reads the pipe member's exit code); `cmd; echo RC=$?; if
  [ $? -eq 0 ]` reads ECHO's status, not cmd's — capture `rc=$?` on the
  line DIRECTLY after the gated command and branch on `$rc`. A repaired
  gate is validated against a known-active line state before its first
  OPEN verdict is trusted. Incident restore may push as soon as no pass
  runs — it does not wait for the cycle's end gate.

## Deploy windows

- Deploy only while no conductor pass runs.
- PRE-DEPLOY BENCH GATE (2026-10-24 grill, user GO): before EVERY
  deploy window, run the fabro-test workbench —
  `cd ~/dev/fabro-test && just up && just workbench` (refreshes the
  local image, then the facet table: probes + platform checks + mini,
  ~15 min). A RED table means NO window: fix or file first (the repo's
  own rule — new product bugs land as FACETS row + probe BEFORE any
  deploy). The bench refuses a non-local FABRO_SERVER; carry the local
  override explicitly.
- GATE-TOUCHING DEPLOYS: before the window, run
  `just smoke-window-gate` (the twin-scenario smoke: scripted 429 ->
  tick holds + manual fire refused; reopen -> next recheck tick fires)
  — a gate fix that needs a deploy to find out is a 40-minute
  turnaround; the smoke answers it in seconds, locally.
- Pause the line first: automation replace with FULL body + `If-Match`
  revision + explicit `on_overlap: skip`; re-GET and verify it
  survived. Deploy nonblocking; smoke on the live host; re-enable;
  monitor via heartbeat.
- POST-DEPLOY PROOF CEREMONY (2026-10-12, fork.14 lesson): a deploy is
  only 'done & working' after ONE live proof of the shipped behavior —
  for gate/automation changes fire the automation once (between ticks,
  on_overlap-safe) and observe the run starting; never report success
  from version strings alone (the 17:30 Berlin tick once ran 13 min
  BEFORE the 17:43 apply and faked a 'fix does not work').
- FAST DEPLOY RECIPE (same lesson): warm gopass BEFORE the build, edit
  the tofu pin DURING the build, skip CLI install + prune when
  unchanged — mechanical target <=30 min from commit to live proof.

## Toolchain and releases

- Tooling-repin rule (user directive 2026-10-04): a fork release repin
  is COMMITTED locally and NEVER pushed on its own — the session owns
  every line push (behind the gate); repins riding their own push
  bypass the gate and move the tip past the deployed sha (the fork.5
  incident: parity refusal + stale release clone). Deploy-window ORDER:
  repin (if pending) -> version bump -> push -> clone-build -> tofu
  apply -> pin-toolchain — one window restores
  `line tip == deployed sha == pin target`.
- Fork releases follow ADR-0025 naming `<frozen-base>-fork.N` — the
  upstream `cargo dev release` path (origin main push) is WRONG for the
  line branch. Manual bump = workspace `Cargo.toml` version +
  `cargo update --workspace` + `just image-release` (tag
  `<version>-<sha>`); N increments once per built release; the base
  moves only with a deliberate intake decision.
- Toolchain pairing: `fabro env pin-toolchain --from-run-images` (from
  the repo at the deployed tip) — the parity gate refuses a tag whose
  sha is not the deployed server's (working as designed; the wrapper
  `.fabro/scripts/pin-toolchain.nu` checks the registry manifest
  first). Run it FROM THE REPO AT THAT TIP.
- Recompute the sandbox-plugin sha per deploy (same pin rev usually
  means the same sha — verify, never assume):
  `docker run --rm --entrypoint sh <image> -c 'sha256sum
  /usr/local/bin/sandbox-driver-docker'`.

## Secrets

- Secret-gated steps (tofu/gopass) are probed with a BLOCKING
  `gopass show` WITHOUT a timeout wrapper BEFORE the step that needs
  them (user directive 2026-10-15: a `timeout`-wrapped probe KILLS the
  user's unlock prompt mid-entry — the mask disappears as soon as they
  start typing; the probe must wait for the unlock, not race it); the
  store relocks on its own TTL — ask the user to warm it EARLY, not at
  deploy time. gopass cold cache hangs non-interactively.

## Production deploy

- After substantial engine changes (binary-need check: any `lib/` path
  in the merged work): `just image-release` + fabro-tofu apply —
  NON-INTERACTIVE (the justfile profile picker is fzf-only by design;
  name the profile explicitly):
  `cd ~/dev/fabro-tofu && export TF_VAR_state_passphrase="$(mise exec --
  gopass show -o fabro-tofu/state-encryption)" && TF_DATA_DIR=
  .terraform-prod mise exec -- tofu plan -var-file=envs/prod.tfvars`
  — apply identically with `apply -auto-approve` after reviewing the
  plan's add/change/destroy shape.
- After EVERY tofu deploy: commit the fabro-tofu `variables.tf` image
  pin in the same session (2026-09-30 near-miss: a night deploy applied
  via `-var` override, the uncommitted default bump was lost, and the
  next plain `tofu apply` would have rolled production back). The
  committed default must name the LIVE image before the session ends.

## Bench tooling (fabro-test)

- Script changes land through the one-command PR dance
  (`just land <subject>`), NEVER a manual reset dance; scripts with a
  deterministic-reset step refuse dirty trees.
- The bench (`nu scripts/workbench.nu`, local stack default) is the
  engine gate — a full table with only tracked known-reds (YELLOW,
  exit 0) is the pass state; every fresh red is a finding. Nu scripting
  pitfalls: `mulch prime nu` and `mulch prime tooling`.
