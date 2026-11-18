# Glossary

The working vocabulary of the work-item tracker (ADR-0032). One term, one
meaning; every entry names where the term lives in code and what it is
confused with.

## Work item

One unit of work a human files and a run eventually closes: a title, a body,
labels, a priority and a state (`open` / `closed`). Content is owned by a
tracker provider (GitHub today), never by the server.

- Key: `item_key`, `<trackerRepo>#<number>` for GitHub — e.g.
  `denkhaus/work#12`. This is what `fabro work show` takes and what the API
  returns as `itemKey`.
- Priority: derived from the `prio:pN` label, default `2`.
- Not a **claim**: an item says what should happen; a claim says who is doing
  it right now. An item outlives every claim on it.

## Claim

The server-owned assignment of one work item to one run, arbitrated in the
claim ledger (ADR-0032 decision 2). High-frequency and conflict-prone, which
is why it never travels through the tracker.

- Unique per `(project, item)` and per run while non-terminal.
- States: `granted` → `active` → `awaiting_merge` → `landed`, plus
  `released` when a run gives the item back.
- Carries the run id, the lane, and the PR url of the run that holds it.
- Not a **label on the item**: a claim is a ledger row. The tracker carries
  at most the visible echo of one (`claimLabelPrefix`, `claimCommentEnabled`).

## Work project

One registered tracker binding: a project id, a provider, a tracker
repository, the label that scopes items to it (`projectLabel`, e.g.
`project:fabro`), the lane→label map, and the per-lane backpressure cap.

- `fabro work` verbs always name it explicitly with `--project <id>`; there
  is no implicit default, so a script line always says which project it
  means.
- Not a **code repository**: `targetRepo` (where the runs work) is an
  optional second field. A project may track work without owning code.

## Lane

A named queue inside a work project (`loop`, `develop`, …). The lane name is
what callers pass (`--lane loop`); the project's `laneLabels` maps it to the
tracker label that marks an item as belonging to that lane
(`lane:loop`). Lane labels are how an item is queued; the lane is also the
unit of backpressure (`backpressureOpenPrCap`).

- Not a **label**: the label is the projection, the lane is the concept. An
  unknown lane name is a hard error, never a silent fallback to a raw label.

## Legacy id

A `.seeds` identifier (`fabro-d5f0`), optionally written `seeds:fabro-d5f0`.
It names an item that lived in `.seeds/issues.jsonl` before the cutover and
now lives on the tracker.

- Accepted by `fabro work show` / `close`, which pass it to the server
  unchanged; the server resolves it through the migration label seam
  (`seeds:<id>` on the tracker item, stage 1a) and returns the item.
- A **transition affordance with a removal deadline**, not a second key: new
  items never get one, and once the migration window closes the seam and the
  acceptance go together. The live proof of that resolution waits for the
  import, which is the step that writes the labels.

## Closure radius

The set of open work items linked to the OBSERVATIONS discovered in a
work item's runs (ADR-0038: the chain is work item ← observation ←
run, carried in fabro's database). A claimed item's closure radius is
empty when every observation-linked work item is closed; the `work
close` verb refuses while the radius is open (an explicit override
names them). Tranche selection prefers closing open radii before
opening new themes — a preference, never a gate.

## Observation (record)

A platform record of what a stage durably observed, persisted per
project/run/stage through the `fabro_observation_record` agent tool.
The genus is the observation (user decision 2026-10-30: an observation
can contain painpoints, never the reverse); its two categories are
`painpoint` (friction worth acting on) and `note` (context worth
keeping, including the explicit "nothing durable to record" entry).
Calling the tool at least once per agent stage under
`x.observations="required"` is a presence contract — a stage that
never called it is invalid (engine-validated beside the output
schema). Work items link to observation records; the bird's-eye
run-end hook files them, deduping at the link. [The filing sentence is
superseded by ADR-0039: the adjudicator workflow files and closes; the
run-end hook strips.]

## Triage state

The derived lifecycle of an observation record over its work-item
links and triage decisions: `untriaged` (no linked item, no decision),
`filed` (at least one linked item open, none with a live claim),
`in_progress` (a linked item open with a live claim), `closed` (EVERY
linked item closed), `reviewed` (a no-filing decision exists and no
link — the triager judged no item needed; a later filing overrides).
Never stored — every reader re-derives it from the links, the
decisions, and the linked items' live states.

- Not a **decision recorded on the observation**: the states project
  the linked items' tracker states and the decision row; `closed`
  means the chain ran to its end (a wontfix close closes the painpoint
  too), never that the friction was verified resolved.
