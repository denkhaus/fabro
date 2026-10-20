# Revision — run 01M5CX74R17T0GSNSYHXX0SC4Q

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M5CX74R17T0GSNSYHXX0SC4Q.md
- seeds filed: none — zero balance credit this pass (0 same-pass stale/superseded closes); 1 new overflow journaled, 1 overflow-dup link, 6 findings dropped as duplicates of existing open seeds
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M5CX74R17T0GSNSYHXX0SC4Q, workflow version absent, commit 0173eaf3206379dc91b96117de4f9cb32f883c18
- revised_at_commit: 0173eaf3206379dc91b96117de4f9cb32f883c18 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Make brief mechanism-divergence visible** — overflow
The brief prescribed `--no-tests fail` (probe-verified flag); the shipped diff contains no such flag — the implementer silently substituted a Summary-line parser + `ran < names` guard (run diff: the `cargo nextest run` invocation line unchanged); Loop Gate and reviewer both went green. Concrete change: add a bullet-class marker (`required-literal:` vs `acceptance:`) in `.fabro/workflows/loop/prompts/planner.md` and a matching tier in `.fabro/workflows/loop/scripts/loop-gate.nu` that greps the run diff for each `required-literal`, bouncing to implementer on miss. Dedup: `seeds search divergence` and `seeds search literal` return nothing covering it (fabro-8ac5 is lane-toml parity); closed fabro-1314 covered only planner-side phrasing.

- overflow: Make brief mechanism-divergence visible — marker (`required-literal:` vs `acceptance:`) in `.fabro/workflows/loop/prompts/planner.md` plus a loop-gate.nu tier grepping the run diff for each required-literal, bounce on miss; effect: silent mechanism substitution (this run: brief's `--no-tests fail` dropped for a Summary-line parser, gate+reviewer green) becomes a deterministic RED instead of passing unnoticed.

**2. Triage Loop Gate warning noise** — overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in 01M2XQJC3TWRMJ1128WRXRQXH.md)
Same theme already open: the run's 35 warnings (17 date-pin, 18 routing-named) are exactly the noise class that open overflow covers (allowlisting the date-pin nag and routing-intent schemas); this finding's variant (waived marker in `.fabro/scripts/prompt-lint.nu` + collapse routing-named notices to one line per file) rides the existing entry rather than opening a second one.

**3–8. Duplicates of existing open seeds** — dropped, per analyst dedupe (id lookups verified via `seeds show`/tracker):
- top-N ready view → duplicate_of: fabro-c3b4
- nushell-scripts skill → duplicate_of: fabro-d19e
- in-flight bound + fabro_run_search recipe → duplicate_of: fabro-6b58
- shared prompt-cache prefix → duplicate_of: fabro-944d
- 100-char title cap → duplicate_of: fabro-b529 (prompt-side cap note rides its demand)
- `nextest show-config` probes → duplicate_of: fabro-e137
- reviewer substring-collision → duplicate_of: fabro-97fd

Closeout-race re-check at filing time: count 0 — no seed citing this run.
