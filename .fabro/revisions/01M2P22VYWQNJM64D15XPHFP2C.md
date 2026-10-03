# Revision — run 01M2P22VYWQNJM64D15XPHFP2C

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2P22VYWQNJM64D15XPHFP2C.md
- seeds filed: fabro-ea41 — Bake cargo-insta into the run toolchain image so snapshot accepts are mechanical (capability-affecting, needs-user); fabro-7aac — Closeout: re-file deferred actions disclosed in the implementation summary, not only brief EXEMPTION arms
- basis: run 01M2P22VYWQNJM64D15XPHFP2C, workflow version 1e95a3bfe93d5d70d9b6aa6a844359b97607388445120dab1286674bd42e4a86, commit 6ce08e99326797d9b54814bac574823c770e1020
- revised_at_commit: 6ce08e99326797d9b54814bac574823c770e1020 (ADR-0015: engine drift signal for later judgement)

## Findings

### Bake cargo-insta into the run toolchain image — filed fabro-ea41

Basis: run 01M2P22VYWQNJM64D15XPHFP2C (seed fabro-3916), implementer@2 painpoint — `cargo insta` absent from the sandbox forced hand-fixing two drifted inline `--help` snapshots from the raw nextest diff. Change: add cargo-insta to `.fabro/Dockerfile.toolchain` (same bake avenue open `fabro-3b1b` notes for java). Expected effect: snapshot drift fixes become one accept command; removes a hand-edit correctness-risk class. Capability-affecting (ADR-0019): adds a tool to an agent-reachable surface — filed with `needs-user,revision`, implementation awaits explicit user approval; fix direction is engine-mediated image bake only, no raw clients/tokens.

### Closeout: file deferred implementation-summary actions — filed fabro-7aac

Basis: run 01M2P22VYWQNJM64D15XPHFP2C closed `fabro-3916` with a human follow-up (re-run `bun run generate` on a Java-capable host) living only in `implementation_summary`/journal; closeout filed nothing. Change: extend the `fabro-534e` mechanism in `.fabro/workflows/develop/scripts/closeout.nu` so deferred/regen-confirm actions disclosed in the implementation summary are filed as open seeds before `sd close`. Complementary to `fabro-534e` (different trigger source, same failure mode) — cross-referenced, nothing closed. Expected effect: tracker-visible follow-ups on every run closing with disclosed deferred work.
