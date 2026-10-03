# Merge report: upstream/main 8e67377a9 -> cfd484360 (v0.356.0 content) — landed via park branch

**Landing:** merge commit 0cac5a8a3 parked on park/merge-upstream-v0.356 (session
interrupt 2026-09-15 07:29), integrated onto denkhaus conflict-free as c7cc55aaa
(+ import fixes 4b0fd01e2), pushed 2026-09-15. Same-day follow-up: salvage PR #152
(802942443) landed the 429-stranded fabro-7627 resume work on top.

## 1. Merge identity
- Range 8e67377a9..cfd484360, 25 commits, up to #875 no-fabro-pricing.
- After landing: upstream only 1 commit ahead (170291b9f version bump).
- Second merge (park -> denkhaus over #146-#151) auto-resolved the expected
  server.rs/tests.rs overlaps; #148 supervisor suites re-verified green.

## 2. Verification
- Full nextest (8 pkgs, env-clean): 4944/4957; the 13 fails are SESSION
  ENV LEAK (OPENAI_API_KEY overrides vault fixtures) — proven twice:
  identical fails on clean pre-merge denkhaus, and 31/31 green with the
  env keys unset.
- clippy --workspace --all-targets -D warnings: green after removing two
  merge-leftover unused imports (fabro-types test import, finalize.rs
  FailureDetail moved into the test module).
- fmt green; web typecheck + 714/714 (later 719/719 with salvage pins).
- Fork presence suites green at HEAD (fork_seam, fork_line_recovery,
  fork_duplicate, fork_taxonomy, fork_legacy, #148 staleness_supervisor).

## 3. Upstream changes (highlights)
- billing -> usage rename end to end (#874 one-usage-type): Usage/ModelUsage
  types, stage/run events carry usage{usd_micros,source}, web Billing tab ->
  Usage tab, TS client regenerated (billing-*.ts -> usage/run-usage-totals).
- #875 no-fabro-pricing: lithos-llm prices every response once; cost source
  catalog; unknown costs render unknown (not zero).
- Pebble repin a39f43e (older stored session records refused clearly),
  sandbox-driver 64c14b8, Daytona guard + slow-timeout fixes, exec --verbose,
  twin-mode test fixes.

## 4. Our-code impact
- Fork legacy normalizers adapted to the usage vocabulary (KEY billing->usage,
  now emit ModelUsage/Usage + cost{usd_micros,source}).
- clamp_legacy_token_counts ported to the JSON boundary; the store projection
  test removed as type-impossible with u64 TokenCounts.
- NEW fork-only files (user directive 2026-09-15): run_event/fork_legacy_read.rs
  (fabro-types), pipeline/fork_terminal_taxonomy.rs (fabro-workflow),
  server/fork_taxonomy_tests.rs (fabro-server) — presence-pinned, touchpoints
  rows added.

## 5. Conflict-policy additions (this class)
- usage-rename class: when upstream renames a wire/event vocabulary
  (billing->usage), fork normalizers and fixtures must be adapted to EMIT the
  new vocabulary while still READING the old one (fork_legacy_read) — do not
  keep parallel old-type emitters.
- billing-key-rename in JSON boundaries: struct-literal fixtures fail as
  E0559 "no field named billing; available: usage" — the compiler enumerates
  the worklist; adapt fixtures mechanically (BilledModelUsage -> ModelUsage).

## 6. Follow-ups filed
- fabro-581b: extend mold linker anchoring to toolchain/run image + CI
  (needs-user per ADR-0019 — capability surface).
- fabro-986b live-evidence extension: executor no-edge rewrite defeats the
  run-level park (runs 01M2J1BA2J/01M2J191AG) — fix direction documented.
- pebble_agent.rs skill-expansion tests still live in an UPSTREAM file
  (extraction candidate, next merge pass).

## DEPLOY
- PENDING at report time: local docker daemon down, fabro-tofu checkout
  missing on this machine. Path decided with user: tag + CI image
  (cargo dev release --nightly -> ghcr nightly) or classic image-release
  once docker is back; rollout via fabro-tofu apply.
