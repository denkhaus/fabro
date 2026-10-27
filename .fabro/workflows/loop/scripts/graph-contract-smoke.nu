#!/usr/bin/env nu
# Loop graph-contract smoke (fabro-70b5): pins the loop lane's
# load-bearing graph contract — pure file assertions, no seeds, no git
# (develop graph-contract-smoke pattern, wired into scripts/qualitygate.nu's
# loop-asset tier).
#
#   nu .fabro/workflows/loop/scripts/graph-contract-smoke.nu
#
# Contract:
#   1. planner node carries NO goal_gate — the deterministic guards
#      legitimately bypass it (same fabro-92e2 class as develop).
#   2. NO preflight -> exit edge — report-only preflight, never a script
#      exit.
#   3. The tracker_guard node is GONE (fabro-786a): its .seeds
#      drained-queue read was a split-brain against the work-line claim;
#      the planner's "Tracker empty" -> exit edge carries the drained
#      case now, and no tracker_guard/claim_check node may return.
#   4. The preflight sits AFTER the planner (develop's topology):
#      planner -> preflight ("Seed claimed"), preflight -> implementer
#      ("Preflight done"), preflight -> planner re-plan on failure (the
#      fabro-c42f claim-contract retry).
#   5. planner -> exit ("Already landed") edge EXISTS.
#   6. Lane wiring: the preflight script line carries --work-item
#      --assignee loop with stdin_source=current_item_json (the
#      granted-item packet; --assignee drives the bd55 lane-scoped
#      envelope_risk); evidence carries --lane loop --work-item with
#      stdin_source=current_item_json; closeout is the denkhaus/work#990
#      script form — closeout.nu --work-item --lane loop with the
#      packet on stdin, the channel carrying every tracker call (the
#      lane label routes filed residuals back into THIS lane's queue).
#   6b. The planner forwards the granted-item packet: x.context_allow_keys
#      names current_item_json (the evidence and preflight stages read
#      it from stdin; the e47c allow-keys lint drops undeclared keys — a
#      missing entry silently starves both).
#   7. Implementer envelope: x.fs_write names EXACTLY the loop-asset
#      set (.fabro/**, scripts/**, justfile, .seeds/issues.jsonl) plus
#      the fabro-dot snapshot dir (fabro-9973 — the one lib/ exception,
#      mechanically derived from .fabro/workflows graphs) and fs_hide
#      hides .agents/** (the meta lane must SEE loop assets).
#   8. tester -> implementer ("Gate red") edge EXISTS — the loop lane has
#      no gatebounce node; the red bounce returns straight to the
#      implementer.
#   9. The tester runs the loop gate, NOT the product gate
#      (just qualitygate must not appear on the tester node).
#  10. Reviewer evidence line budget: x.preamble_output_max_lines on the
#      reviewer node stays >= 1000 (fabro-31f7) — the stage Output tail
#      cap must never silently omit an evidence capture's head.
# 11. Toolchain-placement guard (fabro-3ab2): the env_guard node is the
#      FIRST stage after start, runs the shared guard through POSIX sh
#      (never nu — the failure mode is a sandbox without nu), and hands
#      to tracker_guard. A dropped guard silently returns the cryptic
#      "nu: command not found" death on misplaced manual fires.
#  12. Spec-paths implementer self-check: the shared claim-check emits
#      the `claim-check: spec-paths` list into stage output and the
#      implementer prompt diffs it against the run diff before routing
#      Implemented — the spec-paths miss dies at the implementer,
#      never as a tester battery bounce.

const GRAPH = ('.fabro/workflows/loop/workflow.fabro' | path expand)
const FS_WRITE_EXPECTED = 'x.fs_write=".fabro/**,scripts/**,justfile,lib/components/fabro-dot/src/snapshots/**"'

def fail [what: string]: nothing -> nothing {
    print -e $"loop graph-contract-smoke: FAIL — ($what)"
    exit 1
}

