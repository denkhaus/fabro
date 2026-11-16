# ADR-0044: The triage separation — the triager gear, links at create, the manage door

## Status

Accepted (2026-11-15, grilled in session; REVISED the same day before
any implementation landed — the first draft extended the adjudicator's
worklist to merger runs, the grill replaced that with the triager
separation below). Extends ADR-0039 decisions 3, 7 and 12 (the
adjudicator's filing arm moves out; the worklist stays `develop`).
Amends the link-writing posture of ADR-0038 decision 4: links were a
user-only write; create-time linking adds a worker's write through the
create door, and the manage door adds the triager's link-existing
write. Epic carrier: denkhaus/work#1365.

## Context

The 2026-11-15 grill of the painpoint pipeline found the triage numbers
hollow. Four structural breaks, each verified against live data
(mirtuell.net) and code:

1. **Filings never link.** The adjudicator's agent pass files follow-ups
   through `fabro_work_create` (title, body, labels — no observation
   reference), while the observation-link door is user-only
   (`refuse_worker` on link and unlink both). The adjudicate prompt's
   "filing is enough" sentence did not hold: every agent filing stayed
   unlinked. All 71 links behind the filed/closed painpoints were
   written in ONE manual operator backfill (2026-11-12 23:21 UTC);
   zero links before or since.
2. **No terminal state for reviewed-no-item.** The triage state is a
   pure derivation over links (`untriaged | filed | in_progress |
   closed`). A painpoint the pass consciously does NOT file has no
   representation — it reads `untriaged` forever, indistinguishable
   from never-reviewed. The count measured nothing.
3. **Merger painpoints are ownerless.** The adjudicator worklist filter
   is `develop`-only; merger runs (every 30 minutes) record painpoints
   (the parked-conflict class among them) that no gear ever reviews.
   And the first draft's fix (worklist `[develop, merger]` with a
   stretched verdict semantics for itemless runs) was the wrong lens:
   the verdict queue is RUN-driven and settles — triage is
   RECORD-driven and must outlive every run. Future painpoint-producing
   workflows would each have needed verdict semantics just to get their
   painpoints seen.
4. **The no-filing answer has no address.** `fabro_observation_record`
   writes only on the CALLER's own run (verified: params are category
   and text, no target run), so a triager's decision note would land on
   the triager's run with no keyed connection to the painpoint — the
   derivation would need a global text scan, exactly the prose-marker
   pattern ADR-0039 decision 11 retired.

## Decision

1. **Filing IS linking.** `fabro_work_create` — the agent tool and the
   CLI facade alike (door symmetry, ADR-0039 decision 12) — gains
   optional observation references (`ObservationRef`, the canonical
   `fabro_types` type reused through `with_replacement`): run id plus
   seq, repeatable. The server writes the `work_observation_links` rows
   in the same request, idempotently; dedupe at the link stays. The
   manual link/unlink door stays user-only: curation remains an
   operator power; the create door is the worker's link write.
2. **The manage door.** One keyed platform row per observation
   (`observation_triage_decisions`, PK `run_id, seq`) — never a note on
   the caller's own run (break 4). The door
   (`POST /api/v1/observations/manage`, agent tool
   `fabro_observation_manage`, CLI `fabro observation manage`) hosts
   the triage actions: `no_filing` (the triager judged no item
   warranted; the record derives `reviewed`) and `link_existing`
   (an already-filed item carries the finding — the link row is
   written beside the decision, dedupe at the link, ADR-0038 decision
   5; the pre-fix backlog of adjudicator filings drains through it).
   A reason is mandatory — a decision without a rationale teaches
   nothing. Upsert: the latest decision on a record wins. Writers:
   user principals always; a workflow run only when its workflow is
   the triager (the mark door's allowlist posture).
3. **The triager gear.** A fourth serial scheduler gear (workflow slug
   `triager`, tick every 15 minutes, `on_overlap: skip`, watchdog
   window 45m) owns GLOBAL painpoint triage. Its sweep is LLM-free and
   feed-driven: one read of the global observation feed
   (`category=painpoint, triage=untriaged`, oldest first, fact-head
   dedupe, batch-capped); an empty feed is natural completion — the
   convergence steady state. Its agent pass disposes each painpoint:
   file linked, link existing, or no_filing. It never verdicts, never
   merges, never closes. Painpoints from EVERY workflow reach it —
   merger included — and no future workflow needs verdict semantics to
   be seen.
4. **The adjudicator slims to verdicts and closes.** The stage-C
   trigger STAYS: a develop run's painpoints still force the
   adjudicator's agent pass, because the merge verdict reads them as
   input — but the filing duty moves to the triager (the lane-drained
   tranche-demand filing is the one routed exception: a lane signal is
   a worklist concern, not a painpoint). The adjudicate prompt's
   filing section retires; "tilt toward refused with a filing" becomes
   "tilt toward refused — the triager files what the refusal leaves
   open".
5. **Triage convergence is the throughput metric.** `untriaged` means
   "never reviewed by any pass, younger than one tick" and nothing
   else. No counters ride the pass envelope.

## Consequences

- The prompt sentence "link nothing manually; filing is enough" becomes
  true (it was the bug's camouflage).
- Operator backfill sessions (2026-11-12) become unnecessary; the
  backlog drains through the chain — existing-but-unlinked items
  through `link_existing`, the rest through fresh linked filings or
  no_filing decisions.
- The global feed becomes a gear worklist: its triage-filtered reads
  must stay cheap (the N+1 fix, denkhaus/work#1363, landed as this
  epic's prerequisite — the derivation is two-stage, item states
  resolve once per distinct item per request, and `untriaged`
  short-circuits without tracker I/O at all).
- The create API surface changes: OpenAPI spec, progenitor types, TS
  client regen, and `fabro-tool` params ride one change (the API
  workflow); `ObservationRef` is shared vocabulary (`fabro-types`),
  proven by a parity test.
- The wire gains the `reviewed` triage state; the glossary documents
  the full derived lifecycle and the triager beside the adjudicator.
- One more gear to operate: automation row `triager@fabro`, watchdog
  coverage (`GEARS` row), Slack terminal notifications — the standing
  gear pattern, no new mechanism.
