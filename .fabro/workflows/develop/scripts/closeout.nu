#!/usr/bin/env nu
# Closeout (ADR-0010, ADR-0032, denkhaus/work#990): deterministic
# post-approval bookkeeping.
#
# The reviewer's Approved edge lands HERE. This script closes EXACTLY the
# work item this run claimed — delivered by the engine via stdin
# (stdin_source="current_item_json" on the node, the planner-forwarded
# granted-item packet). One item per run: after a successful close the
# run exits; the next item is the next run's job (user directive
# 2026-09-04 — bounded runs keep the working field clear and single-item
# and keep run cost predictable).
#
# The item comes from CONTEXT, never from "first claimed item":
# the tracker carries parallel claims (stale or live), and run
# 01M1NK4V3YG3AQAMEKDJ6V471F closed two WRONG stale items that way
# before this fix.
#
# Since denkhaus/work#990 the tracker calls go through the per-run
# capability channel: `fabro work …` routes over FABRO_RUN_CHANNEL_SOCK
# with the run's worker principal, and the claim-bound authorization
# (denkhaus/work#991) makes closing a FOREIGN item impossible by
# construction — stronger than any seeds-era gate. No agent stage, no
# plan manifest: the script IS the executor again.
#
# Never an LLM: run 01M0… measured planner@2 at 21s / $0.021 / ~7% of wall
# time for exactly these mechanical actions. Failure keeps the item open —
# the next run re-enters the review cycle; an approved-but-unclosed item is
# re-approvable harmlessly.

# Shared lane-script modules (denkhaus/work#1108): the run-anchor triple
# (branch/run-id/run-base) and the work channel (server/bin resolution,
# the facade call, the observation door) live in .fabro/scripts/lib/ —
# one implementation, every lane script a consumer (the adjudicator
# fast-path script and the stage-G reporter source the same files).
use ../../../scripts/lib/run_anchor.nu
use ../../../scripts/lib/work_channel.nu

# ---------------------------------------------------------------------------
# Dockerfile-touch warning (fabro-6f6e)
#
# WHY: run 01M1YTVK73YEJXW4542MWDX4BJ (seed fabro-05d0) delivered only a
# .fabro/Dockerfile.toolchain edit; Docker is absent from run sandboxes so
# no image build ran, the review approved green — yet the fix stayed INERT
# because fabro-toolchain:noble is BUILT, not mounted, and a Dockerfile*
# edit takes effect only after that image rebuilds. Same class as the gh
# breakage in .fabro/Dockerfile (since 98ed9f1) that still produced exit
# 127 in run containers on Sep 7. This warning names the touched files at
# the moment the close is decided. It must NEVER fail or block the close:
# every git call below is wrapped in `do { ... } | complete` (git errors
# degrade to silence) and the whole check runs under `do -i` — `complete`
# only accepts external commands, so the outer guard is do-i rather than
# do|complete, with the same degrade-to-silence semantics.
#
# SPEC SCOPE: run-summary/PR-body surfacing of this warning is exactly the
# channel open seed fabro-5b0a proposes and does NOT exist yet — this seed
# deliberately does NOT build it. The warning's captured stdout/stderr
# lands in the stage journal and run output, which satisfies this seed;
# fabro-5b0a remains the follow-up for PR-body surfacing.
# ---------------------------------------------------------------------------

# Pure filter over a path list: any path with a SEGMENT matching the glob
# `Dockerfile*` (covers .fabro/Dockerfile.toolchain, .fabro/Dockerfile,
# root Dockerfile, nested x/Dockerfile.dev). Pure over its input so the
# smoke test (closeout-smoke.nu) exercises it without git.
def dockerfile-hits [paths: list<string>]: nothing -> list<string> {
    $paths | where {|p| ($p | split row "/" | any {|seg| $seg | str starts-with "Dockerfile"})}
}

# --- diff-anchor helpers: the run-anchor module (denkhaus/work#1108)
# owns branch/run-id/run-base — run-base anchoring since denkhaus/work#1056
# (claims are server-side); this script computes the anchor ONCE in main
# and threads it (the pre-extraction triple computation retired).

# The Dockerfile* paths the run diff (run-base anchored) touches, [] on
# any git failure: the shared seam of the warning above and the reporter
# fact below (ADR-0039 stage C) — one detection, two surfaces. Takes the
# THREADED anchor (computed once in main).
def dockerfile-hits-in-diff [anchor: record<base: string, short: string, grounded: bool>]: nothing -> list<string> {
    let res = (do { git diff --name-only $anchor.base } | complete)
    if $res.exit_code != 0 { return [] }
    dockerfile-hits ($res.stdout | lines | compact)
}

