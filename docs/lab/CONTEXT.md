# Lab platform context — glossary

Engine/workflow dogfooding vocabulary for the platform world
(`docs/lab/`, ADRs 0009+). Terms defined here are binding for seeds,
prompts, and ADRs; avoid the listed synonyms.

## Terms

- **revisable** — property of a develop run eligible for a revisor pass:
  terminal, live sandbox (`sandbox_available == true`), and fresh
  (workflow version matches, see *stale-evidence run*). Synonyms to
  avoid: "open", "pending", "eligible" (ambiguous with run status).
- **stale-evidence run** — a terminal run whose `workflow_version_id`
  differs from the current registered develop workflow version, or whose
  sandbox is gone. Stale-evidence runs are skipped by selection; they
  must not produce seeds. Synonyms to avoid: "outdated run", "old run".
- **seed basis** — the provenance line every revisor seed carries:
  source run id, `workflow_version_id`, repo commit. Consumed by the
  develop planner's stale-basis check; a seed whose basis no longer
  resolves against the current tree is closed as `superseded`
  (ADR-0014). Synonyms to avoid: "seed origin", "evidence pointer".
- **revision pass** — one select -> analyze -> file cycle within a
  revisor invocation. Bounded per invocation by `revisions_per_pass`
  (ADR-0015); one revisor run performs at most that many passes.
