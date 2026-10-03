# ADR-0023: Native seeds & mulch — fabro-owned Rust implementations in their own public repos

- Status: Accepted
- Date: 2026-09-26
- Deciders: user (grilling rounds 1-4), agent (analysis + live probes)
- Related: ADR-0012 (dogfooding), ADR-0013 (upstream posture), ADR-0017 (tool-agnostic engine), ADR-0022 (judgment layer)

## Context

The dev loop tracks work in seeds (`sd`, `@os-eco/seeds-cli` 0.5.15) and
expertise in mulch (`ml`, `@os-eco/mulch-cli` 0.10.7): Node CLIs installed via
mise (bun-backend workaround for non-TTY aborts), pinned in three places
(`.mise.toml`, `.fabro/Dockerfile.toolchain`, AGENTS.md onboard markers).
No acute failures exist (14-day rootprint audit plus journal scan: zero real
incidents), but the friction is structural: install chore, CLI-text seams
wrapped in fail-open code at ~10 call sites, and feature sovereignty sits
with an upstream maintainer whose releases are quiet (mulch since 2026-06-02,
seeds since 2026-07-17 — both verified as npm `latest`, our pins are current).

Both stores are trivial, fully characterized formats: `.seeds/` is three
JSONL files, `.mulch/expertise/` one JSONL per domain; seed ids are
`<project>-<hex4>` (project from `config.yaml`), dependencies are `blockedBy`
arrays on the record. Live probe 2026-09-26: `sd` 0.5.15 does
read-modify-write and **preserves unknown record fields** through `update`
and `close` — additive fields survive mixed operation with the old CLI.

## Decision

1. **Rebuild both tools in Rust as fabro-owned products in their own public
   repos**: `denkhaus/seeds` first, `denkhaus/mulch` later, cloned from the
   seeds pattern. Crates and binaries are named `seeds` / `mulch` (both free
   on crates.io; `sd` is not — it collides with the sed-style `sd`). Each
   README states the format-compatibility promise and attributes
   jayminwest/seeds and jayminwest/mulch. These are the first non-fabro
   projects developed autonomously by fabro lines: the platform-proof
   milestone beyond ADR-0012's self-dogfooding.
2. **Format compatibility contract**: read+write drop-in against
   seeds-cli@0.5.15 / mulch-cli@0.10.7. Additive fields are the only
   extension mechanism. Round-trip tests (our writer writes, the reference
   CLI reads and updates, our fields survive) are the crate's acceptance
   gate.
3. **Freeze policy**: no upstream following. The format is frozen on our
   side; we extend additively for our own reasons. The round-trip suite
   stays runnable against future upstream releases as an alarm, never as an
   obligation.
4. **fabro integration**: compile-in via a pinned git dependency on the
   public seeds repo (the pebble-coding-agent precedent) plus
   `fabro seeds` / `fabro mulch` subcommands as the primary surface —
   installing fabro installs the tracker. Cross-repo coordination has no
   tracker edges: seeds-repo release tags are the sync points, and "bump
   the pin" is an explicit fabro seed per release. Public repos avoid the
   git-credential problem for sandbox builds.
5. **Server/UI v1 is read-only**: the server reads its host checkout (line
   branch, refreshed via fetch); OpenAPI-first endpoints serve seed
   list/show/dependency graph; the fabro-web seed browser (list, detail,
   dep graph) renders on the existing viz-js hook with `prepareSvg`
   click-through routing. Write endpoints wait until after cutover proof.
6. **Bootstrap doctrine**: the seeds repo tracks its own development in
   seeds format from day one, starting on `sd` 0.5.15; the first
   self-hosting cutover (the product replacing `sd` in its own repo) is
   the parity proof, delivered *before* any fabro integration. Mulch's
   canonical-domain layer — fixing this repo's 31-domain fragmentation
   (develop/dev-loop/develop-loop/develop-workflow/devloop/loop/loop-assets)
   via alias resolution — moves to the mulch phase.
7. **fabro-repo cutover**: retire `sd`/`ml` (mise, Dockerfile.toolchain,
   AGENTS.md markers, docs/agents/issue-tracker.md, script seams) after 10
   consecutive clean develop cycles on the new CLI, including at least one
   full revisor pass and one closeout; the seam (command-switch variable)
   decays one release after that. In-sandbox call cost of `fabro seeds`
   must match today's PATH-shim cost of `sd` — an acceptance criterion of
   the subcommand work, not an afterthought.
8. **Posture**: the new repos are public from day one; fabro itself keeps
   its ADR-0013 posture, and fabro-internal surfaces stay internal-first
   per ADR-0022. Upstream ecosystem effect of publishing compatible
   rewrites is accepted.

## Consequences

- We own two more products: their bugs, schemas, release cadence, and the
  compat promise. Upstream improvements never reach us automatically.
- Every fabro workspace build fetches the seeds git dep (public repo, no
  sandbox credential work).
- The dev loop's fail-open `sd` wrappers stay until cutover; afterwards
  they collapse into typed subcommand calls.
- Native provenance (run_id / workflow_version_id as record fields instead
  of description text) and judgment-layer classification of records become
  possible only after the compile-in exists — sequenced after cutover.
- Cross-repo work splits trackers: seeds-repo product seeds live in the
  seeds repo's own tracker (created during bootstrap); only fabro-side
  integration seeds live here.
