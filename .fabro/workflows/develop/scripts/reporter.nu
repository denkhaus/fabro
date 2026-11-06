#!/usr/bin/env nu
# The reporter stage (ADR-0039 stages C and G, denkhaus/work#1107 and
# #1115): the run reports FACTS as observation records — it never
# judges, never closes, never files. The adjudicator decides; the chain
# closes.
#
# The reviewer's Approved edge lands HERE. The engine delivers the
# granted-item packet via stdin (stdin_source="current_item_json" on
# the node). The item comes from CONTEXT, never from "first claimed
# item": the tracker carries parallel claims (stale or live), and run
# 01M1NK4V3YG3AQAMEKDJ6V471F closed two WRONG stale items that way
# before that fix.
#
# Stage-G cutover (2026-11-06): the closeout's authority retired. The
# park gate, the closure-discipline pre-close GATE, the three advisory
# marker sweeps, and the claim-bound close are GONE — the stage-E close
# door made the adjudicator the only closer, and the visibility
# CHECK survives because it computes the FACT's verdict (not-visible
# and degraded are painpoints; the adjudicator's agent pass owns the
# decision the park gate once made). A develop run after the cutover:
# implement, verify, produce the PR, report facts, exit.

# Shared lane-script modules (denkhaus/work#1108): the run-anchor triple
# (branch/run-id/run-base) and the work channel (server/bin resolution,
# the facade call, the observation door) live in .fabro/scripts/lib/ —
# one implementation, every lane script a consumer (the adjudicator
# fast-path script and this reporter source the same files).
use ../../../scripts/lib/run_anchor.nu
use ../../../scripts/lib/work_channel.nu

# ---------------------------------------------------------------------------
# Dockerfile-touch detection (fabro-6f6e)
#
# WHY: run 01M1YTVK73YEJXW4542MWDX4BJ (seed fabro-05d0) delivered only a
# .fabro/Dockerfile.toolchain edit; Docker is absent from run sandboxes
# so no image build ran, the review approved green — yet the fix stayed
# INERT because fabro-toolchain:noble is BUILT, not mounted, and a
# Dockerfile* edit takes effect only after that image rebuilds. The
# reporter fact below carries that verdict into the chain as a
# painpoint (an image rebuild is a pending action).
# ---------------------------------------------------------------------------

# Pure filter over a path list: any path with a SEGMENT matching the glob
# `Dockerfile*` (covers .fabro/Dockerfile.toolchain, .fabro/Dockerfile,
# root Dockerfile, nested x/Dockerfile.dev). Pure over its input so the
# smoke test (reporter-smoke.nu) exercises it without git.
def dockerfile-hits [paths: list<string>]: nothing -> list<string> {
    $paths | where {|p| ($p | split row "/" | any {|seg| $seg | str starts-with "Dockerfile"})}
}

# The Dockerfile* paths the run diff (run-base anchored) touches, [] on
# any git failure. Takes the THREADED anchor (computed once in main).
def dockerfile-hits-in-diff [anchor: record<base: string, short: string, grounded: bool>]: nothing -> list<string> {
    let res = (do { git diff --name-only $anchor.base } | complete)
    if $res.exit_code != 0 { return [] }
    dockerfile-hits ($res.stdout | lines | compact)
}

# --- Reporter facts (ADR-0039 stage C, denkhaus/work#1107) ---------------
# The run reports FACTS as observation records; it never judges — the
# adjudicator does. The category encodes whether a fact needs a DECISION
# (stage-C decision 5, trigger-based): painpoints and markers FORCE the
# adjudicator agent pass; notes never trigger. Facts fire before the
# run ends, so every terminal run carries its verdicts for adjudicator
# pass one.

