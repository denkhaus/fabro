# Nu scripts follow the main + helpers convention

Every nu script in either world (workflow scripts under
`.fabro/workflows/*/scripts/`, root scripts under `scripts/`) is structured
the same way:

- `def main` is the only entry point and auto-executes when the script
  runs; `main` stays linear — wiring only, no inline logic.
- Logic lives in private helpers (`def`, kebab-case) with typed I/O
  signatures (`nothing -> T`, pipeline input declared in the signature,
  not as a positional).
- Quality-gate checks are `nothing -> bool`: each prints its own section
  and returns false with details on failure; `main` stops at the first
  red check and exits non-zero.
- External commands: arguments as separate values (never interpolated
  command strings), status via `complete` + `exit_code`; chatter from
  side-effect commands (e.g. `git fetch`) is captured and discarded so it
  cannot pollute gate logs. Internal commands that can fail in-process
  (e.g. `nu-check`) use `try/catch` yielding a bool.
- `match` for multi-branch on one value, early `return` for skip paths,
  constants in SCREAMING_SNAKE_CASE.
- When a script's output is a contract other assets reference (evidence
  section titles), structural changes must be verified behavior-preserving
  (diff the output against the old script on a real run branch).

Enforcement is mechanical: both worlds' `just qualitygate` run `nu-check`
(parse check) over every nu script in their tree — workflow scripts AND
root scripts. A script that fails to parse fails the gate in that world.
Convention source: nushell-pro skill (modules-and-scripts, anti-patterns).
