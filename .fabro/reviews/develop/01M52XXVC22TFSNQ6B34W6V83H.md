# Improve review — run 01M52XXVC22TFSNQ6B34W6V83H

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (7m33s, revisor pass — reason and cost in run detail)
- generated: 2026-10-16 18:58+0000 by revisor `fabro_ask`

---

# Recommendations for the Loop workflow — grounded in run 01M52XXVC22TFSNQ6B34W6V83H

**What this run was:** claimed seed `fabro-deb5` (conductor failed-child revise edge), one-unit edit across 5 files + tracker, all 10 stages green on first visit, zero retries, PR #410, 7m33s wall, $0.48 total (implementer alone: 275s / $0.298 = 62% of cost). Sources: run conclusion + stage transcripts (run events), journal file `.fabro/journal/01M52XXVC22TFSNQ6B34W6V83H.jsonl`, gate/evidence captures, workspace scripts. Ordered by expected impact.

---

**1. Enable run-level fabro tools so the planner's in-flight check stops running degraded** *(tool usage / error handling)*
- **What happened:** the graph pins `x.fabro_tools="fabro_run_search"` on the planner node, but the planner session's actual tool list (event seq 85) contained 14 native tools and **zero** fabro tools — the run spec carries `agent.fabro_tools: false`. The planner journaled it as a painpoint and ran step 5 (IN-FLIGHT EXCLUSION) **degraded, blind to open PRs and non-terminal runs** — in a pass where 4 of the top 5 candidates were in-flight on run `01M52A25Y210V42WAKEA8V86V7` (from planner@1 transcript + journal line 5). Per the documented engine semantics (fabro-96c6, closed: "[run.agent] fabro_tools only ENABLES; per-node x.fabro_tools NAMES"), the node pin is inert without the enable bit — and `.fabro/workflows/loop/workflow.toml` has no `[run.agent]` section at all.
- **Change:** add `[run.agent] fabro_tools = true` to `.fabro/workflows/loop/workflow.toml`; the per-node pins already restrict every other node (`x.fabro_tools=""`).
- **Expected effect:** step 5 executes as written; the duplicate-claim class (historically ~15 min + ~$0.73 per duplicate per fabro-6b58's evidence) becomes preventable in the loop lane.
- **Seed:** *new* — no existing seed covers the loop lane's run-level enable: fabro-0f6e (engine filter params), fabro-6b58 (bounding the check), fabro-9372/fabro-4bb7 (projection fields) all assume the tool is present.

**2. Add a deterministic graph↔schema label-consistency tier — this run's routing change passed the gate with zero drift** *(gate design)*
- **What happened:** the seed *retargeted conductor routing* (soft-exit swapped for a `develop -> revise` edge; edge count stayed 13), and the dot-snapshot tier stayed green because it pins only node/edge counts. Both the implementer and reviewer journaled the gap ("the dot-snapshot tier can miss routing changes like this one" — journal lines 7 and 10). The *only* thing that verified graph-label ↔ schema-enum ↔ prompt consistency was an improvised `/tmp/tmp.WXt3JUhAxO/probe.nu` the implementer wrote mid-stage (evidence capture, check 5).
- **Change:** in `scripts/validate-workflows.nu`, extend the conductor develop schema probe (or add a sibling probe) to parse the routing node's outgoing edge labels from the graph and diff them against the schema enum + validator `expected_labels` — i.e., productize the improvised probe into the battery.
- **Expected effect:** a renamed label that misses one member of the one-unit (graph/prompt/schema/validator) REDs the tester deterministically, instead of depending on an LLM choosing to write a mktemp probe.
- **Seed:** *new* — the snapshot gap is named in this run's journals and covered nowhere: fabro-eeea (shared DOT reader) and fabro-ed4d (schema-probe dedup) are adjacent infrastructure, neither adds this check.

**3. Stop running every lint 4× per run** *(tool efficiency / cost)*
- **What happened:** the implementer's recorded checks ran `just lint-nu` and `prompt-lint` twice each (evidence checks 1–2, then repeated verbatim as 6–7), and the tester ran each twice more — `scripts/validate-workflows.nu:72,81` invokes lint-nu + prompt-lint internally, then `loop-gate.nu` step 2 runs `battery-runner.nu all` which runs both again. That's 4 lint-nu + 4 prompt-lint executions; each prompt-lint render is ~9.5 KB of notes/warnings that also inflated the evidence capture (35.9 KB).
- **Change:** (a) one rule in `.fabro/workflows/loop/prompts/implementer.md`: "record each check once; after later edits re-run only the affected check"; (b) drop the internal lint calls from `scripts/validate-workflows.nu` — the battery-runner is the single lint surface (the fabro-eae5 design intent).
- **Expected effect:** ~10–20 s wall + ~19 KB of repeated output removed from every loop run; one owner for the lint verdict.
- **Seed:** *new* — fabro-eae5 unified the battery surface but left validate-workflows' internal lint calls; no seed covers the duplication observed here.

**4. Adjudicate the 23 standing prompt-lint warnings (5 stale date pins) instead of re-printing them 4× per run** *(noise / reviewer UX)*
- **What happened:** every lint render carried 23 warnings, including 5 date pins older than 45 days (e.g. `loop/scripts/loop-gate.nu` '2026-04-14' — actually the `PINNED_TOOLCHAIN` value, load-bearing; others like `develop/prompts/project-facts.md` '2026-04-14' may be stale). The implementer had to annotate "23 pre-existing advisory warnings only" and the reviewer to discount them (both transcripts).
- **Change:** add an ack marker to `.fabro/scripts/prompt-lint.nu` (e.g. `date-pin-acked: <reason>`) and refresh/ack the 5 flagged files in one pass.
- **Expected effect:** standing advisories drop out of the gate log; a *new* warning becomes visible — right now a real regression could hide inside the 23.
- **Seed:** *new* — fabro-0d16 is a different prompt-lint marker defect (SYTHETIC_IDS name drift); nothing covers date-pin acknowledgment.

**5. Make the preflight legend lane-aware — it currently instructs the loop planner to do the opposite of the lane's rule** *(prompting)*
- **What happened:** the inlined legend tells the planner "verify the PRIMARY fix surface is repo-visible (lib/ apps/ docs/ web)… loop-asset-only seeds are DIRECT WORK (skip)" — the develop-lane meaning, inverted for loop (the graph comment itself says "envelope_risk INVERTS meaning across lanes"). The planner's reasoning trace shows the miscue in real time ("deb5 is envelope_risk (names external-tool surface?)") before it self-corrected via the prompt's surface router (planner@1 reasoning, event seq 89/98).
- **Change:** in `.fabro/workflows/develop/scripts/planner-preflight.nu`, render the `envelope_risk` legend text per `--assignee` (the flag already exists).
- **Expected effect:** removes a contradictory instruction from every loop planner pass; one less failure mode on the lane's core routing decision (skipping a claimable seed, or claiming a misrouted one).
- **Seed:** *new* — fabro-ead4 (closed) added the legend with develop-lane wording only; no seed covers lane-aware legend text.

**6. Cap the `seeds ready` firehose the planner pulls every pass** *(context efficiency)*
- **What happened:** planner@1's first tracker call returned 67 ready issues / ~9.8 KB (event seq 92–95) when the preflight had already adjudicated the top 5 candidates; the planner reasoned explicitly "preflight listed only 5 candidates; ready shows 67" and moved on.
- **Change / seed:** **fabro-c3b4** (open, @loop) — "Use a top-N sd ready view in the planner instead of the full firehose." Land it.
- **Expected effect:** ~10 KB less context in the planner's early turns, faster first output, less tie-break distraction.

**7. Add the reviewer→implementer minor-fix edge before a mechanical rename costs a full re-claim lap** *(graph design)*
- **What happened:** this run's whole diff was a mechanical label rename propagated across 4 files (+ tracker) — exactly the "single mechanical edit" shape. Under the current topology, a "Changes requested" verdict routes reviewer→planner→claim_check→implementer: a full re-claim lap (~2–4 min, ~$0.10–0.40) for a one-line fix. This run approved, so no lap was paid — but the topology gap was live.
- **Change / seed:** **fabro-b659** (open, @loop) — "Add reviewer->implementer 'Changes requested (minor)' edge for single mechanical edits." This run is a fresh existence proof of the diff shape it targets.
- **Expected effect:** minor review findings cost one implementer visit instead of a full cycle.

**8. Port probe discipline to the implementer prompt (the run's only tool error)** *(error handling)*
- **What happened:** the implementer's one errored shell call (25 calls, 1 error — implementer@1 stats) was check 4: `sh -c grep -c "…" f1 f2 && ! grep -rn "…"` → exit 2 usage error from nested quote escaping, repaired by a nu probe (reviewer explicitly adjudicated it). The "each probe its own shell call; a chained probe masks a failing segment" rule exists only in the *planner* prompt.
- **Change:** add to `.fabro/workflows/loop/prompts/implementer.md`: one assertion per shell call; never nest quoted patterns through `sh -c` — use a nu one-liner/probe file.
- **Expected effect:** fewer wasted verification rounds and less reviewer adjudication of self-inflicted exit-2s.
- **Seed:** *new* — fabro-7808 covers grep *exit 1 = no-match* semantics only; fabro-b6f9 is planner-scoped; neither covers chained/quoted probes in the implementer.

---

**Not inspected:** blob-backed patch contents (`blob://sha256/…` refs aren't readable with my tools); diff-level claims above come from the evidence stage's complete diff capture and the conclusion summary (7 files, +58/−23). Timings/cost from the run conclusion; every journal quote from `.fabro/journal/01M52XXVC22TFSNQ6B34W6V83H.jsonl`.
