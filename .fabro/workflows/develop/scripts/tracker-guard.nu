#!/usr/bin/env nu
# Deterministic tracker guard (fabro-0da8): sits AFTER start and BEFORE
# the planner (ahead of the preflight too — a drained tracker makes the
# already-landed check moot), proving tracker state in ~1-3 s of shell
# so the drained-tracker terminal case costs ZERO planner tokens.
#
# Basis (run 01M0SFEYVC9TD6MP816RHEBFQY): planner@1 spent 27.7 s wall /
# 25.4 s inference / $0.0186 solely to discover 0 open seeds — what two
# `sd list` calls prove mechanically.
#
# Ownership rule (PROJECT_FACTS): BOTH calls filter `--assignee fabro
# --limit 200` — plain `sd list` would count unassigned/user-owned seeds
# this line must never work. The empty-tracker exit fires only when no
# FABRO-ASSIGNED seed remains (open or in_progress). `sd list` default
# output is OPEN issues only, which includes blocked-open seeds (per the
# seed body; `sd ready` alone lists unblocked only and would misroute,
# fabro-aa3d) — hence open via the default listing, in_progress via
# `--status in_progress`.
#
# Routes (output_schema="routing" on the node):
#   - BOTH lists empty -> preferred_next_label "Tracker empty" — the
#     existing label contract already declared on the planner's exit
#     edge, wired here to the plain exit edge (natural completion: the
#     goal is achieved, NOT an error; same family as the planner's own
#     "Tracker empty" edge).
#   - either non-empty -> preferred_next_label "Tracker non-empty"
#     (unconditional edge to the preflight, then the planner unchanged).
#     This node produces NO context value — none is required downstream.
#
# FAIL-OPEN (annotated choice): if `sd` errors, exits non-zero, or
# prints non-JSON output, the node routes "Tracker non-empty" (degraded
# mode) — it NEVER parks and NEVER fails the run: a broken tracker must
# not dead-end the dev loop, and the planner adjudicates the residue.
# Mirrors planner-preflight.nu's fail-open contract.

# Count issues in one `complete`-style sd result; -1 marks failure
# (non-zero exit, invalid JSON, or success:false) for the fail-open path.
def sd-issue-count [res: record] {
    if $res.exit_code != 0 { return (-1) }
    # nu's `from json` does NOT error on invalid input — it echoes the
    # raw string back — so the failure check is a type test, not try/catch.
    let parsed = (try { $res.stdout | from json } catch { null })
    if not ($parsed | describe | str starts-with "record") { return (-1) }
    if ($parsed.success? | default true) == false { return (-1) }
    ($parsed.issues? | default [] | length)
}

# Pure decision over the two sd results (smoke-testable without
# shelling): {exit_code, stdout} records -> routing record.
def guard-decision [open_res: record, inprog_res: record] {
    let open_n = (sd-issue-count $open_res)
    let inprog_n = (sd-issue-count $inprog_res)
    if $open_n < 0 or $inprog_n < 0 {
        # Fail-open: degraded mode routes the preflight/planner normally.
        {"outcome": "succeeded", "preferred_next_label": "Tracker non-empty"}
    } else if $open_n == 0 and $inprog_n == 0 {
        {"outcome": "succeeded", "preferred_next_label": "Tracker empty"}
    } else {
        {"outcome": "succeeded", "preferred_next_label": "Tracker non-empty"}
    }
}

def main []: nothing -> nothing {
    let open_res = (do { sd list --format json --assignee fabro --limit 200 } | complete)
    let inprog_res = (do { sd list --format json --status in_progress --assignee fabro --limit 200 } | complete)
    (guard-decision $open_res $inprog_res) | to json --raw | print
}