- Not the **followup marker** (`unfinished | exempt | minor`): the
  marker is a data field on the record naming what survives the run;
  the triage state says what the platform did about it.

## Orchestration fields

The two coupling fields on the run record (ADR-0039), one writer
each: `merge: pending|parked|enabled|refused` (the adjudicator
writes) and `closable: pending|parked|enabled|refused` (the merger
writes). Dataset fields are the ONLY coupling between the scheduler
gears — no messaging conventions. `parked` is an operator escalation.

- Not a **status enum on the item**: the verdict belongs to ONE run;
  an item can carry several runs.
- Not a **bool**: `false` cannot say "refused" apart from "not yet
  decided".

## Automation

The platform record that binds a workflow to a cron schedule with an
overlap policy (`on_overlap: skip`): what `POST /api/v1/automations`
creates and the `fabro automations` facade manages (list, pause,
unpause, breaker, fire, status). An automation says WHEN a workflow
runs — it is not the workflow and not the firing component. The
line's automations are the gears; any scheduler-driven workflow can
be one.

## Scheduler

The server component that evaluates every automation's cron schedule
and fires the runs (the automation scheduler). It can wedge while the
HTTP API keeps serving — the gap the `fabro automations status`
fire-drift view and the watchdog exist to cover.

## Adjudicator

The scheduler-driven serial workflow (ADR-0039) that reviews the
observations of all runs one after another, watches the closure
radius, asks the operator for policy decisions (park + resume), marks
`merge` on the run record, and — in its second pass, once the merger
marked `closable` — closes the work item. The ONLY closer; a mechanical
fast path handles the trivial case without an LLM call. Painpoint
filings are NOT its work (ADR-0044): the triager owns every painpoint's
disposition globally; the adjudicator reads painpoints as VERDICT
input (the lane-drained tranche-demand filing is the one routed
exception).

- Not the **revisor** (retired, ADR-0038): the revisor filed from
  journal prose; the adjudicator reads observation records and
  orchestration fields.
- Not the **reviewer** (a stage inside the develop run judging the
  diff): the adjudicator judges the RUN and its aftermath.