# Pure: the visibility fact over the demand check's verdict. NOT visible
# is the fabro-02c4 class — as a note, the guard would erode (an
# approval without an implementing diff must not reach merge:enabled).
# The machine head on line one (lib/fact_head.nu owns the grammar): the
# fast path's parse and the engine-retry dedupe key on it.
def visibility-fact [verdict: string, item_key: string, base_short: string]: nothing -> record<category: string, text: string> {
    let head = $"fact=patch-visibility verdict=($verdict) base=($base_short) item=($item_key)"
    if $verdict == "visible" {
        {category: "note", text: $"($head)\npatch-visibility: the demand of ($item_key) is visible in the run-anchored diff \(base ($base_short)\)"}
    } else if $verdict == "degraded" {
        {category: "painpoint", text: $"($head)\npatch-visibility: the demand of ($item_key) could not be determined \(check degraded, base ($base_short)\) — the adjudicator must verify the patch"}
    } else {
        {category: "painpoint", text: $"($head)\npatch-visibility: the demand of ($item_key) is NOT visible in the run-anchored diff \(base ($base_short)\) — the fabro-02c4 class: an approval without an implementing diff must not merge"}
    }
}

# Pure: the Dockerfile-touch fact. Null when nothing touched (no
# empty-entry noise).
def dockerfile-fact [hits: list<string>]: nothing -> any {
    if ($hits | is-empty) { return null }
    let head = $"fact=dockerfile-touch paths=($hits | str join ',')"
    {category: "painpoint", text: $"($head)\ndockerfile-touch: the run diff touches \(($hits | str join ', ')\) — the fix stays INERT until the fabro-toolchain:noble image is rebuilt \(fabro-6f6e\)"}
}

# One fact through the CLI door (`fabro observation record`, the agent
# tool's twin — door symmetry, ADR-0039 decision 12). HARD by contract
# (user decision 2026-11-01): the work channel's observation door itself
# exits non-zero on a failed record (a failed fact fails the stage) —
# the engine retries, the caller fixes the error, never absorbs it.
def record-fact [fact: any]: nothing -> nothing {
    if ($fact | is-empty) { return }
    let res = (work_channel observation-record 'reporter' $fact.category $fact.text)
    if $res.exit_code != 0 {
        print -e $"reporter: observation record failed \(category ($fact.category)\): ($res.stderr | str trim | str substring 0..240)"
        return
    }
    print $"reporter: reported fact — ($res.stdout | str trim)"
}

# ---------------------------------------------------------------------------
# The visibility check (fabro-02c4 heritage; the GATE retired at stage
# G, the CHECK survives as the fact's verdict source)
#
# WHY: seed fabro-9967 was closed 2026-09-09 15:42 although its demand
# was never implemented — NO implementing diff existed — and the
# revisor caught it only by accident (run 01M23J61HH8Z, regression
# finding). The closeout's pre-close GATE retired with its authority;
# the same computation now feeds the visibility FACT: not-visible and
# degraded are painpoints, and the adjudicator's agent pass owns the
# decision. Tracker/journal bookkeeping is excluded from the patch
# (those files mechanically echo item metadata and would satisfy the
# match with zero implementation).
#
# A DEGRADED check (tooling failure) reports degraded — never a verdict
# the check never established.
# ---------------------------------------------------------------------------

# Pure: distinctive demand tokens from an item title — lowercase
# alphanumeric runs of length >= 4, minus function-word stopwords.
def demand-tokens [title: string]: nothing -> list<string> {
    let stopwords = [seed task fabro with from that this when then they them will must into else only than have been does were what which where while about]
    $title | str lowercase | split row --regex '[^a-z0-9]+' | where {|t|
        ($t | str length) >= 4 and not ($t in $stopwords)
    } | uniq
}

# Pure: is the item's demand literally visible in the run patch? Empty
# patch -> never visible (the fabro-9967 shape). Empty tokens -> any
# non-empty patch passes (degrade). Otherwise ANY distinctive title
# token found in the patch counts.
def demand-visible [tokens: list<string>, patch: string]: nothing -> bool {
    if ($patch | str trim | is-empty) { return false }
    if ($tokens | is-empty) { return true }
    $tokens | any {|t| $patch | str contains $t }
}

