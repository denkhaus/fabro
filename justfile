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
