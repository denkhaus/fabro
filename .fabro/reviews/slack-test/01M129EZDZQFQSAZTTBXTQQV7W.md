# Improve review — run 01M129EZDZQFQSAZTTBXTQQV7W

- workflow: slack-test
- branch integrated: denkhaus-lab
- status: succeeded (completed), 0.1 min, $0.0
- generated: 2026-08-27 21:01+0200 by `fabro ask` with `scripts/prompts/improve.md`

---

## Improve-review — run `01M129EZDZQFQSAZTTBXTQQV7W` ("Verify Slack notifications")

**What actually happened (from run events):** 3-node graph `start → ping → done`; the only work was `echo slack smoke` (exit 0, 12 bytes, 170 ms, seq 27–28). Wall time run.started→completed was 8.48 s; end-to-end created→completed ≈ 19.6 s. Zero model usage, zero files changed. No notification-delivery events appear anywhere in the 63-event log.

### Recommendations, by expected impact

1. **The smoke test verifies nothing about Slack.** Node `ping` in `.fabro/workflows/slack-test/workflow.fabro` just echoes; run success == "echo exited 0". The exact failure this workflow exists to catch — per the comment in `.fabro/project.toml` (lines 74–79), every notification previously failed silently with `channel_not_found` — would still produce a "succeeded" run today, because no delivery result is asserted or even recorded as an event.
 **Change:** have `ping` emit a unique token (`echo "slack-smoke $(date -u +%FT%TZ.%3N)"`) and add a `verify` node that checks the token landed in `#dev-fabro` (Slack `conversations.history` via a small script, or assert on a fabro `notification.delivered` event if the engine can expose one as a gate).
 **Effect:** broken channels/credentials fail the run instead of green-lighting it.

2. **Duplicate `run.completed` routing.** `workflow.toml` smoke notifier subscribes to `["run.started", "run.completed", "run.failed"]`, and project-level `terminal` (`.fabro/project.toml` line 83) subscribes to `["run.completed", "run.failed"]` — both to `#dev-fabro` (both confirmed in the run's effective settings, seq 1). Every completed run double-posts.
 **Change:** drop `run.completed` (and `run.failed`, already covered by `terminal`) from `[run.notifications.smoke]` events, leaving `["run.started"]`.
 **Effect:** one message per lifecycle event; `terminal` keeps credential-rot visibility as intended.

3. **Git/meta overhead dwarfs the work: 7.4 s of snapshots + empty commit + double push for a 170 ms echo.** From events: init metadata snapshot 3 414 ms (seq 20–21), checkpoint snapshot 1 880 ms (seq 31–32), finalize snapshot 2 083 ms (seq 39–40); commit `fa387c5` with `files_changed: 0` (seq 33–34); two `git.push` events on the same run branch 1.1 s apart (seq 35, 38) with no new SHA in between.
 **Change:** in `workflow.toml`, disable `run_branch.push` and `meta_branch.push` for this workflow (both are per-run settings in the spec), and/or have the engine skip the checkpoint commit+push when `diff_summary.files_changed == 0`.
 **Effect:** smoke wall time drops from ~8.5 s toward ~2–3 s and stops creating two remote branches per ping.

4. **No retries on the one node that needs them.** `stage.started` for `ping` (seq 26) shows `attempt: 1, max_attempts: 1`. A smoke run whose purpose is catching flaky infra (clone 1.7 s, push, Slack webhook) has zero tolerance for transients.
 **Change:** set `max_attempts = 2` (short backoff) on the `ping`/`verify` nodes in `workflow.fabro`.
 **Effect:** transient webhook/clone failures self-heal instead of requiring a manual re-run.

5. **Make the ping payload correlatable.** `echo slack smoke` output is stored as a blob (seq 28) and carries nothing unique; a human checking Slack can't match a message to this specific run.
 **Change:** include a timestamp/UUID in the echoed string (same change as rec 1's token).
 **Effect:** the Slack message and the run log share a verifiable token — the manual fallback to rec 1 becomes trustworthy.

6. **Minor: prepare steps and an unattributed gap.** `mise install` (185 ms) + `just bootstrap` (779 ms) ran for an echo-only graph (seq 12–17), and there's a 3.386 s gap between `sandbox.ready` (18:59:37.511) and `setup.started` (18:59:40.897) that no event accounts for. If per-workflow `prepare` overrides are supported in `workflow.toml`, empty them for `slack-test`; the 3.4 s gap is engine-side and worth an event of its own.

**Not applicable:** prompting/tool-usage improvements — this run had zero inference stages (usage all zeros), so there are no stage transcripts or model calls to optimize; the leverage is all in assertions (rec 1) and overhead (recs 2–3).
