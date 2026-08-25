# Conflict resolution policy (fabro fork merges)

Core rule: **both sides survive.** Upstream ships the platform direction;
our fork ships feature work that must not regress. Adapt OUR call sites to
upstream's new signatures; never revert upstream, never drop our features.

## Resolution procedure

1. Classify the conflict (see classes below). `git diff --name-only
   --diff-filter=U`, then read each hunk with both sides' context.
2. If both sides ADD tests / helpers / struct fields: keep both, adapt
   names if they collide.
3. If upstream CHANGED a signature we call: take upstream's signature,
   adapt our call sites (find the old-arg mapping in upstream's own
   callers: `git show upstream/main:<file> | grep -n "<fn>("`).
4. If upstream MOVED/RENAMED plumbing (e.g. origin-url derivation): take
   upstream's location, re-attach our additions at the new site.
5. Mechanical marker removal is DANGEROUS on add/add TEST conflicts:
   conflict blocks can swallow closing delimiters (`}`, `);`). After
   resolving, `cargo build` immediately — an "unclosed delimiter" error
   means a test body lost its tail; restore it from
   `git show HEAD:<file>` around the old marker position.

## Conflict classes seen (newest first)

- 2026-08-25 v0.336.0: `demote_large_values_for_prompt` gained
  `budget: usize` param + sandbox-env call shape. Our aggregate-budget
  tests + upstream runtime-directory tests: keep both, adapt our calls
  (git_integration.rs used the old default: pass the literal budget).
- 2026-08-25 v0.336.0: RunSession gained `workflow_path` field — thread
  the existing local `let workflow_path` into the initializer.
- 2026-08-25 v0.336.0: `pr_origin_url` derivation moved to
  `runtime_origin_url` (skip_clone-aware). Take upstream's; keep our
  `pr_content_model` resolution fields alongside.
- 2026-08-25 v0.336.0: `build_single_file_tar` gained `mode: u32`.
  Keep our helper functions; adopt the mode param (Dockerfile: 0o644).
