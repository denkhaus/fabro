#!/usr/bin/env bash
# Report added lines under the frozen engine half of fabro-workflow.
#
# The engine half of `lib/components/fabro-workflow` takes bug fixes only;
# new engine behaviour goes to Petri (`lib/components/fabro-petri`). This
# script lists every frozen file the current branch adds lines to, compared
# with the base ref, and exits 1 when there is at least one. It knows nothing
# about pull request labels: the CI job (`.github/workflows/engine-freeze.yml`)
# waives a failure when the pull request carries the `bugfix` label.
#
# Usage: scripts/check-engine-freeze.sh [<base-ref>]   (default: origin/main)
set -euo pipefail

BASE_REF="${1:-origin/main}"
LABEL="bugfix"

# The frozen paths, relative to the fabro-workflow crate's `src/`. A directory
# freezes everything under it. `preamble` in the integration plan is
# `handler/llm/preamble.rs`, which `handler/` covers; `transforms/preamble.rs`
# is a graph transform and is not frozen.
FROZEN=(
  handler
  lifecycle
  pipeline/execute.rs
  pipeline/execute
  graph/routing.rs
  node_handler.rs
  retry.rs
  condition.rs
  context.rs
  model_fallback.rs
)

if [ "${FREEZE_LIST_ONLY:-}" = "1" ]; then
  printf '%s\n' "${FROZEN[@]}"
  exit 0
fi

ROOT="$(git rev-parse --show-toplevel)"
CRATE_SRC="lib/components/fabro-workflow/src"

paths=()
for entry in "${FROZEN[@]}"; do
  paths+=("$CRATE_SRC/$entry")
done

# Three dots: the changes since the merge base, which is what a pull request
# adds to its base branch.
numstat="$(git -C "$ROOT" diff --numstat "$BASE_REF...HEAD" -- "${paths[@]}")"

offenders=()
while IFS=$'\t' read -r added _deleted path; do
  [ -n "${path:-}" ] || continue
  case "$added" in
    ''|*[!0-9]*) continue ;;  # binary files report '-'
  esac
  if [ "$added" -gt 0 ]; then
    offenders+=("$added	$path")
  fi
done <<< "$numstat"

if [ "${#offenders[@]}" -eq 0 ]; then
  echo "engine freeze: no lines added under the frozen paths of fabro-workflow since $BASE_REF"
  exit 0
fi

echo "engine freeze: this branch adds lines to the frozen engine half of fabro-workflow (since $BASE_REF):"
for line in "${offenders[@]}"; do
  echo "  +${line%%	*} ${line#*	}"
done
echo
echo "The engine half of lib/components/fabro-workflow takes bug fixes only."
echo "New engine behaviour goes to Petri (lib/components/fabro-petri)."
echo "Frozen paths under $CRATE_SRC/:"
for entry in "${FROZEN[@]}"; do
  echo "  $entry"
done
echo
echo "A bug fix passes CI when the pull request carries the '$LABEL' label."
exit 1
