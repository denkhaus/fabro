# Installation-token API budget — audit and hygiene

Audit and remediation record for denkhaus/work#1364: on 2026-11-15 the
tracker provider refused reads with GitHub API 403 (rate limit exceeded,
installation ID 161242007) — `fabro work show` failed outright and the
observation feed's item-state resolution degraded to silent `None`.

## What consumes the budget

Two distinct GitHub budgets are in play; both are charged per line of
traffic above:

1. **The installation token's API budget** — every REST call
   `GitHubTracker` makes (`issues/*`, `search/issues`, `labels/*`),
   authenticated with the installation token minted for the tracker
   repository.
2. **The app-level mint budget** — every
   `POST /app/installations/{id}/access_tokens`. `work_tracker_for_project`
   (`lib/apps/fabro-server/src/server.rs`) constructs a FRESH
   `InstallationTokenSource` per call: the 60-minute token cache inside
   `token_source.rs` never survives the call, so every tracker
   construction mints a new installation token even when one from a
   minute ago would still be valid.

Per-hour consumers observed on the line:

| Consumer | Cadence / shape | Load |
| --- | --- | --- |
| Adjudicator gear | every 15 min | ~4 passes/hr; each adjudication resolves candidate item states |
| Merger gear | every 30 min | ~2 passes/hr; resolves merge/closable worklists |
| Develop ticks | per run claim | `fabro_work_next` claim + show reads |
| Global observation feed | per feed page | one tracker read per DISTINCT linked item, memoized per request (the per-record N+1 was fixed as denkhaus/work#1362/#1363 — the resolver in `orchestration.rs`) |
| Agent run tools | per agent call | `fabro_work_show` / `fabro_work_list` reads during implement/test/review passes |
| Operator CLI | on demand | `fabro work list/show`, `fabro observation feed` invocations |
| Token mints | every tracker construction | one app-budget POST per `work_tracker_for_project` call — the amplifiers above multiply it (one mint per distinct project per feed page, per gear pass, per CLI call) |

The tracker-construction-per-call mint is the largest remaining
amplifier and is NOT addressed here (see "Follow-ups").

## Hygiene landed with this item (denkhaus/work#1364)

- **Rate-limit retry with backoff**
  (`lib/components/fabro-github/src/work_tracker.rs`): a 403/429
  rate-limit answer (429, or a 403 whose body names the rate limit,
  carries a `Retry-After`, or shows `x-ratelimit-remaining: 0` — the
  same classification the pull-request door uses, denkhaus/work#1332)
  is retried up to 3 total attempts, honoring `Retry-After` capped at
  20s; a budget that stays exhausted fails as the typed
  `TrackerApiError::RateLimitExhausted` instead of a generic 403.
- **Degraded-but-explicit feed states**
  (`lib/apps/fabro-server/src/server/handler/orchestration.rs`): link
  resolution now distinguishes the tracker's not-found verdict
  (honest null `item_state`) from a FAILED read — rate limit exhausted,
  transport, configuration — which the wire names with
  `item_state_stale: true` on `ObservationWorkItemLink`
  (`docs/public/api-reference/fabro-api.yaml`). The feed never silently
  degrades to `None` again.

## Follow-ups (not this item)

- Reuse one `InstallationTokenSource` (its 60-min cache) across tracker
  constructions instead of minting per `work_tracker_for_project` call.
- Diagnostics door observability: name remaining installation quota and
  reset time so the watchdog sees exhaustion coming (the item's
  optional third bullet).
