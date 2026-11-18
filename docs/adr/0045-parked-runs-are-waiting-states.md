# ADR-0045: Parked decision-request runs are waiting states, not terminal ones

## Status

Accepted (2026-11-18, grilled in session with the operator; Jev
pre-screen 0.95 for the chosen option). Supersedes the terminality
clause of work#1072 ("the decision request is TERMINAL, never a failure
or a block"): the decision request REMAINS its own status kind — never
a failure, never a block — but it is no longer terminal. Implementation
is deliberately NOT in the fork.41 window (frozen, reviewed, benching):
it lands phased after the window, per the plan below.

## Context

A parked run ends with status kind `human_needed`. Today that kind is
terminal (`RunStatus::is_terminal`: succeeded | failed | human_needed |
dead, fabro-types status.rs). Three consequences of that
classification, all observed live on 2026-11-18 during the fork.41
window prep:

1. **The claim yields at the grant window.** The claim ledger's
   uniqueness binds only "while non-terminal" (GLOSSARY; the grant
   path keys on the owning run's terminality). A parked run is
   terminal, so the window opens: a fresh lap re-claimed the same item
   (runs 01M7NWYS… and 01M7QB9Y… on denkhaus/work#1312), re-verified
   the same already-landed evidence, and re-parked. One burned lap per
   pass; nothing structural prevents dozens of identical parked runs
   overnight. Filed as work#1448.
2. **No steering in place.** The park is an execution EXIT (the guard's
   `Policy::Exit`), not a suspension. A human cannot answer a question
   and continue the SAME run; every steering round costs a new lap or
   fork. The answer door (work#1345's `current_question` surface,
   work#1312's verdict actions) exists, but it dispositions a finished
   run — it cannot resume one.
3. **The work#1419 pin leaks.** "Parked HOLDS the claim" is enforced in
   the reconciler and the push-gate classification, but the grant
   window never sees it — the pin protects the wrong two of the three
   surfaces.

`blocked` is already non-terminal (the active family, with a live
`Paused { prior_block }` transition machinery) — the taxonomy treats
one waiting kind as alive and the other as dead.

## Decision

`human_needed` becomes a NON-terminal waiting state. Three commitments:

1. **The claim stays live while parked.** The uniqueness window closes
   only on a real release (refused verdict, item close, landing) —
   never on the parked run's own terminality. The re-claim stampede
   becomes structurally impossible.
2. **The answer door resumes the SAME run.** The park becomes a
   suspension with a question (checkpoint/resume, the existing question
   machinery): the operator answers, the execution continues from the
   park point. Steering is first-class, not lap-shaped.
3. **The decision request remains its own kind.** Never a failure,
   never a block (work#1072's substance stands; only its terminality
   clause is superseded). The distinction from `blocked` is the
   audience: `blocked` pauses for system reasons (quota, infra),
   `human_needed` waits for THE OPERATOR's judgment.

## Migration plan (phased, post fork.41)

- **P1 — the interim brake** (small, next window): the grant window
  respects the park pin — `claim_next` skips an item whose live claim
  is held by a parked run (work#1448, red-first pin observed GRANTED
  against the current code). Relevant until P2 lands; stays correct
  under P2 (parked = non-terminal ⇒ the uniqueness holds by itself,
  and the explicit check becomes a belt-and-suspenders assertion).
- **P2 — the engine core**: petri park-as-suspension (the guard parks
  WITH a question instead of exiting-with-a-question; vendored change,
  ADR-0034 export), `human_needed` removed from `is_terminal`, the
  answer door posts to the suspended execution and it resumes.
- **P3 — the consumers**: scheduler lap completion and watchdog
  pass-completion treat waiting as neither active nor completed (a
  parked lap is not a stall); the adjudicator queue grows an explicit
  waiting worklist (parked runs leave the terminal-without-verdict
  view); reconciler rules for live waiting claims; push-gate
  classification (waiting + held claim blocks — the operator's
  disposition releases); ps/UI badge semantics; GLOSSARY entries for
  the waiting family.

## Consequences

- The push-gate's default posture tightens: a waiting run's held claim
  blocks pushes until the operator dispositions (close citing the
  note, or answer). The 2026-11-18 window-prep wedge (one stale claim
  blocking every push) was exactly this state WITHOUT a reachable
  disposition door in the UI — the door is work#1312's landed verdict
  actions plus the item close; the class is operator-attention, not a
  semantics defect.
- The adjudicator's pass-one "refuses PR-less runs" rule stops firing
  for parks once P2 lands (suspensions are not terminal, never reach
  the queue).
- Migration risk is concentrated in P2 (vendored engine + fork export
  + resume path); P1 and P3 are ordinary platform seams. Each phase
  ships behind its own red-first pins; P2 rides ADR-0034's
  export-in-the-same-change rule.
- Epic carrier: this ADR plus work#1448 (P1). The follow-up items for
  P2/P3 file from this record when the phase opens.
