# ADR-0042: Run-built environment images; the toolchain parity principle retires

- Status: Accepted (2026-11-09, same design session as ADR-0041)
- Deciders: user (all four shape decisions and the product framing,
  2026-11-09), agent (facts, drafting)
- Relates to: ADR-0035 (the run channel — the seam this ADR stops
  freezing and starts checking), ADR-0034 (vendored fork crates — the
  petri contract change rides the export flow), ADR-0041 (the host
  release path — the server image becomes the only deploy artifact),
  the 2026-09-16 release-pipeline decision that introduced the parity
  rule (revised here)

## Context

Fabro once auto-built sandbox images: `image.dockerfile` on the docker
provider built `fabro-runner-<sha12>` at sandbox initialization, inline
dockerfile, empty context, content-hash cache (fabro-969f, 2026-08-23,
built to kill a measured 24.5 s `mise install` in every ~32 s run). The
sandbox-driver cutover deleted it for architectural reasons, not as a
product verdict (3d33935fb, 2026-09-09; `fabro-sandbox` deleted
f054082f8, 2026-09-19). The petri contract now pins the opposite: "the
runner builds no image" — admission strips `image.dockerfile` before
the engine sees it (01713c6aa).

The toolchain environment became a server-managed special case: built
at release time on the dev machine, pushed to ghcr, pinned server-side
by `pin-toolchain` behind a fail-closed parity gate (the tag's sha12
must equal the deployed server's build sha; 2026-09-16 rule). The
2026-11-08 window exposed its cost: the gate compares shas, not
function, so a server tag (575a61e18) and a toolchain tag (b5a5d00a2)
from different release runs — functionally compatible — could only be
reconciled by re-pushing tags and a second apply; and the 2.1 GB
transfer per release exists to serve the pin.

The product argument (user, 2026-11-09): fabro is not an end in itself.
Other repositories with their own toolchains must run workflows on
fabro; the line dogfooding its own deploy pipeline cannot be the model.
A parity rule that only a server-managed environment can satisfy does
not generalize.

The seam the parity rule tried to freeze is real: a deterministic stage
calling `fabro work` runs the sandbox CLI (checkout version) against
the deployed server's router over the per-run channel socket
(ADR-0035). Freezing both sides to one sha never tested that seam — a
candidate run's client-server compatibility was invisible by
construction.

## Decision

Four parts.

1. **The driver builds environment images at scope initialization.**
   sandbox-driver gains Build for run environments, mirroring the
   existing GitHub-actions `ContainerImage::Build`: an environment
   declares `image.dockerfile` — inline, or a path whose build context
   is the run's prepared checkout. The engine builds before the sandbox
   exists, under a content-hash tag, warmed by the daemon's BuildKit
   layer cache, reporting the build as scope progress (the
   SnapshotCreating/Ready/Failed shape). Docker provider only in v1;
   remote providers stay honestly without. The admission strip is
   reversed: `image.dockerfile` becomes a live platform key again. The
   petri contract change (CONTRACT, HANDOFF, FORMAT: "the runner builds
   no image" becomes "the driver builds at scope acquisition") lands in
   the fork and reaches fabro through the ADR-0034 export flow.

2. **The line's toolchain environment becomes repo-declared.**
   `Dockerfile.toolchain` is rewritten multi-stage from the raw
   repository context — `chef prepare`/`cook` run inside the image, and
   the fabro CLI is built inside the image from the checkout. No
   version install, no pin: the checkout IS the version, so image and
   code under test are the same sha by construction.

3. **Parity retires completely.** The `pin-toolchain` command, the
   parity gate, the server-managed toolchain environment, and the ghcr
   toolchain path die. The toolchain image stops being a release or
   deploy artifact: no per-release churn, no transfer, no mirror.

4. **The seam is checked, not frozen — a compatibility floor in the
   channel handshake.** The client's channel attach site
   (`with_run_channel_handshake`, the one place every Unix-socket
   client builder passes) probes the serving server's reported API
   version at connect and compares it against the client's compile-time
   compatible floor. Below the floor the client refuses loudly — a
   clear version error naming both versions, not a 404 trail. The
   predicate is a FLOOR, never equality: a candidate run's CLI must be
   able to talk to the deployed server; only a server older than the
   client's floor fails. Bumping the floor is part of deliberately
   breaking a channel-API change.

## Alternatives rejected

- **Parity by construction** (the release recipe builds server and
  toolchain at the same sha): keeps the dogfood special case alive and
  never tests the seam; rejected on the product argument.
- **A fabro-shim** (a platform-injected, versionless server client in
  the sandbox): a second transport beside the run channel, which
  already carries the token/principal boundary; solves nothing the
  handshake check does not.
- **A machine-readable `fabro version` verdict that scripts evaluate
  (user's first proposal, 2026-11-09):** opt-in, script-side, easy to
  forget; the handshake is automatic at the one attach site. The user
  chose the handshake over it in the same session.

## Consequences

- A run at a candidate sha exercises the real client-server seam; a
  server rollback turns incompatibility into a clear stage failure.
- An unbumped breaking change still surfaces as transport errors — the
  floor is a discipline, not a proof. Acceptable for this line.
- Deterministic stages need no version preflight; the refusal happens
  on first channel use.
- The `fabro validate` fast path is unaffected (local create-check, no
  server contact).
- The first build after a dependency-graph change pays the cook on the
  host daemon; later builds ride the content-hash and layer caches.
- Petri's contract text, HANDOFF and FORMAT change in the fork; the
  fabro-side environment plumbing (path resolution in the run compiler,
  bundling as a dependency) already exists unused since the cutover.
