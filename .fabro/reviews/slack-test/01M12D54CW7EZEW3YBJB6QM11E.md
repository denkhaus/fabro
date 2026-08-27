# Improve review — run 01M12D54CW7EZEW3YBJB6QM11E

- workflow: slack-test
- branch integrated: denkhaus-lab
- status: succeeded (completed), 0.1 min, $0.0
- generated: 2026-08-27 22:06+0200 by `fabro ask` with `scripts/prompts/improve.md`

---

# Improve-review — run `01M12D54CW7EZEW3YBJB6QM11E` (`slack-test` / Smoke)

**What actually happened (from run events):** `start → ping → done`, sole work `echo slack smoke` — exit 0, 12 bytes, **165 ms** (seq 27–28). Zero inference, zero tokens, $0. End-to-end created → sandbox stopped: **15.65 s** for 165 ms of work (~1% duty cycle). Checkpoint commit `d328421` had `files_changed: 0` and was pushed twice. The 43 pre-session events contain **zero `notification.*` events**. Two notifiers were configured (seq 1: `terminal` → `#dev-fabro`, `smoke` → `#fabro-ops`).

**Improved since the last two reviews (01M129B2, 01M129EZ) — credit where due:** channels are now split (`#fabro-ops` smoke vs `#dev-fabro` terminal, commit `ee0ff22`), the 3.4 s `sandbox.ready→setup.started` gap is gone (now 363 ms), and prepare is cheaper (`mise install` 113 ms, `just bootstrap` 570 ms).

## Recommendations, by expected impact

**1. (Third consecutive review: the run still cannot fail for the reason it exists.)** Goal is "Verify Slack notifications", but `ping` in `.fabro/workflows/slack-test/workflow.fabro:3` only echoes; success == "echo exited 0". No `notification.*` event exists in the log, so a repeat of the documented `channel_not_found` incident (`project.toml:74–79`) would still be a green run — including for the brand-new `#fabro-ops` route, whose existence this run does not prove.
**Change:** add a `verify` node after `ping` that asserts delivery for this run (Slack `conversations.history` matching a run-unique token, or gate on an engine `notification.delivered` event once it exists — see rec 2).
**Effect:** broken channel/credential turns the run red instead of green.

**2. Engine: emit `notification.delivered` / `notification.failed` events.** Both notifiers are visible in effective settings (seq 1) yet leave no trace in the journal — the silence that hid the original 403/channel failures is still the default.
**Change:** platform-side, one event per post with channel, event type, and error code.
**Effect:** notification health becomes queryable per run, and rec 1 gets a free oracle (assert on the event instead of calling Slack).

**3. Skip persistence on zero-diff runs; dedupe pushes.** Empty commit `d328421` (`files_changed: 0`, seq 33–34) was pushed to `fabro/run/…` at seq 35 **and again** at seq 38 (~2.5 s later, same SHA, no commit in between). Three metadata snapshots cost 2 155 + 1 989 + 2 341 = **6.5 s** vs 165 ms of work, and every ping leaves two remote branches (run + meta).
**Change:** in `.fabro/workflows/slack-test/workflow.toml`, override `[run.run_branch] push = false` / `[run.meta_branch] push = false` for this workflow; engine-side, skip commit+push when `diff_summary.files_changed == 0` and skip a push with no new SHA.
**Effect:** smoke end-to-end drops from ~15.6 s toward ~5–6 s; no empty commits or branch clutter per ping.

**4. Add a retry policy to `ping`.** Seq 26 shows `attempt: 1, max_attempts: 1`. This repo already has the vocabulary — `retry_policy="standard"` on nodes in `.fabro/workflows/develop/workflow.fabro:48`.
**Change:** add `retry_policy="standard"` to the `ping`/`verify` nodes in `workflow.fabro`.
**Effect:** a transient Slack 429/clone hiccup self-heals instead of failing the smoke run (matters once rec 1 asserts against a live API).

**5. Make the payload correlatable.** Output is the constant string `slack smoke` (blob at seq 28); a human can't match a Slack message to this specific run.
**Change:** `echo "slack-smoke $(date -u +%FT%T.%3N) $RUN_ID"`-style token (feeds rec 1).
**Effect:** run log and Slack message share a verifiable token.

**6. Minor telemetry: `wall_time_ms` doesn't reconcile.** `run.completed` reports `wall_time_ms: 7164`, but event timestamps give `run.started` (20:04:09.503) → `run.completed` (20:04:20.028) = 10.5 s; no event-bounded window equals 7 164 ms.
**Change:** define the measurement window in the event schema (or emit `timing_window_start/end`).
**Effect:** downstream cost/latency reporting from events becomes trustworthy.

**Not applicable:** prompting/tool-usage — zero inference stages, so there are no transcripts or model calls to optimize; all leverage is in the assertion (recs 1–2) and overhead (rec 3).
