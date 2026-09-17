# ADR-0022: Revisor filing-balance duty (loop intake regulation)

- Status: Accepted (user decision 2026-09-17, chat session)
- Scope: revisor workflow file stage, loop tracker economics
- Supersedes: the parked anti-circling options (i)/(ii)/(iii) of fabro-8776

## Context

The revisor files seeds; the develop line drains them at a hard cap of one
seed per run (gate + review + auto-merge). fabro-8776's measurement
(2026-09-04..11) found 226 creates vs 91 closes per week and 65% of closures
being loop self-optimization - the loop generates demand about itself and
then burns its own capacity satisfying it. Current window: 155:80, 306 open,
median age 9.9d; `backlog_stagnation` 0.709 and `drain_deficit` 0.484 are
the dominant friction components. The architect pass 2026-09-17 re-derived
intake regulation as its highest-leverage finding (fabro-c77a).

## Decision

1. **Filing-balance duty (Bilanzpflicht)**: per revision pass, the revisor
   may file at most as many new seeds as the SAME pass closed as
   stale/superseded. Implemented closes do not count as credit (they are the
   develop line's work, not cleanup).
2. **Overflow semantic**: surplus findings ride the pass journal
   (`observations`, named with their concrete change) and the NEXT pass may
   re-file them against its own balance. Nothing is dropped; ADR-0002
   (painpoints/journals are the only platform channel) stays untouched -
   the gate sits at filing, not at reporting.
3. **Same-file consolidation is unconditional**: findings targeting the same
   file merge into ONE multi-arm seed (fabro-ae74 structured arms schema),
   independent of the balance.
4. **Parked sibling options** (decided by the agent 2026-09-17, user
   delegation): (ii) root-fix quota and (iii) loop-share P3 demotion stay
   parked - one regulator at a time keeps effects attributable. Review
   checkpoint: when the friction trend after >= 1 week of Bilanzpflicht
   passes shows creation still outpacing drain 2:1 or worse, revisit (ii)/(iii).
   (iv) 15-min needs-user triage is adopted as standing ceremony: the
   line-watch report carries a NEEDS-USER QUEUE section; a triage/grill
   session is proposed when the queue reaches 5 items or one blocks the line.

## Consequences

- Creation couples to drain: the revisor must EARN filings by cleaning the
  backlog; the Medium prompt-tweak cluster collapses via consolidation.
- Genuine new findings can be delayed by up to one pass (bounded by the
  journal re-file path); needs-user and security items are exempt from the
  balance (they are not loop demand).
- Expected effect: `backlog_stagnation` and `drain_deficit` fall over the
  next weeks; the friction verdict should leave `grind` territory.
- The revisor's file-stage prompt + graph change land as ONE unit
  (ADR-0008); implementation seed: fabro-c77a (assigned to the line).
