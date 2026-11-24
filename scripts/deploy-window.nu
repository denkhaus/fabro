#!/usr/bin/env nu
# Deploy window ceremony (ADR-0041 host-built path) — the mechanical
# phases of a release window as repeatable steps. The HUMAN moments stay
# outside by design: the bench gate before the window (a judgment, never
# a recipe), the plan review between `plan` and `apply`, gopass warmth
# (a live store), and the live proof click after `apply`.
#
#   just deploy pause    pause every gear whose schedule trigger is on,
#                        verify by re-GET, remember the paused set for
#                        `arm`
#   just deploy build    host-release at the PUSHED tip (refuses a local
#                        tip that is not origin/denkhaus — the toolchain
#                        parity gate would catch it later; fail early).
#                        Host identity is resolved from the tofu prod
#                        profile at runtime and NEVER echoed (the public
#                        repo carries no addresses, 2026-11-09 leak).
#   just deploy pins     recompute the sandbox-plugin sha from the NEW
#                        image over ssh; write variables.tf defaults
#                        (fabro_image_ref + petri_sandbox_docker_sha256).
#                        The DEFAULT is the pin — an apply via -var
#                        override with an uncommitted default is the
#                        night-deploy rollback trap.
#   just deploy plan     BLOCKING gopass probe (no timeout wrapper — a
#                        cold store hangs, that IS the probe), then the
#                        tofu plan against envs/prod.tfvars.
#   just deploy apply    probe gopass AGAIN immediately before (the
#                        store relocks on its ~30 min TTL — warming
#                        early is not warming enough, 2026-11-10), then
#                        apply -auto-approve, then the version skew
#                        check (server sha == the deployed tip).
#   just deploy arm      re-enable exactly the gears `pause` paused
#                        (never a blanket enable), verify. NO toolchain
#                        pin: ADR-0042 retired it — the toolchain
#                        environment is repo-declared and cooked by the
#                        engine at run initialization from the checkout.
#   just deploy commit   commit the fabro-tofu variables.tf pin with a
#                        release message through a message file (git
#                        commit -F discipline). Push policy stays the
#                        session's — this phase commits, it never pushes.
#
# The paused-gear set lives in tmp/deploy-window-state.json; `arm`
# refuses to run without it.

const STATE_FILE = 'tmp/deploy-window-state.json'
const GEARS_JSON_ERROR_LIMIT = 200

def fabro-bin []: nothing -> string {
    let installed = ('~/.fabro/bin/fabro' | path expand)
    if ($installed | path exists) { $installed } else { 'fabro' }
}

# The tofu checkout must not be behind origin: a stale plan acts on a
# configuration missing live commits (2026-11-18: a stale plan offered
# to destroy the work#1188 alert webhook that origin had applied that
# afternoon — the shape review caught it, but the guard makes the
# refusal mechanical). Fails pins/plan/apply until the pull happened.
def assert-tofu-fresh [tofu_dir: string]: nothing -> nothing {
    let fetch = (do { git -C $tofu_dir fetch origin --quiet } | complete)
    if $fetch.exit_code != 0 {
        error make {msg: $"deploy-window: tofu fetch failed: ($fetch.stderr | str trim | str substring 0..160)"}
    }
    # Curated errors on the git failure modes: `into int` on an empty
    # rev-parse answer would fail closed but name nothing (the bare or
    # corrupt checkout case). Detached HEAD resolves against
    # origin/HEAD — the default branch, which is the intent here.
    let branch = (do { git -C $tofu_dir rev-parse --abbrev-ref HEAD } | complete)
    if $branch.exit_code != 0 {
        error make {msg: 'deploy-window: resolving the tofu branch failed — is the checkout a git worktree with a HEAD?'}
    }
    let branch = ($branch.stdout | str trim)
    let count = (do { git -C $tofu_dir rev-list --count $"HEAD..origin/($branch)" } | complete)
    if $count.exit_code != 0 {
        error make {msg: 'deploy-window: counting commits behind origin failed — inspect the tofu checkout'}
    }
    let behind = ($count.stdout | str trim | into int)
    if $behind > 0 {
        # Plain-string concatenation, NOT $"..." interpolation: the message
        # carries (s) and (2026-11-18: ...) — an interpolated string parses
        # those paren groups as subexpressions (the lint-nu bare-paren class).
        let msg = ('deploy-window: the tofu checkout is '
            + ($behind | into string)
            + ' commit(s) behind origin/'
            + $branch
            + ' — pull first; a stale plan acts on a configuration missing live commits (2026-11-18: it would have destroyed the work#1188 alert webhook applied from origin that afternoon)')
        error make {msg: $msg}
    }
}

