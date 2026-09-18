#!/usr/bin/env bash
# Self-test for scripts/check-engine-freeze.sh against a synthetic repository:
# an added line under a frozen path exits 1, and deletions or additions
# elsewhere exit 0.
set -euo pipefail

CHECK="$(cd "$(dirname "$0")" && pwd)/check-engine-freeze.sh"
SRC="lib/components/fabro-workflow/src"

export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.com
export GIT_COMMITTER_NAME=test GIT_COMMITTER_EMAIL=test@example.com

repo="$(mktemp -d)"
trap 'rm -rf "$repo"' EXIT
cd "$repo"
git init -q -b main
mkdir -p "$SRC/handler" "$SRC/pipeline/execute" "$SRC/transforms"
printf 'a\nb\nc\n' > "$SRC/handler/agent.rs"
printf 'a\nb\n' > "$SRC/pipeline/execute/tests.rs"
printf 'a\n' > "$SRC/retry.rs"
printf 'a\n' > "$SRC/transforms/preamble.rs"
printf 'a\n' > "$SRC/pipeline/finalize.rs"
git add -A
git commit -q -m base

failures=0
expect() {
  local name="$1" want="$2"
  git checkout -q -b "$name" main
  "case_$name"
  git add -A
  git commit -q --allow-empty -m "$name"
  local got=0
  "$CHECK" main > /dev/null || got=$?
  if [ "$got" -eq "$want" ]; then
    echo "ok   $name (exit $got)"
  else
    echo "FAIL $name: expected exit $want, got $got"
    failures=$((failures + 1))
  fi
  git checkout -q main
}

case_adds_to_frozen_file() { echo d >> "$SRC/handler/agent.rs"; }
case_adds_to_frozen_dir() { echo c >> "$SRC/pipeline/execute/tests.rs"; }
case_rewrites_frozen_line() { printf 'a\nB\nc\n' > "$SRC/handler/agent.rs"; }
case_deletes_from_frozen_file() { printf 'a\n' > "$SRC/handler/agent.rs"; }
case_adds_outside_freeze() {
  echo b >> "$SRC/transforms/preamble.rs"
  echo b >> "$SRC/pipeline/finalize.rs"
}
case_no_change() { :; }

expect adds_to_frozen_file 1
expect adds_to_frozen_dir 1
expect rewrites_frozen_line 1
expect deletes_from_frozen_file 0
expect adds_outside_freeze 0
expect no_change 0

if [ "$failures" -ne 0 ]; then
  echo "$failures case(s) failed"
  exit 1
fi
echo "all cases passed"
