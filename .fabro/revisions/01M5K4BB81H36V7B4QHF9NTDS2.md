# Revision — run 01M5K4BB81H36V7B4QHF9NTDS2

- status reviewed: failed
- review: .fabro/reviews/develop/01M5K4BB81H36V7B4QHF9NTDS2.md
- items filed: none — tracker unreachable (`no work project is bound to target repository denkhaus/fabro`; no project id exists server-side), zero balance credit this pass; all surviving findings journaled as overflow below
- balance: 0 / 0 — no credit this pass (no same-pass stale/superseded closes possible with the tracker down)
- basis: run 01M5K4BB81H36V7B4QHF9NTDS2, workflow version absent, commit a4447acc3ab4c1b453d7bc472cb8fb79af462d7f
- revised_at_commit: a4447acc3ab4c1b453d7bc472cb8fb79af462d7f (ADR-0015: engine drift signal for later judgement)

## Findings

**1. claim-check: retirement-intent window + quote the offending body line** — overflow
- overflow: claim-check retirement-intent window + quote the offending body line — in `.fabro/workflows/develop/scripts/claim-check.nu` and the shared window logic in `anchor_check.nu`, mirror `creation-named?` with a retirement window (rooted path immediately followed by "no longer exists"/"never existed"/"retired" ⇒ non-citation) and print the quoted body snippet each extracted path came from in the bounce payload; effect: the bounce class that killed this run (3 identical bounces on a retirement parenthetical in seed fabro-d48f's body, $0.329/292s of a 330s run) disappears outright and genuine stale citations resolve in one planner pass. Evidence: `seeds show` bodies byte-identical across claim_check@1–@3; breaker killed the run at event seq 577.
**2. Engine: circuit-breaker failure signatures sliced mid-token** — overflow
- overflow: circuit-breaker failure signatures sliced mid-token — engine-side, wherever the deterministic-cycle breaker builds its `node|class|message` signature, cut at a line/token boundary and lead with the node label (e.g. "claim_check: stale seed body — scripts/closeout-race.nu") instead of a mid-body slice; effect: the run's terminal `failure_reason` (and the Slack `run.failed` notification) stops reading "…acker-guard,closeout,evidence,gate-bounce}.nu…" word salad; humans triage the actual cause. Evidence: failure_reason at seq 577 begins mid-token inside a brace expansion.
**3. Graph: generation-capped deadlock exit for claim_check** — overflow
- overflow: generation-capped deadlock exit for claim_check — in `.fabro/workflows/loop/workflow.fabro` and develop's graph, add `claim_check -> exit [x.kind="deadlock", label="Claim-contract deadlock (needs a human)", condition="nodes.claim_check.generation >= 2"]` mirroring the existing tester/reviewer generation>=3 caps; effect: a toxic planner↔claim_check loop stops one full planner visit earlier (~$0.07/~65s per occurrence) and terminates as a labeled human-handoff deadlock instead of an undifferentiated engine `failed(workflow_error)`. Evidence: claim_check's only failure edge bounces to planner with max_visits=20; the breaker fired only after a third planner session.
**4. Planner prompt: verify what the gate sees after `--set-description`** — overflow
- overflow: planner post-correction verify loop — in `.fabro/workflows/loop/prompts/planner.md` step 3, after any `seeds update --set-description`, run `seeds show <id> --format json` and grep the description for every rooted token (`scripts/…`, `.fabro/…`, `lib/…`) before routing Seed claimed (delete retirement mentions, never annotate), and skip the title call on re-claim visits; effect: bounce corrections land in one pass (planner@2/@3 both claimed "corrected" while the stored bodies were identical), and the per-re-claim 4-call/3-error `fabro_run_set_title` churn stops. Evidence: three `seeds show` captures identical across bounces; planner stats show set_title 4 calls/3 errors per re-claim visit.
**5. Bind the ADR-0032 work project to target repo denkhaus/fabro (server-side)** — overflow
- overflow: bind the ADR-0032 work project to targetRepo denkhaus/fabro server-side — upsert the central tracker's work project so its targetRepo matches `denkhaus/fabro` (explicit ids `fabro`, `denkhaus`, `fabro/denkhaus/work` all return "work project not found"); effect: every `fabro_work_*` run tool stops failing with "no work project is bound", restoring revisor filing/closeout-race checks, develop claim tracking, and the entire ADR-0032 loop surface; third consecutive pass blocked by this gap. Capability-affecting when filed (ADR-0019: it gates agent-reachable tool availability) — must file as needs-user, engine-mediated fix direction only.

Duplicates dropped (per the ask answer's snapshot searches; live tracker re-check impossible this pass — tracker down):

- preflight non-terminal in-flight filter — duplicate_of fabro-997d (adjacent fabro-6b58, both open per snapshot)
- first-query status enum naming (fabro_run_search/`queued`) — duplicate_of fabro-1cde (open)
- claim-phase firehose cut (`seeds ready` top-N) — duplicate_of fabro-c3b4 with fabro-a08a (both open, already filed)

Note: filing-time closeout re-check (Step 1) ran degraded — both `fabro_work_list {"query": "01M5K4BB81H36V7B4QHF9NTDS2"}` and the limit-100 fallback failed with the binding error; verdicts rest on the analyze-stage ask evidence. The overflow ledger `open` listing was consulted; no open overflow entry covers these five themes (nearest neighbors are claim-check spec-paths bouncing and engine run_search lint, different mechanisms).
