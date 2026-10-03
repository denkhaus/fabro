# ADR-0024: Platform freeze on 0.362 — dep-fork tracking and cherry-pick intake

- Status: Accepted (2026-10-01)
- Deciders: user (GO), agent (assessment; argued for merge-cadence alternative C, overruled — user priority: reliable productivity with fabro NOW; real work creates real improvement needs)
- Evidence: drift assessment 2026-10-01 (merge-tree dry-runs), upstream
  cadence analysis (merge-base 1b4fb1528 of 2026-09-26 → 35 commits / 9
  nightly minors in 5 days), user's Daytona cost exclusion, the nu-agent
  sprint-system model (../nu-agent, 97 sprints without upstream coupling)

## Context

The fork tracked upstream fabro main reactively: a 5-minor drift threshold
triggered merge windows. At upstream's actual cadence (nightly minors every
1–2 days, agent-driven), that threshold is hit every 2–3 days — the policy
degenerates into permanent merging or big batch windows (the pending
0.362→0.371 window: 35 commits, 8 textual conflicts + 1 architectural
collision, est. 2–3 focused days). Meanwhile the line ships: the merge tax
is the volatile cost item, and the largest share of current upstream drift
(Daytona defaults/normalization) has zero value here — Daytona will not be
used, for cost reasons (user, 100%).

## Decision

1. **Freeze the platform base** at `0.362.0-nightly` (merge-base
   1b4fb1528, 2026-09-26). No routine merges of fabro-sh/fabro main.
2. **Dep forks stay tracked**: petri and pebble forks continue to merge
   upstream — selectively. A fork merge must compile and pass gates against
   OUR frozen base; an upstream commit that requires fabro-side changes we
   do not have is skipped and recorded. The fork branch is our intake
   valve, not upstream's delivery channel.
3. **Main-repo intake is cherry-pick only**: a specific upstream fix lands
   as a bounded, justified cherry-pick (feature series or PR range) with
   its own verification. First registered candidate (not pulled now):
   the sandbox-side publication rewrite (#913) — it would retire the
   server-side checkout path and its cache pain (fabro-5726 class).
4. **Quality investment shifts inward**: hardening via our own architect
   workflow (improve-codebase-architecture, seeded findings) on a sprint
   cadence (nu-agent model: sprint ledger + architecture gate every 3rd
   sprint + immediate seed filing), plus regular code reviews. Real
   product work with fabro is the driver of platform improvements —
   needs discovered in use, not in merge windows.
5. **Upstream posture flips to selective offering**: general, non-strategic
   fixes (e.g. the petri fork's full-history clone fix, hook context file)
   are offered upstream when ready; strategic assets (loop lane, seeds
   integration, develop workflows) stay fork-private. Rationale: the
   endgame is convergence (upstream adopts our features) or a deliberate
   wholesale switch — both require a portable `.fabro` layer and an open
   offer channel, not permanent silence.
6. **Portability over mergeability**: the fork-presence discipline
   (fork-only files, thin seams, presence pins) stays binding — its purpose
   changes from surviving the next merge to keeping the product layer
   portable onto a future platform version.

## Consequences

- Merge windows disappear from the calendar; intake cost is paid per
  cherry-pick, on demand, bounded.
- A later full merge or wholesale switch gets MORE expensive over time —
  accepted deliberately: the alternative path is convergence or a one-time
  port project, not continuous mergeability.
- Security or critical fixes in fabro main arrive via cherry-pick; drift
  watching becomes informational (a watch item), not action-triggering.
- The architect automation's schedule is enabled (daily 05:00 UTC, workflow
  self-gates on friction verdict + 48h cooldown) — see also the sprint
  ledger seed filed with this ADR.
