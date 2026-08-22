#!/usr/bin/env nu
# Bridge for the develop workflow's deterministic tester step.
# Runs the project's canonical gate target so the workflow stays
# project-agnostic: every project defines its own `qualitygate` in just.

just qualitygate
