# Session handoff — BUILD FIRST: the A+B window (2026-11-09, code complete)

Production has been dark since 2026-11-08 22:50 BY DECISION (user,
2026-11-09): restore only through the new path, no deadline. The four
automations stay paused. Decisions: ADR-0041 (host release path),
ADR-0042 (run-built environment images). This handoff carries the
OPERATIONAL rest; delete it when the window completes (ADR-0036,
mulch mx-c3d4fc).

## Code status: COMPLETE AND PROVEN (all pushed)

- B: `just host-release <host> <port>` builds the server image on the
  host from an anonymous public clone; tofu consumes the local sha12
  tag; the ghcr mirror is post-deploy, health-gated, default on —
  credential MINTED per apply (fabro-tofu scripts/lib/
  app-installation-token.nu, fabro-the-app packages:write, 1 h life).
- A: image.dockerfile is live end to end — petri parse, lowering to
  ContainerBuild, the executor's holder-based build at scope
  acquisition (E2E-proven on the dev daemon), contract docs, the
  fabro un-strip (build wins over a finished image, layer override).
- Handshake floor: RUN_CHANNEL_MIN_SERVER in fabro-client (0.0 lands).
- Toolchain migration: Dockerfile.toolchain multi-stage FROM THE RAW
  REPOSITORY, PROVEN BY A FULL BUILD (BUILD-EXIT=0, 3.18 GB; smoke:
  fabro 0.362.0-fork.32 built inside, nu 0.116.1, full toolset,
  validate fast path). [environments.toolchain] repo-declared in all
  four line workflows; parity fully retired (pin-toolchain, wrapper,
  run-images.nu, ghcr toolchain path). Probe lessons baked in: the
  engine pack skips a repo's .dockerignore (fork 58481cde79f1); the
  final stage embeds docs/{agents,internal,adr,lab} and needs
  WORKDIR /src (include_str!/include_dir! resolve against the
  workspace-root-relative shape cargo passes rustc).

## The operational rest (this session's successors)

1. REHEARSE on the free host daemon (production is dark — free test
   bed): a full `just host-release <host> <port>` run; optionally a
   toolchain build from the host's clone (the first cook is the long
   pole, 10-30 min; the layer cache pays it once).
2. LIVE APPLY (fabro-tofu): `just tofu apply` with
   `-var fabro_image=denkhaus/fabro -var "fabro_image_ref=:<version>-<sha12>"`
   naming the host-built tag; the mirror pushes after health.
3. Restart list:
   a. Health: `curl https://mirtuell.net/health` → `{"status":"ok"}`.
   b. Unpause all four automations (develop, merger, adjudicator,
      watchdog).
   c. File the outage retroactively in the tracker (emergency clause).
   d. Hygiene: delete the inert server-managed `toolchain`
      environment entry (the repo-declared build wins by layer
      override anyway).
   e. FIX BEFORE THE FIRST DEVELOP FIRE: cmd::orchestration
      observation_list_json is RED on clean HEAD (fork.32 observation
      batch: the mock payload lacks `linked_work_items`) — the tester
      gate would fail every run.
   f. Live proofs: the first develop fire builds the toolchain image
      at run init on the host daemon (first build pays the cook) and
      must gate GREEN (nu 0.116.1 + regen; closes #1177/#1183);
      watchdog pass title verifies the titling chain (d52bff59f/#513).
   g. Retry/fork #1186's stranded run; #1173 re-fork is OBSOLETE.
4. Fallback at any point: `0.362.0-fork.32-575a61e18554` is clean on
   ghcr; the legacy recovery (fix host docker auth → pull → apply →
   unpause) works verbatim.

Dev-machine leftovers (harmless, /tmp/opencode): tc-ctx clone,
probe-target, fabro-toolchain:repo-probe image, dbg/scratch images.

When all of the above is done: check off the todo items this window
satisfied, then DELETE this handoff and the .todo directory once
empty (ADR-0036, mulch mx-c3d4fc).