def main [] {
    if not ($GRAPH | path exists) { fail $"graph not found: ($GRAPH)" }
    let text = (open --raw $GRAPH)
    let lines = ($text | lines)

    # 1. planner block must not carry goal_gate (comments stripped)
    let start = ($lines | enumerate | where {|e| ($e.item | str trim) == 'planner ['} | get -o index | first | default null)
    if $start == null { fail 'planner node not found' }
    let end = ($lines | enumerate | where {|e| ($e.index > $start) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $end == null { fail 'planner node block not terminated' }
    let block = ($lines | skip ($start + 1) | take ($end - $start - 1))
    if ($block | any {|l| ($l | split row '//' | first | str contains 'goal_gate')}) {
        fail 'planner node carries goal_gate: deterministic guard exits would die at the terminal gate check'
    }

    if ($lines | any {|l| ($l | str contains 'preflight -> exit')}) {
        fail 'preflight -> exit edge exists: report-only preflight must never exit the run'
    }
    # 3. the guards are GONE (fabro-786a): no tracker_guard or claim_check
    # node may return, and the planner carries the drained-queue exit
    if ($lines | any {|l| ($l | str trim | str starts-with 'tracker_guard [')}) {
        fail 'tracker_guard node resurrected — the .seeds split-brain guard is retired (fabro-786a); the planner routes the drained case'
    }
    if ($lines | any {|l| ($l | str trim | str starts-with 'claim_check [')}) {
        fail 'claim_check node resurrected — the claim contract lives in the preflight packet validation (fabro-786a)'
    }
    if not ($lines | any {|l| ($l | str contains 'planner -> exit') and ($l | str contains 'Tracker empty')}) {
        fail 'planner -> exit ("Tracker empty") edge missing — the drained-queue fast exit rides the planner now'
    }
    # 4. preflight after the planner (develop's topology) with the
    # c42f retry edge
    if not ($lines | any {|l| ($l | str contains 'planner -> preflight') and ($l | str contains 'Seed claimed')}) {
        fail 'planner -> preflight ("Seed claimed") edge missing'
    }
    if not ($lines | any {|l| ($l | str contains 'preflight -> implementer') and ($l | str contains 'Preflight done')}) {
        fail 'preflight -> implementer ("Preflight done") edge missing'
    }
    if not ($lines | any {|l| ($l | str contains 'preflight -> planner') and ($l | str contains 'Claim contract failed')}) {
        fail 'preflight -> planner re-plan edge missing — a dropped packet would dead-end instead of retrying'
    }
    if not ($lines | any {|l| ($l | str contains 'planner -> exit') and ($l | str contains 'Already landed')}) {
        fail 'planner -> exit ("Already landed") edge missing'
    }

    # 6. lane wiring on the script lines
    if not ($lines | any {|l| ($l | str contains 'planner-preflight.nu') and ($l | str contains '--assignee loop') and ($l | str contains '--work-item')}) {
        fail 'preflight script line lost --work-item --assignee loop (legacy candidate mode / wrong risk lane would ship)'
    }
    let pfstart = ($lines | enumerate | where {|e| ($e.item | str trim) == 'preflight ['} | get -o index | first | default null)
    if $pfstart == null { fail 'preflight node not found' }
    let pfend = ($lines | enumerate | where {|e| ($e.index > $pfstart) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $pfend == null { fail 'preflight node block not terminated' }
    let pfblock = ($lines | skip ($pfstart + 1) | take ($pfend - $pfstart - 1))
    if not ($pfblock | any {|l| ($l | str trim | str trim -r -c ',') == 'stdin_source="current_item_json"'}) {
        fail 'preflight stdin_source lost current_item_json (the packet plumbing)'
    }
    if not ($lines | any {|l| ($l | str contains 'evidence.nu') and ($l | str contains '--lane loop') and ($l | str contains '--work-item')}) {
        fail 'evidence script line lost --lane loop --work-item (product review scope / legacy seed protocol would ship)'
    }
    let estart = ($lines | enumerate | where {|e| ($e.item | str trim) == 'evidence ['} | get -o index | first | default null)
    if $estart == null { fail 'evidence node not found' }
    let eend = ($lines | enumerate | where {|e| ($e.index > $estart) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $eend == null { fail 'evidence node block not terminated' }
    let eblock = ($lines | skip ($estart + 1) | take ($eend - $estart - 1))
    if not ($eblock | any {|l| ($l | str trim | str trim -r -c ',') == 'stdin_source="current_item_json"'}) {
        fail 'evidence stdin_source lost current_item_json (the packet plumbing)'
    }
    # closeout script form (denkhaus/work#990): the capability channel
    # carries the tracker calls — closeout.nu --work-item --lane loop on
    # the node with the packet on stdin; NO prompt, NO fabro_tools, and
    # the agent prompt file is GONE (a reappearing one would mean the
    # retired choreography crept back)
    let cstart = ($lines | enumerate | where {|e| ($e.item | str trim) == 'closeout ['} | get -o index | first | default null)
    if $cstart == null { fail 'closeout node not found' }
    let cend = ($lines | enumerate | where {|e| ($e.index > $cstart) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $cend == null { fail 'closeout node block not terminated' }
    let cblock = ($lines | skip ($cstart + 1) | take ($cend - $cstart - 1))
    if not ($cblock | any {|l| ($l | str contains 'closeout.nu --work-item --lane loop')}) {
        fail 'closeout node must run closeout.nu --work-item --lane loop — the deterministic channel form (denkhaus/work#990)'
    }
    if not ($cblock | any {|l| ($l | str trim | str trim -r -c ',') == 'stdin_source="current_item_json"'}) {
        fail 'closeout stdin_source lost current_item_json — the packet must reach the close node'
    }
    if ($cblock | any {|l| ($l | str contains 'prompt=') or ($l | str contains 'fabro_tools')}) {
        fail 'closeout must NOT be an agent node anymore — the channel carries the tracker calls'
    }
    let closeout_prompt = ('.fabro/workflows/loop/prompts/closeout.md' | path expand)
    if ($closeout_prompt | path exists) { fail $"the closeout agent prompt must stay deleted: ($closeout_prompt)" }

    # 6b. planner packet plumbing
    let pstart = ($lines | enumerate | where {|e| ($e.item | str trim) == 'planner ['} | get -o index | first | default null)
    if $pstart == null { fail 'planner node not found (6b)' }
    let pend = ($lines | enumerate | where {|e| ($e.index > $pstart) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $pend == null { fail 'planner node block not terminated (6b)' }
    let pblock = ($lines | skip ($pstart + 1) | take ($pend - $pstart - 1))
    if not ($pblock | any {|l| ($l | str trim | str starts-with 'x.context_allow_keys=') and ($l | str contains 'current_item_json')}) {
        fail 'planner x.context_allow_keys lost current_item_json — the e47c lint would drop the packet and starve evidence'
    }

    # 7. implementer envelope pins
    let istart = ($lines | enumerate | where {|e| ($e.item | str trim) == 'implementer ['} | get -o index | first | default null)
    if $istart == null { fail 'implementer node not found' }
    let iend = ($lines | enumerate | where {|e| ($e.index > $istart) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $iend == null { fail 'implementer node block not terminated' }
    let iblock = ($lines | skip ($istart + 1) | take ($iend - $istart - 1) | each {|l| $l | split row '//' | first | str trim | str trim -r -c ','})
    if not ($iblock | any {|l| $l == $FS_WRITE_EXPECTED}) {
        fail $"implementer x.fs_write drifted — expected exactly ($FS_WRITE_EXPECTED) \(the a9bb protection surface\)"
    }
    if not ($iblock | any {|l| ($l | str starts-with 'x.fs_hide=') and ($l | str contains '.agents/')}) {
        fail 'implementer x.fs_hide lost .agents/ — the meta lane must still NOT see/edit session-level skills'
    }

    # 8. gate-red bounce edge
    if not ($lines | any {|l| ($l | str contains 'tester -> implementer') and ($l | str contains 'Gate red')}) {
        fail 'tester -> implementer ("Gate red") edge missing — red bounces would dead-end'
    }

    # 9. tester runs the loop gate, never the product gate
    if ($lines | any {|l| ($l | str contains 'just qualitygate')}) {
        fail 'graph names the product gate (just qualitygate) — the loop tester owns loop-gate.nu only'
    }
    if not ($lines | any {|l| ($l | str contains 'loop-gate.nu')}) {
        fail 'tester script line lost loop-gate.nu'
    }

    # 10. reviewer evidence line budget (fabro-31f7): the stage Output
    # render is a TAIL cap, and evidence.nu puts the integrity header +
    # seed-work diff at the capture HEAD — a low cap silently drops
    # exactly that head ("(275 lines omitted)" hid new-script heads from
    # a reviewer). The reviewer's x.preamble_output_max_lines must stay
    # >= 1000 so every realistic capture renders whole; pathological
    # sizes are evidence.nu's own HARD_CAP disclosure, never this cap.
    let rstart = ($lines | enumerate | where {|e| ($e.item | str trim) == 'reviewer ['} | get -o index | first | default null)
    if $rstart == null { fail 'reviewer node not found' }
    let rend = ($lines | enumerate | where {|e| ($e.index > $rstart) and (($e.item | str trim) == ']')} | get -o index | first | default null)
    if $rend == null { fail 'reviewer node block not terminated' }
    let rblock = ($lines | skip ($rstart + 1) | take ($rend - $rstart - 1) | each {|l| $l | split row '//' | first | str trim | str trim -r -c ','})
    let cap = ($rblock | where {|l| $l | str starts-with 'x.preamble_output_max_lines='} | first | default null)
    if $cap == null {
        fail 'reviewer node lost x.preamble_output_max_lines — the engine default tail cap would silently omit evidence heads'
    }
    let value = ($cap | str replace --regex '\D+' '' | into int)
    if $value < 1000 {
        fail $"reviewer x.preamble_output_max_lines=($value) is below the fabro-31f7 floor of 1000 — evidence heads would be silently omitted"
    }

    # 11. toolchain-placement guard (fabro-3ab2; lane flag + single-start
    # edge added in fabro-2357 so all three lane smokes assert the same
    # contract). start routes through env_guard (sh, never nu) before
    # tracker_guard, the node names the loop lane, and start has no second
    # outgoing edge that would bypass the guard. Reads COMMENT-STRIPPED
    # lines: the rationale comments name the contract, only code may
    # satisfy it.
    let guard_lines = ($lines | each {|l| ($l | split row '//' | first)})
    if not ($guard_lines | any {|l| ($l | str contains 'start -> env_guard')}) {
        fail 'start -> env_guard edge missing — a misplaced manual fire would die cryptically at the first nu stage'
    }
    if not ($guard_lines | any {|l| ($l | str contains 'env_guard -> planner')}) {
        fail 'env_guard -> planner edge missing — the placement guard must precede every nu stage'
    }
    if not ($guard_lines | any {|l| ($l | str contains 'toolchain-guard.sh loop') and ($l | str starts-with '        script="sh ')}) {
        fail 'env_guard script line must run the shared guard via POSIX sh with the loop lane flag — nu cannot guard a sandbox without nu'
    }
    let guard_start_edges = ($guard_lines | where {|l| ($l | str trim | str starts-with 'start ->')})
    if ($guard_start_edges | length) != 1 {
        fail $'start must have exactly ONE outgoing edge, the placement guard; found ($guard_start_edges | length) — a second edge would bypass it'
    }

    # 12. spec-paths coverage after the claim-check retirement
    # (fabro-786a): the tester's seed-spec-paths tier (qualitygate,
    # sourcing the shared lib) owns the kill; the implementer prompt
    # must name that tier so a spec-paths miss routes Blocked at the
    # latest before the tester, and the retired claim-check script must
    # stay gone (its pins died with it; the lib is canonical).
    let IMPLEMENTER_PROMPT = ('.fabro/workflows/loop/prompts/implementer.md' | path expand)
    if not ((open --raw $IMPLEMENTER_PROMPT) | str contains 'seed-spec-paths tier') {
        fail 'implementer prompt lost the seed-spec-paths tier naming — a spec-paths miss would bounce off the tester unexplained'
    }
    let CLAIM_CHECK = ('.fabro/workflows/develop/scripts/claim-check.nu' | path expand)
    if ($CLAIM_CHECK | path exists) {
        fail 'claim-check.nu still exists — the retired script and its pins died with the node (fabro-786a); the lib is canonical'
    }

    print 'loop graph-contract-smoke: OK — guard retirement, preflight topology, lane wiring, envelope pins, red bounce, evidence budget, placement guard, spec-paths tier naming intact'
}