# The tofu prod profile answers "which host?" — parsed at runtime, the
# values never echo (user directive 2026-11-15: infrastructure
# addresses stay out of every artifact this session produces).
def host-identity [tofu_dir: string]: nothing -> record {
    let profile = ($tofu_dir | path join 'envs' 'prod.tfvars')
    if not ($profile | path exists) {
        error make {msg: $"deploy-window: no prod profile at ($profile)"}
    }
    let text = (open --raw $profile)
    let ip = ($text | parse --regex 'ip\s*=\s*"([^"]+)"' | get -o 0.capture0)
    let port = ($text | parse --regex 'port\s*=\s*(\d+)' | get -o 0.capture0)
    let user = ($text | parse --regex 'user\s*=\s*"([^"]+)"' | get -o 0.capture0)
    if ($ip | is-empty) or ($port | is-empty) or ($user | is-empty) {
        error make {msg: 'deploy-window: the prod profile names no complete host object'}
    }
    {user: $user, ip: $ip, port: ($port | into int)}
}

def workspace-version []: nothing -> string {
    open Cargo.toml | get -o workspace.package.version | default ''
}

def image-tag []: nothing -> string {
    let sha12 = (git rev-parse HEAD | str trim | str substring 0..11)
    let version = (workspace-version)
    if ($version | is-empty) {
        error make {msg: 'deploy-window: Cargo.toml names no workspace version'}
    }
    $"($version)-($sha12)"
}

def ssh-run [tofu_dir: string, remote: string]: nothing -> string {
    let h = (host-identity $tofu_dir)
    let out = (ssh -p $h.port
        -o ServerAliveInterval=30 -o ServerAliveCountMax=6
        $"($h.user)@($h.ip)" $remote | complete)
    if $out.exit_code != 0 {
        error make {msg: $"deploy-window: remote command failed: ($out.stderr | str trim | str substring 0..($GEARS_JSON_ERROR_LIMIT))"}
    }
    $out.stdout | str trim
}

def gears-on [server: string]: nothing -> list {
    # The schedule trigger's enabled flag IS the authoritative pause
    # state (AGENTS.md): an automation fires only while its schedule
    # trigger is on; api-trigger "partial" rows do not fire on their own.
    let bin = (fabro-bin)
    let out = (^$bin automations --server $server list --json | complete)
    if $out.exit_code != 0 {
        error make {msg: $"deploy-window: automations list failed: ($out.stderr | str trim | str substring 0..($GEARS_JSON_ERROR_LIMIT))"}
    }
    let list = ($out.stdout | from json)
    $list | where {|gear| ($gear.triggers | where type == 'schedule' and enabled == true | length) > 0 }
}

def phase-pause [server: string]: nothing -> nothing {
    let bin = (fabro-bin)
    let enabled = (gears-on $server)
    mkdir tmp
    if ($enabled | is-empty) {
        print 'deploy pause: every gear is already off — nothing to pause'
        {paused: [], at: (date now)} | to json | save --force $STATE_FILE
        return
    }
    for gear in $enabled.id {
        print (^$bin automations --server $server pause $gear | complete | get stdout | str trim)
    }
    # Verify by re-GET: a pause that did not survive is no pause.
    let still_on = (gears-on $server)
    if not ($still_on | is-empty) {
        error make {msg: $"deploy-window: gears still on after pause: ($still_on.id | str join ', ')"}
    }
    {paused: $enabled.id, at: (date now)} | to json | save --force $STATE_FILE
    print $"deploy pause: ($enabled.id | str join ', ') paused and verified off"
}

def phase-build [tofu_dir: string, arch: string]: nothing -> nothing {
    git fetch origin denkhaus --quiet
    let head = (git rev-parse HEAD | str trim)
    let tip = (git rev-parse origin/denkhaus | str trim)
    if $head != $tip {
        let head12 = ($head | str substring 0..11)
        let tip12 = ($tip | str substring 0..11)
        error make {msg: $"deploy-window: HEAD ($head12) is not the pushed tip ($tip12) — push the release bump first; the parity gate refuses a tag off an undeployed sha otherwise"}
    }
    let h = (host-identity $tofu_dir)
    # host and port are SEPARATE recipe arguments — quoting them as one
    # word makes the arch land in the port slot (live 2026-11-15:
    # `scp: bad port "amd64"`).
    just host-release $"($h.user)@($h.ip)" $"($h.port)" $arch $head
}

