#!/usr/bin/env nu
# Bootstrap the workspace toolchain beyond what mise provides.
# Called by `just bootstrap` from the [run.prepare] step (after `mise install`).
#
# npm-distributed CLIs land via `bun install -g` because mise's aube/npm
# backend demands interactive confirmation; the runner image pins BUN_INSTALL
# to a directory already on PATH, so the shims are visible to every stage.

bun install -g @os-eco/seeds-cli@0.5.15 @os-eco/mulch-cli@0.10.7

# Sanity: every tool the develop workflow relies on must resolve.
# A failing external command aborts the script with a non-zero exit.
sd --version
ml --version
go version
just --version
nu --version

print "bootstrap ok"
