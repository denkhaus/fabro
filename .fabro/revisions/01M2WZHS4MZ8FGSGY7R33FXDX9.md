# Revision — run 01M2WZHS4MZ8FGSGY7R33FXDX9

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2WZHS4MZ8FGSGY7R33FXDX9.md
- seeds filed: fabro-95e1 — Bake libssl-dev into the run toolchain image (capability-affecting, needs-user)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass (no same-pass stale/superseded closes)
- basis: run 01M2WZHS4MZ8FGSGY7R33FXDX9, workflow version 651b1a6e2a981a032674237f71e9a360311e8ba2901832de7fa9b0b3046836b7, commit 104533378c921dff0ad5aa7502f814f27afcc1ef
- revised_at_commit: 104533378c921dff0ad5aa7502f814f27afcc1ef (ADR-0015: engine drift signal for later judgement)

## Findings

### Qualitygate workspace-mode fallback must still run the touched-crate nextest tier
- overflow-to-journal (no balance credit this pass; re-file next pass after re-dedupe)
- Concrete change: in `scripts/qualitygate.nu` the root-manifest branch (~lines 203-207) degrades the tester to `cargo check --workspace` only when root `Cargo.toml`/`Cargo.lock` change, skipping the nextest tier (~lines 174-175). Keep the workspace check but run the touched-crate nextest tier first — a workspace dep bump must not disable test execution. Run 01M2WZHS added `bollard` to the root manifest, so the 12-test `fork_seam_test.rs` suite (523 lines) was never executed before PR #280 auto-merged. Expected effect: latent test failures surface in-run instead of after auto-merge. Distinct from open fabro-2f70 (verify.nu test detection) and open fabro-4815 (gate exit-status classification).

### Teach implementers the fork two-layer rule and mechanize it in verify.nu
- overflow-to-journal (no balance credit this pass; re-file next pass after re-dedupe)
- Concrete change: add the two-layer checklist bullet (fork-owned engine file change ⇒ matching row in `.agents/skills/merge-upstream/references/touchpoints.md`, rule 00ffd60f6) to `.fabro/workflows/develop/prompts/implementer.md`, and per the landed fabro-9ec3 policy make it mechanical in `scripts/verify.nu`: when the diff adds fork-owned engine files, require the matching touchpoints row before PASS. Run 01M2WZHS's only review bounce was exactly this missing row; the rule exists only in `prompts/reviewer.md` and the bounce cost a full planner@2→reviewer@2 re-cycle (~2.6 min, ~$0.14). Expected effect: fork-feature seeds ship both layers first pass; this bounce class disappears. Complements open fabro-b659 (minor-changes edge).

### Bake libssl-dev into the run toolchain image
- filed as fabro-95e1 (needs-user, capability-affecting, exempt from balance per ADR-0019/ADR-0022)
- Concrete change: add `libssl-dev` to `.fabro/Dockerfile.toolchain`'s apt line (~line 45), or unify reqwest to rustls-only. Expected effect: removes the guaranteed openssl-sys build dead-end diagnosis from every Rust-touching run.
