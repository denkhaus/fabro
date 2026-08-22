#!/usr/bin/env nu
# Bootstrap the platform toolchain beyond what mise provides.
# Called by `just bootstrap` from the [run.prepare] step (after `mise install`).

bun install -g @os-eco/seeds-cli@0.5.15 @os-eco/mulch-cli@0.10.7

# Sanity: every tool the platform world relies on must resolve.
sd --version
ml --version
nu --version
just --version
fabro --version
uv --version

print "bootstrap ok"