def replace-default [text: string, variable: string, value: string]: nothing -> string {
    # Single-quoted parts concatenated: no escape processing, the regex
    # stays readable and nothing interpolates by accident. The match is
    # PROVEN by parse first — a replace onto an already-correct default
    # changes nothing, and "unchanged" is NOT "no match" (idempotency:
    # rerunning a phase must not read as failure).
    let pattern = ('(?s)(variable "' + $variable + '" \{.*?default\s*=\s*")[^"]*(")')
    let hits = ($text | parse --regex $pattern | length)
    if $hits == 0 {
        error make {msg: $"deploy-window: found no default for variable ($variable)"}
    }
    $text | str replace -r $pattern ('${1}' + $value + '${2}')
}

def phase-pins [tofu_dir: string]: nothing -> nothing {
    assert-tofu-fresh $tofu_dir
    let tag = (image-tag)
    let variables = ($tofu_dir | path join 'variables.tf')
    # Recompute, never assume: the same pin rev USUALLY means the same
    # plugin sha — verify against the image the host just built.
    let sum_line = (ssh-run $tofu_dir ('docker run --rm --entrypoint sh denkhaus/fabro:' + $tag + " -c 'sha256sum /usr/local/bin/sandbox-driver-docker'"))
    let sha = ($sum_line | parse --regex '([0-9a-f]{64})' | get -o 0.capture0)
    if ($sha | is-empty) {
        error make {msg: $"deploy-window: no sha256 in remote output: ($sum_line | str substring 0..120)"}
    }
    let text = (open --raw $variables)
    let text = (replace-default $text 'fabro_image_ref' (':' + $tag))
    let text = (replace-default $text 'petri_sandbox_docker_sha256' $sha)
    $text | save --force $variables
    print $"deploy pins: fabro_image_ref=:($tag), petri sha ($sha | str substring 0..12)… written to variables.tf"
    print 'deploy pins: the description PROSE (the window note) is written by the session — this phase touches defaults only'
}

def state-secret [tofu_dir: string]: nothing -> string {
    # The gopass path is deployment-private knowledge — it lives in the
    # PRIVATE tofu repo (envs/state-secret, one line), never in this
    # public script (user directive 2026-11-15). Same resolution pattern
    # as the host identity.
    let marker = ($tofu_dir | path join 'envs' 'state-secret')
    if not ($marker | path exists) {
        error make {msg: "deploy-window: the tofu repo names no state secret — create envs/state-secret there (one line: the gopass path)"}
    }
    let secret = (open --raw $marker | str trim)
    if ($secret | is-empty) {
        error make {msg: 'deploy-window: envs/state-secret is empty — it must name the gopass path'}
    }
    $secret
}

def tofu-env [tofu_dir: string]: nothing -> string {
    # NOTE: nu env assignments are block-scoped like cd — setting
    # TF_VAR_state_passphrase here would not survive into the calling
    # phase. This helper RETURNS the passphrase (never prints it); each
    # phase assigns the env itself. BLOCKING probe, no timeout wrapper:
    # a cold gopass hang IS the probe failing — warm the store first.
    let probe = (mise exec -- gopass show -o (state-secret $tofu_dir) | complete)
    if $probe.exit_code != 0 {
        error make {msg: 'deploy-window: gopass probe failed — warm the store named by envs/state-secret and retry'}
    }
    $probe.stdout | str trim
}

def phase-plan [tofu_dir: string]: nothing -> nothing {
    assert-tofu-fresh $tofu_dir
    $env.TF_VAR_state_passphrase = (tofu-env $tofu_dir)
    $env.TF_DATA_DIR = '.terraform-prod'
    cd $tofu_dir
    mise exec -- tofu plan -var-file=envs/prod.tfvars
}