- Not the **triager** (ADR-0044): the triager judges RECORDS (every
  painpoint's disposition, any workflow); the adjudicator judges RUNS
  (the merge verdict, the close).

## Triager

The scheduler-driven workflow (ADR-0044) that owns GLOBAL painpoint
triage: its sweep reads the observation feed's `untriaged` painpoints
(feed-driven, workflow-agnostic — no verdict semantics needed), and its
agent pass disposes each one — files a linked work item (filing IS
linking), links an already-filed item (dedupe at the link), or records
a `no_filing` decision (the record then reads `reviewed`). Never
verdicts, never merges, never closes: triage is its whole jurisdiction.
The manage door's allowlist keys on its workflow slug.

- Not the **adjudicator**: the triager judges records; the adjudicator
  judges runs (and closes).
- Not the **revisor** (retired, ADR-0038): the revisor read journal
  prose; the triager reads observation records and writes keyed
  decisions through the manage door.

## Merger

The scheduler-driven workflow (ADR-0039) that integrates every
merge-enabled run of a repo/project — serializes, merges, resolves
conflicts — and marks `closable` on the run record. Owns technical
integration, never the content verdict; parks (operator escalation)
on unresolvable conflicts.

## Reporter

The develop run's facts-only final stage (ADR-0039): reports patch
visibility and Dockerfile touches as observation records. Supersedes
the closeout — the name retires with the close authority, because the
run never closes its own item. Landed with the stage-G cutover
(2026-11-06): the closeout's park gate, marker sweeps, and claim-bound
close retired; the residuals, deferred actions, and EXEMPTION arms
ride painpoint observation records to the adjudicator's agent pass
(their filer), and reviewer residuals record as painpoints too.


## human_needed

A terminal run status (work#1072): the deadlock-exit family parks a
run as a DECISION REQUEST — work is preserved, a human decides. The
run boards in the decision column (ahead of the outcome columns),
never in the failure column; the claimed work item's claim holds
until the adjudicator's verdict. Not a **failure**: a human ask is not
a fault (the breaker never counts it; the watchdog owns stall
visibility). Not **blocked**: a block waits for an answer mid-run;
human_needed is terminal.

## Watchdog

The scheduler-driven dead-man watch (ADR-0039 stage F): an LLM-free
workflow on its own hourly tick that reads every gear's automation
state and pass history through the read doors and ALARMS by failing
its own run — the Slack lifecycle notification carries the alarm. A
gear stalls when its automation is ENABLED and no completed pass sits
within 3x its tick interval. A disabled gear stays silent: the user
pause is authoritative. Nothing watches the watchdog — its automation
row and `fabro ps -a --workflow watchdog` carry the meta-stall
visibility.

## Gear

A scheduler-driven workflow (ADR-0039 decision 9) with its own cron
tick and `on_overlap: skip`, coupled to the other gears ONLY through
the orchestration fields — no messaging conventions. The three serial
line gears: develop (item-driven, direct fire), adjudicator
(run-driven), merger (merge-marker-driven). The watchdog watches the
gears but is not a gear itself. A gear IS an automation — the word
names the line's instance class, not a different mechanism; the
scheduler fires it like any automation.

## Wedge

Incident vocabulary (denkhaus/work#1143): a component stuck alive —
it stops doing its work while every health signal still reads ok, and
a restart heals it. Named after the scheduler wedge of the fork.30
window: scheduler ticks AND the log pipeline stopped together while
the HTTP API kept answering. Not a crash (no error path fires), not a
pause (nobody paused anything) — the discriminating signal is silence
against an ENABLED schedule.

## Workflow version

The immutable package of one workflow — the entrypoint graph, its
prompts, and every referenced file (imports, child workflows, the goal) —
registered through the workflow-versions API and stored content-addressed.

- Key: `workflow_version_id`; a run's spec names the exact version it
  admitted (`fabro inspect` shows it).
- Same content → same id. Any change to graph, prompts, or referenced
  files → a new version. A platform release does NOT move a version: the
  id addresses workflow content only.
- Not a **checkout**: the version is the workflow's own content; the
  checkout is the repository state the run works on.

## Environment image

The image an environment names for its sandboxes — either a finished
reference it pulls, or a Dockerfile it builds from at scope
acquisition (ADR-0042). One concept, two sources; the environment
declaration says which.

- Not a **runner image** (petri's term): the upstream base images the
  engine's own execution starts from, pinned in petri (`RUNNER_PIN`).
  An environment image is what a project's environment asks for.

## Toolchain image

The environment image a project declares for running its own
workflows: the tools and warm build layers its stages rely on, built
from the repository's own checkout at run initialization
(ADR-0042). For the fabro line: the repo-declared environment image
carrying the gate toolset, cooked dependency layers, and the fabro CLI
compiled from the checkout.

- Not **parity-pinned** (retired, ADR-0042): the old server-managed
  toolchain environment frozen to the deployed server's sha. The
  version now follows the checkout; the client-server seam is checked
  at the run channel instead.

## Run channel

The per-run capability path between a sandbox and the serving
platform: the platform serves its router on a socket that belongs to
one run, the sandbox reaches it through that run's mount, and every
request rides the run's principal through a per-spawn handshake token
(ADR-0035). The client-server compatibility seam lives here — checked
against a compile-time floor at connect, not frozen by version pins
(ADR-0042).

- Not the **work facade**: the work channel is how stage scripts CALL
  `fabro work` verbs; the run channel is the transport underneath.

## Supervisor

The full-access operating mode of the Ask Fabro agent (ADR-0046): global
context over runs, observations and work items, with the full run-tool
registry (including `run_mark`, `run_merge`, `run_create`, `work_close`)
available for countermeasures. Reached through the docked run chat, the
global chat page, or `fabro ask --capability full`.

- Not the **controlling session**: the session is the human-side agent
  driving the CLI; the supervisor is the productized in-product partner.
- Not the **run-local read-only chat**: that is Ask Fabro's default
  mode; the supervisor mode is an explicit per-session operator grant.

## Capability control

The per-session, server-enforced switch between Ask Fabro's read-only
default and supervisor (full access) mode (ADR-0046). Flipping requires
the authenticated principal, persists for the session, and lands an
auditable capability-change event in the run stream. The CLI performs
the same flip via `fabro ask --capability full`.

- Not a **role model**: today's production reality is a single
  operator; graded tiers, if fabro ever becomes multi-user, ride the
  existing token scope machinery.
- Not a **client toggle**: the tool policy enforces the mode
  server-side; a UI switch alone would be decoration.
- Not the **capability channel**: the codebase already says
  "capability channel" for the ADR-0035 run-channel socket — the
  per-run transport between a sandbox and the platform. The capability
  control is a DIFFERENT concept: the per-session tool-policy switch
  between Ask Fabro's read-only default and supervisor mode.
