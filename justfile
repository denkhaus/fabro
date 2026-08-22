# Platform commands. Recipes are thin launchers for nu scripts — no inline
# shell logic, same convention as the product world.

# Bootstrap platform tooling (npm CLIs via bun, toolchain checks).
bootstrap:
    nu scripts/bootstrap.nu

# Platform quality gate: workflow lint. Exit 0 = green.
qualitygate:
    nu scripts/qualitygate-meta.nu