# Best-effort detection + warning: warn when the run diff (run-base
# anchored — work-item claims are server-side, no tracker-file claim
# anchor exists; denkhaus/work#1056) touches any Dockerfile* path.
# Prints the warning to BOTH stderr and stdout (both are captured into
# the stage journal / run output); never blocks the close. Takes the
# THREADED anchor (computed once in main).
def warn-dockerfile-diff [anchor: record<base: string, short: string, grounded: bool>]: nothing -> nothing {
    let hits = (dockerfile-hits-in-diff $anchor)
    if ($hits | is-empty) { return }
    let msg = $"closeout: WARNING — diff touches Dockerfile path\(s\): ($hits | str join ', '). This fix is INERT until the fabro-toolchain:noble image is rebuilt — run sandboxes build that image and never mount it live."
    print -e $msg
    print $msg
}

# --- Reporter facts (ADR-0039 stage C, denkhaus/work#1107) ---------------
# The run reports FACTS as observation records; it never judges — the
# adjudicator does. The category encodes whether a fact needs a DECISION
# (grilled 2026-11-01): a painpoint forces the adjudicator's agent pass,
# a note never triggers anything. Facts fire BEFORE the park gate, so a
# parked run still carries its verdict.

# Pure: the visibility fact over the park gate's verdict. NOT visible is
# the fabro-02c4 class — as a note, the fast path would close what an
# approval without an implementing diff must not. DEGRADED (the check
# failed) never asserts a verdict it did not establish, and the
# adjudicator must look. Line one is the deterministic machine head
# (lib/fact_head.nu owns the grammar): the fast path's parse and the
# retry-dedupe key; prose follows after it.
def visibility-fact [verdict: string, item_key: string, base_short: string]: nothing -> record<category: string, text: string> {
    let head = $"fact=patch-visibility verdict=($verdict) base=($base_short) item=($item_key)"
    if $verdict == "visible" {
        {category: "note", text: $"($head)\npatch-visibility: the demand of ($item_key) is visible in the run-anchored diff \(base ($base_short)\)"}
    } else if $verdict == "degraded" {
        {category: "painpoint", text: $"($head)\npatch-visibility: the demand of ($item_key) could not be determined \(check degraded, base ($base_short)\) — the park gate degraded to proceeding; the adjudicator must verify the patch"}
    } else {
        {category: "painpoint", text: $"($head)\npatch-visibility: the demand of ($item_key) is NOT visible in the run-anchored diff \(base ($base_short)\) — the fabro-02c4 class: an approval without an implementing diff must not close"}
    }
}

# Pure: the Dockerfile-touch fact. Null when nothing touched (no
# empty-entry noise — a command stage has no presence contract). A touch
# is a painpoint: an image rebuild is pending, or the fix stays inert
# (fabro-6f6e). Line one is the machine head (paths comma-joined).
def dockerfile-fact [hits: list<string>]: nothing -> any {
    if ($hits | is-empty) { return null }
    let head = $"fact=dockerfile-touch paths=($hits | str join ',')"
    {category: "painpoint", text: $"($head)\ndockerfile-touch: the run diff touches \(($hits | str join ', ')\) — the fix stays INERT until the fabro-toolchain:noble image is rebuilt \(fabro-6f6e\)"}
}

