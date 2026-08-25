# Project commands. Recipes are thin launchers for nu scripts in scripts/ —
# no inline shell logic lives here, so everything stays testable.

# Bootstrap workspace tooling: npm CLIs via bun plus toolchain checks.
# Called by the [run.prepare] step after `mise install`.
bootstrap:
    nu scripts/bootstrap.nu

# Deterministic quality gate: format, vet, build, test. Exit 0 = green.
# Called by the develop workflow's tester step.
qualitygate:
    nu scripts/qualitygate.nu

# Run a workflow end to end: create+start+attach, wait, integrate the run
# branch (ff-pull when auto-merge landed, else provisional squash-merge),
# then an Ask-Fabro improve review saved to .fabro/reviews/<run-id>.md,
# committed and pushed. Thin wrapper — logic lives in scripts/run_workflow.nu.
# Examples:
#   just run develop --goal "Implement product seed fabro-f74b ..."
#   just run develop --adopt 01M0WW
run *args:
    nu scripts/run_workflow.nu {{ args }}
