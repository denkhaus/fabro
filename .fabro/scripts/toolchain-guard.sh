#!/bin/sh
# Toolchain-placement guard (fabro-3ab2), shared across loop-owned lanes:
# ONE file plus a lane flag (the tracker-guard/closeout/evidence sharing
# pattern). POSIX sh BY NECESSITY: the failure mode it teaches is a
# sandbox WITHOUT nu — a nu guard could never run there, the run would
# die cryptically at "nu: command not found" instead.
#
# A manual `fabro run <workflow>` without --environment falls to the
# server's `default` environment: the CLI always sends an explicit
# intent environment id, and the intent's selection outranks any
# [run.environment] pin in workflow.toml, so the file cannot redirect a
# flag-less manual fire. This guard converts the wasted run into a
# fail-closed teaching error that names the correct fire command. On the
# toolchain environment it passes in milliseconds and changes nothing.
#
# Usage: sh .fabro/scripts/toolchain-guard.sh <lane>
#   lane  loop | develop | revisor — names the lane's fire command in
#         the teaching error. Unknown lanes get the generic form.

set -u

lane="${1:-loop}"

# The toolchain image bakes these (Dockerfile.toolchain); the loop/develop
# first stages call all three directly.
for tool in nu just seeds; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        missing="$tool"
        break
    fi
done

if [ "${missing:-}" = "" ]; then
    exit 0
fi

case "$lane" in
    loop)    refire="just run loop   (or: fabro run loop --environment toolchain)" ;;
    develop) refire="just run develop (or: fabro run develop --environment toolchain)" ;;
    revisor) refire="just run revisor (or: fabro run revisor --environment toolchain)" ;;
    *)       refire="just run <$lane> (or: fabro run <$lane> --environment toolchain)" ;;
esac

cat >&2 <<EOF
toolchain-guard: FAIL — '$missing' is not on PATH; this workflow's stages need the toolchain environment (nu, just, seeds).
The run was placed on the wrong sandbox image: a manual fire without --environment selects the server's 'default' environment, and the intent's environment selection outranks any workflow.toml pin.
Re-fire with: $refire
EOF
exit 1
