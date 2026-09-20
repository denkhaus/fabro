# ADR-0022: Judgment layer (System One) — shadow-first semantic gates

- Status: Accepted
- Date: 2026-09-20
- Deciders: user (grilling rounds 1-2 in chat), agent (analysis + live probes)
- Seeds: fabro-8e13 (shadow hook), fabro-d4c6 (evaluation replay), fabro-deee (reviewer pre-table), fabro-b506 (fabro-llm provider)
- Supersedes: none (extends the report-only preflight doctrine of fabro-83df; dogfooding per ADR-0012)

## Context

The loops' LLM stages make semantic decisions at reasoning-model prices
(measured planner laps: $0.019-0.143 / 28-94 s), while deterministic
preflight scripts cover only greppable facts (tracker_guard, planner
preflight). Between the two sits a class of judgments that is semantic
but not generative: is this changed file residue, is this failure flaky,
is this seed's basis moot. TypeSafe's System One model Jev answers
exactly those as typed judgments (choice/score/noul) with calibrated
probabilities and confidence, input-only billed at ~$0.042/Mtok; live
probe 2026-09-26: 3-question triage in 0.59 s for $0.00003. Gateway
reality: the direct TypeSafe API is waitlist-only; OpenRouter's
/api/alpha/decisions is the only public access today.

## Decision

1. Shadow-pilot first: judgments are journaled, no engine decision
   consumes them (fabro-judgment-v1 stream, own file per run).
2. Vehicle: non-blocking host-side stage_complete hook
   (`judgment-shadow`), sandbox=false — the OpenRouter key stays in
   server env and never enters a run sandbox or a prompt.
3. First use case: reviewer residue adjudication + verdict pre-screen
   in ONE fan-out call over the evidence state.
4. Advisory by default. Autonomy only for low-consequence decisions
   (retry triage) at confidence >= 0.85; irreversible calls (seed
   close, review verdict, tool block, model choice) stay advisory or
   human-gated.
5. Fail-open + degraded-mode journal on every error path, mirroring
   the preflight doctrine.
6. Gateway: OpenRouter now; provider keeps base_url configurable for
   the direct TypeSafe API once waitlist opens.
7. Internal-first (own loops); a product-level judgment stage type is
   a later decision gated on pilot evidence.
8. Canonical domain term: "Judgment" (hook, provider, model-catalog
   kind). Vendor and model names appear only in catalog entries.
9. Evaluation before any threshold goes live: replay run journals and
   review records, derive agreement + confidence report, then set
   thresholds as graph attributes with code defaults.
10. Tests run against a scripted twin (twin_openai pattern); the
    quality gate never needs live OpenRouter or credentials in a
    sandbox. Live validation is the shadow phase itself.

## Consequences

A cheap semantic tier opens between deterministic preflights and
reasoning laps without weakening any existing guard (conditions still
outrank every model call). New external dependency (OpenRouter alpha
endpoint) is mitigated by fail-open and version-pinned model ids.
Calibration in fabro's domain is unproven until the fabro-d4c6
evaluation report exists; fabro-deee (reviewer integration) is blocked
on that report by dependency.