# Best-effort visibility verdict for THIS item against the run diff
# (run-base anchored — work-item claims are server-side,
# denkhaus/work#1056): 'visible', 'not-visible', or 'degraded'. Takes
# the THREADED anchor (computed once in main).
def item-demand-verdict [item_key: string, anchor: record<base: string, short: string, grounded: bool>]: nothing -> string {
    let diff_res = (do { git diff $anchor.base -- . ':(exclude).fabro/journal' } | complete)
    if $diff_res.exit_code != 0 { return "degraded" }
    let title = (do -i { work-item-title $item_key } | default '')
    if ($title | is-empty) {
        # the title never resolved (a channel outage printed its own
        # warning below): an empty patch is honestly not visible; a
        # non-empty one could not be compared.
        if not (demand-visible [] $diff_res.stdout) { return "not-visible" }
        return "degraded"
    }
    let tokens = (demand-tokens $title)
    if (demand-visible $tokens $diff_res.stdout) { "visible" } else { "not-visible" }
}

# The item's title through the channel (denkhaus/work#990): show is the
# authoritative resolver — the packet only NAMES the item. Degrades to
# '' WITH a stderr trace — a channel outage must never disarm the check
# silently.
def work-item-title [item_key: string]: nothing -> string {
    let res = (work_channel call 'reporter' [show --project fabro $item_key --json])
    if $res.exit_code != 0 {
        print -e $"reporter: WARNING — work show \($item_key\) failed, the visibility check degrades: ($res.stderr | str trim | str substring 0..160)"
        return ''
    }
    let item = (try { $res.stdout | from json } catch { null })
    if ($item | describe | str starts-with "record") { $item | get -o title | default '' } else { '' }
}

# The reporter: validate the packet, compute the run anchor once,
# record the facts, exit. --work-item is the ONLY mode (the graphs wire
# the node with stdin_source="current_item_json", the granted-item
# packet).
def main [--work-item]: nothing -> nothing {
    if not $work_item {
        print -e "reporter: --work-item is the only mode (the graphs deliver the granted-item packet)"
        exit 2
    }
    # Non-tty stdin: nu 0.115's `input` only works on a tty and raises an
    # I/O error on pipes (run 01M1PVMS7B6N39MG0041C5F7P6) — the engine
    # pipes the context value, so read it through an external `cat`,
    # which inherits the piped stdin.
    let raw = (cat | str join | str trim)
    if ($raw | is-empty) {
        print -e "reporter: stdin carried no granted-item packet (current_item_json missing — planner context_updates drop)"
        exit 1
    }
    let pkt = (try { $raw | from json } catch { null })
    if not ($pkt | describe | str starts-with "record") {
        print -e $"reporter: granted-item packet is not a JSON object: ($raw | str substring 0..120)"
        exit 1
    }
    let key = ($pkt | get -o itemKey | default "" | str trim)
    if ($key | is-empty) {
        print -e "reporter: granted-item packet carries no itemKey"
        exit 1
    }

    # The run anchor, computed ONCE and threaded (denkhaus/work#1108):
    # the demand verdict and the Dockerfile-hits check anchor on the
    # same run base.
    let anchor = (do -i { run_anchor run-base } | default {base: "HEAD", short: "", grounded: false})

    # The facts (ADR-0039 stage C): both fire before the run ends, so
    # every terminal run carries its verdict for adjudicator pass one —
    # the chain, not this stage, decides what they mean.
    let demand_verdict = (do -i { item-demand-verdict $key $anchor } | default "degraded")
    record-fact (visibility-fact $demand_verdict $key $anchor.short)
    record-fact (dockerfile-fact (dockerfile-hits-in-diff $anchor))

    print $"reporter: reported facts for ($key) — the adjudicator decides"
}