# One fact through the CLI door (`fabro observation record`, the agent
# tool's script twin — argv and transport live in the work-channel
# module): the engine binds FABRO_RUN_ID, FABRO_EXECUTION and
# FABRO_FIRING into this script's environment (stage self-awareness),
# so the call carries no identity flags. HARD by contract (user
# decision 2026-11-01): a failed record fails the stage — the engine
# retries, the caller fixes the error, never absorbs it.
def record-fact [fact: any]: nothing -> nothing {
    if ($fact | is-empty) { return }
    let res = (work_channel observation-record 'closeout' $fact.category $fact.text)
    if $res.exit_code != 0 {
        print -e $"closeout: observation record failed \(category ($fact.category)\): ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    print $"closeout: reported fact — ($res.stdout | str trim)"
}


# ---------------------------------------------------------------------------
# Closure-discipline pre-close check (fabro-02c4)
#
# WHY: seed fabro-9967 was closed 2026-09-09 15:42 (in a seeds:sync)
# although its demand (seeds ready cap + created_since in planner.md) was
# never implemented — NO implementing diff existed — and the revisor
# caught it only by accident while deduping (run 01M23J61HH8Z,
# regression finding). Before its `seeds close`, this gate verifies the
# run actually delivered a diff the seed's demand is visible in
# (run-base anchored — the same run-base helper the
# Dockerfile warning uses). If not visible, the run does NOT close: the
# seed stays OPEN and a PARK note goes to stdout AND stderr (both land
# in the stage journal / run output) for the next cycle to act on.
#
# Tracker/journal bookkeeping (`.seeds`, `.fabro/journal`) is excluded
# from the patch: those files mechanically echo seed metadata (the
# claim itself, journal records quoting the brief) and would satisfy
# the match with zero implementation.
#
# Degrade-to-close: any git/seeds failure inside the verdict returns
# visible=true — this gate must never block a legitimate close on
# tooling error (same philosophy as the Dockerfile warning above). An
# empty token set (title unparsable) degrades to a non-empty-patch
# check.
# ---------------------------------------------------------------------------

# Pure: distinctive demand tokens from a seed title — lowercase
# alphanumeric runs of length >= 4, minus function-word stopwords.
def demand-tokens [title: string]: nothing -> list<string> {
    let stopwords = [seed task fabro with from that this when then they them will must into else only than have been does were what which where while about]
    $title | str lowercase | split row --regex '[^a-z0-9]+' | where {|t|
        ($t | str length) >= 4 and not ($t in $stopwords)
    } | uniq
}

# Pure: is the seed's demand literally visible in the run patch? Empty
# patch -> never visible (the fabro-9967 shape: closed with no
# implementing diff). Empty tokens -> any non-empty patch passes
# (degrade). Otherwise ANY distinctive title token found in the patch
# counts: the patch is this run's own claim-anchored work, so a hit
# means the run touched what the seed names.
def demand-visible [tokens: list<string>, patch: string]: nothing -> bool {
    if ($patch | str trim | is-empty) { return false }
    if ($tokens | is-empty) { return true }
    $tokens | any {|t| $patch | str contains $t }
}

# Best-effort visibility verdict for THIS item against the run diff
# (run-base anchored — work-item claims are server-side, no tracker-file
# claim transition to anchor on; denkhaus/work#1056): 'visible',
# 'not-visible', or 'degraded'. The park gate below degrades to
# PROCEEDING on 'degraded' (a tooling failure must never block a
# legitimate close); the reporter fact degrades to a PAINPOINT (it must
# never assert a verdict the check never established). Takes the
# THREADED anchor (computed once in main).
def item-demand-verdict [item_key: string, anchor: record<base: string, short: string, grounded: bool>]: nothing -> string {
    let diff_res = (do { git diff $anchor.base -- . ':(exclude).fabro/journal' } | complete)
    if $diff_res.exit_code != 0 { return "degraded" }
    let title = (do -i { work-item-title $item_key } | default '')
    if ($title | is-empty) {
        # the title never resolved (a channel outage printed its own
        # warning above): an empty patch is honestly not visible; a
        # non-empty one could not be compared.
        if not (demand-visible [] $diff_res.stdout) { return "not-visible" }
        return "degraded"
    }
    let tokens = (demand-tokens $title)
    if (demand-visible $tokens $diff_res.stdout) { "visible" } else { "not-visible" }
}

# The item's title through the channel (denkhaus/work#990): show is the
# authoritative resolver — the packet only NAMES the item. Degrades to ''
# WITH a stderr trace (the PARK gate then degrades to the non-empty-patch
# check) — a channel outage must never disarm the gate silently.
def work-item-title [item_key: string]: nothing -> string {
    let res = (work_channel call 'closeout' [show --project fabro $item_key --json])
    if $res.exit_code != 0 {
        print -e $"closeout: WARNING — work show \($item_key\) failed, the PARK gate degrades to the non-empty-patch check: ($res.stderr | str trim | str substring 0..160)"
        return ''
    }
    let item = (try { $res.stdout | from json } catch { null })
    if ($item | describe | str starts-with "record") { $item | get -o title | default '' } else { '' }
}

# ---------------------------------------------------------------------------
# Reviewer-journal non-blocking sweep (fabro-22fa)
#
# WHY: a reviewer that approves with residual defects notes them in its
# journal observations with explicit non-blocking wording ("non-blocking",
# "noted but not blocking", "not blocking"). Before fabro-22fa those
# findings lived only in `.fabro/journal/<run_id>.jsonl` — when closeout
# ran `seeds close`, the finding died with the closed seed: no open brief
# existed to fold the follow-up into. This sweep re-files each explicitly
# non-blocking reviewer observation as a NEW open seed BEFORE the close,
# with provenance (finding text, closed seed id, source run id) in the
# body so the finding survives closure traceably.
#
# ADVISORY ONLY: every failure (journal missing, JSON unparsable, seeds
# create error) logs to stderr and NEVER blocks the close — same
# degrade-to-silence philosophy as warn-dockerfile-diff (`do -i` +
# `do { ... } | complete` wrappers). Null path: a journal with no
# non-blocking reviewer findings yields an empty list and ZERO seeds create
# calls — byte-identical close semantics.
# ---------------------------------------------------------------------------

# Pure: does a reviewer observation carry an explicit non-blocking
# marker? Case-insensitive; "not blocking" also covers the "noted but
# not blocking" phrasing. An observation merely mentioning a blocking
# defect ("this is blocking") does NOT match.
def is-nonblocking [obs: string]: nothing -> bool {
    let low = ($obs | str lowercase)
    ($low | str contains "non-blocking") or ($low | str contains "not blocking")
}

# Pure: non-blocking findings from journal JSONL text — reviewer-node
# records only, their observations, filtered by is-nonblocking.
def nonblocking-from-journal [text: string]: nothing -> list<string> {
    let obs = (
        $text | lines | compact
        | each {|l| do -i { $l | from json } }
        | where {|r| ($r | describe | str starts-with "record") and (($r | get -o node | default '') == "reviewer")}
        | get -o data
        | where {|d| $d != null}
        | each {|d| $d | get -o observations | default []}
        | flatten
    )
    $obs | where {|o| is-nonblocking $o}
}

# Best-effort file-level read: missing/unreadable journal -> empty list.
def journal-nonblocking [journal_path: string]: nothing -> list<string> {
    do -i { nonblocking-from-journal (open --raw $journal_path) } | default []
}

# Pure: run id from the run branch (`fabro/run/<run_id>`), '' off-run —
# the run-anchor module's parse (denkhaus/work#1108).

# Pure: filed-seed title — short finding excerpt plus provenance.
def finding-title [finding: string, item_key: string]: nothing -> string {
    let excerpt = (if ($finding | str length) > 70 { $finding | str substring 0..69 } else { $finding })
    $"Reviewer residual \(non-blocking\) from ($item_key): ($excerpt)"
}

# Deterministic filing through the channel (denkhaus/work#990): the
# work-channel module's `create` (argv builder + STDIN body transport;
# tracker text never travels on a command line). Callers own the
# advisory failure handling.

# The fork-8 filing-time dedupe, deterministic since #990 (the agent
# stage that owned it is gone): titles of OPEN items that already cite
# this run — a re-run closeout skips re-filing what an earlier visit
# filed. Degrades to [] (dedupe off) on channel failure, like every
# advisory arm.
def filed-titles [run_id: string]: nothing -> list<string> {
    if ($run_id | is-empty) { return [] }
    let res = (work_channel call 'closeout' [list --project fabro --query $run_id --json])
    if $res.exit_code != 0 {
        print -e $"closeout: WARNING — work list query failed, the dedupe degrades to OFF for this close: ($res.stderr | str trim | str substring 0..160)"
        return []
    }
    let parsed = (try { $res.stdout | from json } catch { null })
    let items = (if ($parsed | describe | str starts-with "record") { $parsed | get -o data | default [] } else { $parsed })
    let kind = ($items | describe)
    if ($kind | str starts-with "list") or ($kind | str starts-with "table") {
        $items | get -o title | default []
    } else { [] }
}

# Advisory sweep: file each non-blocking reviewer finding as a work item
# (type bug, labels `residual` + the lane labels) BEFORE the close, so
# the finding survives the item's closure traceably. Never raises:
# caller wraps in `do -i`; channel failures print to stderr and never
# block the close.
def sweep-reviewer-findings [item_key: string, run_id: string, journal_path: string, lane_labels: list<string>, already_filed: list<string>]: nothing -> nothing {
    for finding in (journal-nonblocking $journal_path) {
        let title = (finding-title $finding $item_key)
        if ($title in $already_filed) {
            print $"closeout: skipped already-filed reviewer residual: ($title)"
            continue
        }
        let desc = $"Reviewer non-blocking residual from ($item_key) — re-filed at closeout so the finding survives the item's closure.\n\nFinding: \"($finding)\"\n\nOrigin: reviewer journal of run ($run_id), closed item ($item_key).\nBasis: run ($run_id), closed item ($item_key)"
        let res = (work_channel create 'closeout' $title $desc "bug" (["residual"] | append $lane_labels))
        if $res.exit_code != 0 {
            print -e $"closeout: WARNING — could not file reviewer finding \(non-blocking, from ($item_key)\): ($res.stderr | str trim)"
        } else {
            print $"closeout: filed reviewer residual as work item: ($res.stdout | str trim)"
        }
    }
}

# ---------------------------------------------------------------------------
# Deferred-action sweep (fabro-7aac)
#
# WHY: run 01M2P22VYWQNJM64D15XPHFP2C (seed fabro-3916's bun-generate
# follow-up) — an implementer discloses a deferred human follow-up (a
# regen-confirm or local-only step that cannot run in-sandbox) only in
# `implementation_summary`, which the planner node consumes but the
# stage journal never carries: when closeout ran `seeds close`, the
# follow-up died with the seed. Channel contract: implementer.md also
# emits each deferred action as a JOURNAL observation starting with the
# deterministic marker `deferred-action: ` (fabro-7aac), and this sweep
# filters implementer-node observations by that marker BEFORE the close.
#
# Advisory semantics identical to fabro-22fa above: `do -i` wrapping,
# complete-wrapped `seeds create`, failures print to stderr and never block
# the close. Placed AFTER the PARK gate so a parked (still-open) seed
# does not double-file on its re-run's closeout. Null path: a journal
# with no marker observations yields an empty list and ZERO seeds create
# calls — byte-identical close semantics.
# ---------------------------------------------------------------------------

# Pure: the deterministic journal marker. Single source of truth for
# both the filter (is-deferred-action) and the strip (deferred-text).
def deferred-marker []: nothing -> string {
    "deferred-action:"
}

# Pure: does an implementer observation carry the marker AT ITS START?
# Case-insensitive and whitespace-tolerant; an observation merely
# MENTIONING the marker mid-sentence does not match (the prompt contract
# puts the marker at the start, one observation per action).
def is-deferred-action [obs: string]: nothing -> bool {
    ($obs | str trim | str lowercase | str starts-with (deferred-marker))
}

# Pure: the action text after the marker, for the filed seed body.
# Non-matching input passes through unchanged (defensive; the sweep
# filter already matched).
def deferred-text [obs: string]: nothing -> string {
    let t = ($obs | str trim)
    let low = ($t | str lowercase)
    if not ($low | str starts-with (deferred-marker)) { return $t }
    let mlen = (deferred-marker | str length)
    $t | str substring ($mlen..) | str trim
}

# Pure: deferred actions from journal JSONL text — implementer-node
# records only, their observations, filtered by is-deferred-action.
def deferred-from-journal [text: string]: nothing -> list<string> {
    let obs = (
        $text | lines | compact
        | each {|l| do -i { $l | from json } }
        | where {|r| ($r | describe | str starts-with "record") and (($r | get -o node | default '') == "implementer")}
        | get -o data
        | where {|d| $d != null}
        | each {|d| $d | get -o observations | default []}
        | flatten
    )
    $obs | where {|o| is-deferred-action $o}
}

# Best-effort file-level read: missing/unreadable journal -> empty list.
def journal-deferred [journal_path: string]: nothing -> list<string> {
    do -i { deferred-from-journal (open --raw $journal_path) } | default []
}

# Pure: filed-seed title — bounded action excerpt plus provenance.
def deferred-title [action: string, item_key: string]: nothing -> string {
    let excerpt = (if ($action | str length) > 70 { $action | str substring 0..69 } else { $action })
    $"Deferred follow-up from ($item_key): ($excerpt)"
}

# Pure: is a stripped action text a no-op payload — empty or exactly
# `none` (case-insensitive)? A `deferred-action: none` observation means
# "nothing deferred" and must never file a seed (fabro-7ece: the sweep
# filed marker observations verbatim, so a none marker became a junk
# open seed).
def deferred-action-empty [text: string]: nothing -> bool {
    let t = ($text | str trim | str lowercase)
    ($t == "") or ($t == "none")
}

# Advisory sweep: file each deferred human follow-up as a work item
# (type task, labels `residual` + the lane labels — same pool semantics
# as sweep-reviewer-findings). Never raises: caller wraps in `do -i`;
# channel failures print to stderr and never block the close.
def sweep-deferred-actions [item_key: string, run_id: string, journal_path: string, lane_labels: list<string>, already_filed: list<string>]: nothing -> nothing {
    for action in (journal-deferred $journal_path) {
        let text = (deferred-text $action)
        if (deferred-action-empty $text) {
            print $"closeout: skipped empty/none deferred-action marker observation \(from ($item_key)\)"
            continue
        }
        let title = (deferred-title $text $item_key)
        if ($title in $already_filed) {
            print $"closeout: skipped already-filed deferred action: ($title)"
            continue
        }
        let desc = $"Deferred human follow-up the implementer disclosed while implementing ($item_key).\n\nAction: \"($text)\"\n\nOrigin: implementer journal of run ($run_id), deferred-action marker observation.\nBasis: run ($run_id), closed item ($item_key)"
        let res = (work_channel create 'closeout' $title $desc "task" (["residual"] | append $lane_labels))
        if $res.exit_code != 0 {
            print -e $"closeout: WARNING — could not file deferred action \(from ($item_key)\): ($res.stderr | str trim)"
        } else {
            print $"closeout: filed deferred action as work item: ($res.stdout | str trim)"
        }
    }
}

# ---------------------------------------------------------------------------
# EXEMPTION-arm sweep (fabro-534e)
#
# WHY: a brief can carry an EXEMPTION bullet — a deferred user/infra
# arm of the seed's demand the run cannot land (image release, deploy,
# external tool, human decision). Before this sweep that arm lived only
# in the brief (planner context) and the closed run's journal: when
# closeout ran `seeds close`, the reminder died with the seed and the
# deferred action went tracker-invisible. Channel contract (the brief
# is context, not a closeout input): planner.md marks each deferred arm
# as an `EXEMPTION: <action>` bullet in the brief AND emits the same
# arm as a JOURNAL observation starting with the deterministic marker
# `exemption: ` — this sweep filters PLANNER-node observations by that
# marker BEFORE the close and re-files each arm as a NEW open seed.
#
# The filed seed carries labels `ops,residual` (machine-filed
# provenance per fabro-2ab8, plus the ops class) and NO assignee:
# an EXEMPTION arm is by definition a user action, and assigning
# backlog seeds is the user's decision — visibility, not routing.
#
# Advisory semantics identical to the sweeps above: `do -i` wrapping,
# complete-wrapped `seeds create`, failures print to stderr and never
# block the close, placed after the PARK gate. Null path: no marker
# observations -> zero seeds create calls — byte-identical close
# semantics.
# ---------------------------------------------------------------------------

# Pure: the deterministic journal marker for brief EXEMPTION arms.
# Single source of truth for the filter (is-exemption) and the strip
# (exemption-text), mirroring deferred-marker above.
def exemption-marker []: nothing -> string {
    "exemption:"
}

# Pure: does a planner observation carry the marker AT ITS START?
# Case-insensitive and whitespace-tolerant; mid-sentence mentions do
# not match (the prompt contract puts the marker at the start, one
# observation per arm).
def is-exemption [obs: string]: nothing -> bool {
    ($obs | str trim | str lowercase | str starts-with (exemption-marker))
}

# Pure: the arm text after the marker, for the filed seed body.
# Non-matching input passes through unchanged (defensive; the sweep
# filter already matched).
def exemption-text [obs: string]: nothing -> string {
    let t = ($obs | str trim)
    let low = ($t | str lowercase)
    if not ($low | str starts-with (exemption-marker)) { return $t }
    let mlen = (exemption-marker | str length)
    $t | str substring ($mlen..) | str trim
}

# Pure: EXEMPTION arms from journal JSONL text — planner-node records
# only, their observations, filtered by is-exemption.
def exemptions-from-journal [text: string]: nothing -> list<string> {
    let obs = (
        $text | lines | compact
        | each {|l| do -i { $l | from json } }
        | where {|r| ($r | describe | str starts-with "record") and (($r | get -o node | default '') == "planner")}
        | get -o data
        | where {|d| $d != null}
        | each {|d| $d | get -o observations | default []}
        | flatten
    )
    $obs | where {|o| is-exemption $o}
}

# Best-effort file-level read: missing/unreadable journal -> empty list.
def journal-exemptions [journal_path: string]: nothing -> list<string> {
    do -i { exemptions-from-journal (open --raw $journal_path) } | default []
}

# Pure: filed-seed title — bounded arm excerpt plus provenance.
def exemption-title [arm: string, item_key: string]: nothing -> string {
    let excerpt = (if ($arm | str length) > 70 { $arm | str substring 0..69 } else { $arm })
    $"EXEMPTION arm from ($item_key): ($excerpt)"
}

# Advisory sweep: file each brief EXEMPTION arm as a work item (type
# task, labels `ops,residual` + the lane labels; NO lane of its own
# beyond that — an EXEMPTION arm is by definition a user action, and
# routing backlog items is the user's decision). Never raises: caller
# wraps in `do -i`; channel failures print to stderr and never block
# the close.
def sweep-exemption-arms [item_key: string, run_id: string, journal_path: string, lane_labels: list<string>, already_filed: list<string>]: nothing -> nothing {
    for arm in (journal-exemptions $journal_path) {
        let text = (exemption-text $arm)
        let title = (exemption-title $text $item_key)
        if ($title in $already_filed) {
            print $"closeout: skipped already-filed EXEMPTION arm: ($title)"
            continue
        }
        let desc = $"Deferred user/infra action the run exempted while implementing ($item_key) — the run could not land this arm itself.\n\nAction: \"($text)\"\n\nOrigin: planner journal of run ($run_id), EXEMPTION marker observation.\nBasis: run ($run_id), closed item ($item_key)"
        let res = (work_channel create 'closeout' $title $desc "task" (["ops" "residual"] | append $lane_labels))
        if $res.exit_code != 0 {
            print -e $"closeout: WARNING — could not file EXEMPTION arm \(from ($item_key)\): ($res.stderr | str trim)"
        } else {
            print $"closeout: filed EXEMPTION arm as work item: ($res.stdout | str trim)"
        }
    }
}

# Lane parameter (fabro-70b5): product (default, develop) or loop. The
# lane fixes the labels the advisory sweeps file under — residuals from
# a loop run re-enter that lane's queue (lane:loop); develop filings
# stay unlaned (human triage). Passed by the calling graph's script
# line; deterministic via the run's workflow spec.
def main [--lane: string = "", --work-item]: nothing -> nothing {
    # ADR-0032 fork 7/8 shipped a plan-only mode whose plan an AGENT
    # stage executed through the engine-mediated run tools — the
    # capability channel (denkhaus/work#990) retires that split: the
    # script files and closes DIRECTLY, `fabro work` over
    # FABRO_RUN_CHANNEL_SOCK with the run's worker principal, and the
    # claim-bound authorization (denkhaus/work#991) is the gate — a
    # foreign item's close is refused by construction, stronger than
    # any seeds-era script check. --work-item is the ONLY mode (the
    # seeds stdin leg died with the tracker); the graphs wire the node
    # with stdin_source="current_item_json", the granted-item packet.
    if not $work_item {
        print -e "closeout: --work-item is the only mode (the seeds stdin leg died with the tracker; the graphs deliver the granted-item packet)"
        exit 2
    }
    # Non-tty stdin: nu 0.115's `input` only works on a tty and raises an
    # I/O error on pipes (run 01M1PVMS7B6N39MG0041C5F7P6) — the engine pipes
    # the context value, so read it through an external `cat`, which inherits
    # the piped stdin.
    let raw = (cat | str join | str trim)
    if ($raw | is-empty) {
        print -e "closeout: stdin carried no granted-item packet (current_item_json missing — planner context_updates drop)"
        exit 1
    }
    let pkt = (try { $raw | from json } catch { null })
    if not ($pkt | describe | str starts-with "record") {
        print -e $"closeout: granted-item packet is not a JSON object: ($raw | str substring 0..120)"
        exit 1
    }
    let key = ($pkt | get -o itemKey | default "" | str trim)
    if ($key | is-empty) {
        print -e "closeout: granted-item packet carries no itemKey"
        exit 1
    }
    let lane_labels = (if ($lane | is-empty) { [] } else { [$"lane:($lane)"] })
    let rid = (do -i { run_anchor run-id '' } | default '')
    let journal_path = $".fabro/journal/($rid).jsonl"

    # The run anchor, computed ONCE and threaded (denkhaus/work#1108):
    # the demand verdict, the visibility fact's base, and the
    # Dockerfile-hits check all anchor on the same run base — the
    # pre-extraction shape computed it three times.
    let anchor = (do -i { run_anchor run-base } | default {base: "HEAD", short: "", grounded: false})

    # Reporter facts (ADR-0039 stage C): the run reports facts as
    # observation records, never judgments — the adjudicator decides.
    # Both facts fire BEFORE the park gate, so a parked run still carries
    # its verdict for adjudicator pass one. HARD by contract (user
    # decision 2026-11-01): a failed record fails the stage; the engine
    # retries, the caller fixes the error, never absorbs it.
    let demand_verdict = (do -i { item-demand-verdict $key $anchor } | default "degraded")
    record-fact (visibility-fact $demand_verdict $key $anchor.short)
    record-fact (dockerfile-fact (dockerfile-hits-in-diff $anchor))

    # Dockerfile-touch warning (fabro-6f6e): advisory only — `do -i` plus
    # the per-call complete wrappers above guarantee no failure here can
    # reach the close below. No Dockerfile touched: byte-identical close
    # semantics (packet validation, work close, exit codes).
    do -i { warn-dockerfile-diff $anchor } | ignore

    # Closure-discipline gate (fabro-02c4): PARK instead of closing when
    # the run diff shows no implementing change for the item's demand
    # (the title resolves through the channel; the diff anchors on the
    # run base — work-item claims are server-side, no tracker-file
    # transition to anchor on). Item stays open; exit 0 — a park is a
    # deliberate hold for the next cycle, not a stage failure. A DEGRADED
    # check proceeds (a tooling failure must never block a legitimate
    # close; the fact above flags it for the adjudicator). The verdict
    # itself is already reported as the fact above.
    if $demand_verdict == "not-visible" {
        let park = $"closeout: PARK — \(($key)\) NOT closed: the run diff \(run-base anchored, journal bookkeeping excluded\) shows no implementing change for the item's demand. fabro-9967 class: an approval with no implementing diff must not close. Item left OPEN — re-enter the review cycle."
        print -e $park
        print $park
        exit 0
    }

    # The fork-8 filing-time dedupe, deterministic since #990: titles of
    # open items that already cite THIS run — an earlier closeout visit
    # of the same run does not double-file.
    let already_filed = (do -i { filed-titles $rid } | default [])

    # Reviewer-journal sweep (fabro-22fa): re-file explicitly non-blocking
    # reviewer findings BEFORE the close, labeled `residual` (fabro-2ab8)
    # so their provenance is machine-visible. Advisory only — `do -i`
    # plus the complete-wrapped channel calls inside guarantee no failure
    # here can reach the close below. After the PARK gate so a parked
    # (still-open) item does not double-file on its re-run's closeout.
    # Null path: no findings -> zero create calls.
    do -i { sweep-reviewer-findings $key $rid $journal_path $lane_labels $already_filed } | ignore

    # Deferred-action sweep (fabro-7aac): re-file deferred human
    # follow-ups the implementer disclosed as `deferred-action:` journal
    # observations (marker contract in implementer.md) BEFORE the close.
    # Advisory only — same wrapping discipline, also after the PARK gate.
    # Null path: no marker observations -> zero create calls.
    do -i { sweep-deferred-actions $key $rid $journal_path $lane_labels $already_filed } | ignore

    # EXEMPTION-arm sweep (fabro-534e): re-file brief EXEMPTION bullets
    # the planner mirrored as `exemption:` journal observations
    # (marker contract in planner.md) as ops work items BEFORE the
    # close. Advisory only — same wrapping discipline, also after the
    # PARK gate. Null path: no marker observations -> zero create calls.
    do -i { sweep-exemption-arms $key $rid $journal_path $lane_labels $already_filed } | ignore

    # The close itself: claim-bound by construction (denkhaus/work#991)
    # — the channel's worker principal may close exactly THIS run's
    # claimed item; a mismatched packet is refused by the engine, not
    # by a script-side gate.
    let reason = $"Closed by run \(($rid)\) after reviewer approval — one item per run. The closeout gate verified the item's demand visible in the run-anchored diff; residuals, deferred actions, and EXEMPTION arms were re-filed before this close — see the closeout report in the run output."
    let res = (work_channel call 'closeout' [close --project fabro $key --reason $reason])
    if $res.exit_code != 0 {
        print -e $"closeout: work close ($key) failed: ($res.stderr | str trim)"
        exit 1
    }
    print $"closeout: closed ($key) — one item per run, exiting"
}
