# Session handoff — BUILD FIRST: the A+B window (2026-11-09)

Production has been dark since 2026-11-08 22:50 BY DECISION (user,
2026-11-09): restore only through the new path, no deadline —
"wir haben keinen Zeitdruck, alles ist experimentell". The four
automations stay paused; no line work is lost while dark. Decisions
and rationale: ADR-0041 (host release path) and ADR-0042 (run-built
environment images). Delete this handoff when the window completes
(ADR-0036).

## The build (A + B, in any useful order)

1. **B — host release recipe (ADR-0041)**, in the fabro repo beside
   `scripts/image-release.nu`: anonymous HTTPS clone of the public
   repo on the host at a pinned sha; `cargo dev docker-build` inside
   containers against the HOST daemon; local tag
   `<version>-<sha12>`. No deploy key, no registry in the path.
2. **B — tofu change (fabro-tofu repo)**: `docker_image.fabro` names
   the local sha12 tag (source pin replaces the digest pin);
   optional ghcr mirror push AFTER a successful deploy (default on,
   `docker push`, never crane). The ghcr-auth module leaves the
   critical path.
3. **A — driver build for run environments (ADR-0042)**, in the
   vendored petri fork (`vendor/petri`), mirrored from the existing
   GH-actions `ContainerImage::Build`: `image.dockerfile` inline OR
   path with the prepared checkout as context; build at scope
   acquisition, content-hash tag, layer cache, Snapshot* progress;
   docker provider only. Reverse the admission strip (01713c6aa);
   update CONTRACT/HANDOFF/FORMAT ("the runner builds no image" →
   "the driver builds at scope acquisition"); export through the
   ADR-0034 vendor flow (`just vendor export petri --message …`);
   readiness suites green before the deploy.
4. **Toolchain migration (ADR-0042)**: rewrite
   `.fabro/Dockerfile.toolchain` multi-stage from the raw repo
   context — `chef prepare`/`cook` inside the image, fabro CLI built
   from the checkout, NO version pin. Declare the environment
   repo-side (`[environments.toolchain]` with `image.dockerfile`);
   retire `pin-toolchain`, the parity gate, the server-managed
   toolchain env, and the ghcr toolchain path (run-images.nu stops
   pushing toolchain).
5. **Channel-handshake compatibility floor (ADR-0042 part 4)**: the
   client attach site (`fabro-http` `with_run_channel_handshake`)
   probes the server's reported API version at connect and refuses
   loudly below the client's compile-time floor. FLOOR, never
   equality — candidate runs must pass. Bump the floor only when
   deliberately breaking the channel API.

## Before the live apply

- REHEARSE on the free host daemon (production is dark — the daemon
  is a free test bed): full recipe run, toolchain build, dry apply.
- The known-good fallback stays available the whole time:
  `0.362.0-fork.32-575a61e18554` is clean on ghcr. If the build
  stalls badly, the recovery steps from the 2026-11-08 handoff (fix
  host docker auth → pull → apply → unpause) still work verbatim.

## Cleanup already done (2026-11-09)

- The nohup'd toolchain-tar rsync is dead (log gone, stalled at 45%,
  00:48). The transfer plan (rsync → crane push → second apply on
  b5a5d00a26dd → pin) is OBSOLETE under ADR-0042 — the partial
  /tmp/toolchain-img.tar was removed from the host.

## After the restart (unchanged obligations from the old window)

1. Health: `curl https://mirtuell.net/health` → `{"status":"ok"}`.
2. Unpause all four automations: develop-fabro, merger-fabro,
   adjudicator-fabro, watchdog-fabro.
3. File the outage retroactively in the tracker (emergency clause) —
   the tracker was down with the server.
4. Live proofs riding the restart: first develop fire must gate GREEN
   on the run-built toolchain (nu 0.116.1 + regen; closes
   #1177/#1183); watchdog pass title verifies the titling chain
   (d52bff59f/#513).
5. Retry/fork #1186's stranded run (the livelock fix lives in its
   checkpoint); the #1173 re-fork is OBSOLETE (landed as #515).
6. Check off the todo items this window satisfies; then DELETE this
   handoff and the .todo directory with todo.md once empty (ADR-0036,
   mulch mx-c3d4fc — the md files are a dark-window transport
   workaround only; daily work lives in the tracker and mulch).