def phase-apply [tofu_dir: string, server: string]: nothing -> nothing {
    assert-tofu-fresh $tofu_dir
    # Capture the deployed tip BEFORE the cd: after `cd $tofu_dir`, a bare
    # `git rev-parse HEAD` resolves the TOFU repo's tip and the skew check
    # compares against the wrong repository (live 2026-11-16: the fork.38
    # apply "failed" with the tofu pin sha while the server was already
    # live at the right tip).
    let head = (git rev-parse HEAD | str trim)
    # Probe AGAIN immediately before the apply: the store relocks on its
    # ~30 min TTL — warming early is not warming enough (2026-11-10).
    $env.TF_VAR_state_passphrase = (tofu-env $tofu_dir)
    $env.TF_DATA_DIR = '.terraform-prod'
    cd $tofu_dir
    mise exec -- tofu apply -auto-approve -var-file=envs/prod.tfvars
    # Never report success from version strings alone — the skew check
    # is the mechanical half; the LIVE PROOF (the shipped behavior) is
    # the human half and follows this phase.
    # The container is seconds old — wait for the server to answer
    # before the skew check (bounded: the wait outlives a legitimate
    # startup, never hangs on a dead deploy).
    let bin = (fabro-bin)
    mut deployed = ''
    let deadline = (date now) + 2min
    loop {
        let out = (^$bin version --server $server --json | complete)
        if $out.exit_code == 0 {
            $deployed = ($out.stdout | from json | get -o server.git_sha | default '')
        }
        if ($deployed | is-not-empty) { break }
        if (date now) > $deadline {
            error make {msg: 'deploy-window: the server did not answer the skew check within 2 minutes of the apply'}
        }
        sleep 5sec
    }
    # $head was captured before the cd (top of this phase) — resolving it
    # here would read the TOFU repo after `cd $tofu_dir`.
    let head12 = ($head | str substring 0..11)
    if ($deployed | str substring 0..11) != $head12 {
        error make {msg: $"deploy-window: server reports sha ($deployed) — expected the deployed tip ($head12)"}
    }
    # The container is seconds old: the projections are still warming and
    # verification reads (claims list, radius) under-report until they are
    # (2026-11-23, the b656126ab36f window: the claims list read 0 rows with
    # 166 in the table). Name the window at the exact place the operator
    # starts verifying — verification reads need a re-read minutes later.
    print $"deploy apply: server is live at ($head12) — projections are still warming after the container recreate: verification reads such as the claims list and the radius can under-report — re-read them after a few minutes"
    print "deploy apply: now the LIVE PROOF of the shipped behavior"
}

def phase-arm [tofu_dir: string, server: string]: nothing -> nothing {
    if not ($STATE_FILE | path exists) {
        error make {msg: 'deploy-window: no pause state — nothing to re-arm (run `just deploy pause` first)'}
    }
    let state = (open $STATE_FILE)
    let bin = (fabro-bin)
    for gear in ($state.paused | default []) {
        print (^$bin automations --server $server unpause $gear | complete | get stdout | str trim)
    }
    # NO toolchain pin step: ADR-0042 retired it — the toolchain
    # environment is repo-declared and cooked by the engine at run
    # initialization from the checkout itself.
    let on = (gears-on $server)
    print $"deploy arm: gears on: ($on.id | str join ', ') — no toolchain pin, ADR-0042"
}

def phase-commit [tofu_dir: string]: nothing -> nothing {
    let tag = (image-tag)
    cd $tofu_dir
    let dirty = (git status --porcelain | lines | where {|l| ($l | str substring 3..) == 'variables.tf'})
    if ($dirty | is-empty) {
        print 'deploy commit: variables.tf unchanged — nothing to commit'
        return
    }
    let msg = ('/tmp/opencode/tofu-pin-' + (date now | format date '%Y%m%d-%H%M%S') + '.txt')
    let body = ([
        'tofu pin: the live image is :' + $tag
        ''
        'The sandbox-plugin sha rode the same edit (recomputed from the'
        'new image over ssh, never assumed).'
    ] | str join "\n")
    $body | save --force $msg
    git add variables.tf
    git commit -F $msg
    print (git log -1 --format=%B | str trim)
    rm $msg
    print 'deploy commit: committed (this phase never pushes — the push policy belongs to the session)'
}

export def main [
    phase: string                                    # pause|build|pins|plan|apply|arm|commit
    --server: string = 'https://mirtuell.net'        # the production line
    --tofu-dir: string = '~/dev/fabro-tofu'          # the private deploy repo
    --arch: string = 'amd64'                         # release host arch
]: nothing -> nothing {
    let tofu = ($tofu_dir | path expand)
    match $phase {
        'pause' => { phase-pause $server }
        'build' => { phase-build $tofu $arch }
        'pins' => { phase-pins $tofu }
        'plan' => { phase-plan $tofu }
        'apply' => { phase-apply $tofu $server }
        'arm' => { phase-arm $tofu $server }
        'commit' => { phase-commit $tofu }
        _ => { error make {msg: $"deploy-window: unknown phase ($phase) — pause|build|pins|plan|apply|arm|commit"} }
    }
}
