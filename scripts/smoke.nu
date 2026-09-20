#!/usr/bin/env nu
# Post-deploy smoke test (thin launcher: `just smoke`). Regression born
# 2026-08-25: the instance was healthy and `fabro ps` roundtripped fine,
# while the SPA was dead — index.html referenced an asset the server did
# not embed (404). Health + API say nothing about the UI, so this walks
# the routes a user actually hits:
#
#   1. /health            — server up
#   2. /                  — SPA index serves AND references >= 1 asset
#   3. every referenced   — each assets/*.js / *.css the index names
#      asset                answers 200 (the exact index/asset mismatch
#                            regression)
#   4. /runs              — SPA deep route falls back to the index
#   5. <cli> ps           — authenticated API roundtrip through the
#                            installed CLI
#   6. /api/v1/automations — authenticated automations list (dev token
#                            from the CLI auth store; skipped when absent)
#
# ANY red check prints an ALARM block and exits 1, so `just up` aborts
# instead of shipping a broken instance.
#
# INSTALL MODE (fabro-b03f): a `just up` refresh that bumps the image
# version can boot the server unconfigured. Root cause: pre-2026-04
# images kept settings OUTSIDE the /storage volume (baked
# /etc/fabro/settings.toml + FABRO_CONFIG env, config at
# /var/fabro/.fabro/settings.toml in the container layer — see
# docs/superpowers/specs/2026-04-18-web-install-design.md, "Container
# packaging"); the current image looks at /storage/.home/settings.toml
# (FABRO_HOME=/storage/.home) and finds nothing, so it falls into
# install mode: /health and the SPA serve, but every /api/v1/* route
# 404s. No adoptable prior state exists in the volume, so the fix is
# documentation plus this targeted diagnostic: when the signature
# (health 200 + SPA serves + CLI API roundtrip 404) matches, the
# verdict prints an install-mode remediation block instead of the
# generic ALARM.

# One probe: {name, ok, detail}. curl via complete keeps a 404 a data
# point, not a script crash (same pattern as wait-healthy). `--html`
# sends a browser Accept header: the SPA fallback serves index.html only
# to navigations that accept HTML — a bare curl gets 404 BY DESIGN.
def probe [name: string, url: string, --html]: nothing -> record {
    let res = (if $html {
        do { ^curl -sS -m 5 -H "Accept: text/html" -o /dev/null -w "%{http_code}" $url } | complete
    } else {
        do { ^curl -sS -m 5 -o /dev/null -w "%{http_code}" $url } | complete
    })
    let status = ($res.stdout | str trim)
    {
        name: $name
        ok: ($res.exit_code == 0 and $status == "200")
        detail: $"($url) -> ($status) ($res.stderr | str trim)"
    }
}

def main [port: string = "32276", cli: string = "~/.fabro/bin/fabro"]: nothing -> nothing {
    let base = $"http://127.0.0.1:($port)"
    mut results = []

    # 1. health
    let health = (probe "health endpoint" $"($base)/health")
    let health_ok = ($health.ok)
    $results = ($results | append $health)

    # 2. SPA index + asset references
    let index = (do { ^curl -sS -m 5 $base } | complete)
    let body = ($index.stdout | str trim)
    let index_ok = (($index.exit_code == 0) and (($body | str length) > 0))
    $results = ($results | append {
        name: "SPA index serves"
        ok: $index_ok
        detail: $"GET / -> exit ($index.exit_code), ($body | str length) bytes"
    })
    let assets = (if ($body | is-empty) { [] } else {
        $body | parse --regex '(?<ref>assets/[A-Za-z0-9._-]+\.(js|css))' | get ref | uniq
    })
    $results = ($results | append {
        name: "index references assets"
        ok: (($assets | length) >= 1)
        detail: ($assets | if ($in | length) > 0 { str join ", " } else { "no assets/*.js|css referenced in index.html" })
    })

    # 3. every referenced asset answers
    for asset in $assets {
        $results = ($results | append (probe $"asset ($asset)" $"($base)/($asset)"))
    }

    # 4. SPA deep route falls back to the index (browser navigation)
    $results = ($results | append (probe "SPA deep route /runs" $"($base)/runs" --html))

    # 5. CLI API roundtrip
    let ps = (do { ^$cli ps } | complete)
    let ps_detail = ($ps.stderr | str trim | if ($in | is-empty) { "exit ($ps.exit_code)" } else { $"exit ($ps.exit_code): ($in)" })
    # Install-mode signature input (fabro-b03f): in install mode the CLI
    # roundtrip fails with a 404 from the install router (only /install
    # and static SPA routes serve).
    let install_mode_hit = ($health_ok and $index_ok and ($ps.exit_code != 0) and ($ps_detail | str lowercase | str contains "404"))
    $results = ($results | append {
        name: "CLI API roundtrip (ps)"
        ok: ($ps.exit_code == 0)
        detail: $ps_detail
    })

    # 6. Automations API answers (authenticated). Regression born
    # 2026-09-06: deploy #5 was healthy and `fabro ps` roundtripped while
    # /api/v1/automations 500'd (schema migration missing in the binary) —
    # the automations tab was dead and no probe noticed. The CLI has no
    # automations command yet (fabro-fe35), so this reads the local dev
    # token from the CLI auth store directly. No stored token for this
    # server => SKIPPED (gray), not red.
    let auth_path = ($env.HOME | path join ".fabro" "auth.json")
    let server_key = $"http://127.0.0.1:($port)"
    # `get -o` (optional) needs nushell >= 0.105; deploy hosts still run
    # 0.101 (weblab, 2026-09-09 fabro-afb8), where the flag is a parse
    # error. `get -i` (ignore-errors) exists since well before 0.101 and
    # yields the same null-on-missing shape chained with `default`.
    let token = (if ($auth_path | path exists) {
        open $auth_path
        | get -o servers
        | default {}
        | get -o $server_key
        | default {}
        | get -o token
        | default ""
    } else {
        ""
    })
    if ($token | str trim | is-not-empty) {
        let bearer = ($token | str trim)
        let res = (do {
            ^curl -sS -m 5 -o /dev/null -w "%{http_code}" -H $"Authorization: Bearer ($bearer)" $"($base)/api/v1/automations"
        } | complete)
        let status = ($res.stdout | str trim)
        $results = ($results | append {
            name: "Automations API (authenticated)"
            ok: ($res.exit_code == 0 and $status == "200")
            detail: $"/api/v1/automations -> ($status)"
        })
    } else {
        print $"smoke: - automations API probe skipped, no stored token for ($server_key)"
    }

    # verdict
    let failed = ($results | where not $it.ok)
    for r in $results {
        if $r.ok {
            print $"smoke: ✓ ($r.name)"
        } else {
            print -e $"smoke: ✗ ($r.name) — ($r.detail)"
        }
    }
    if ($failed | length) > 0 and $install_mode_hit {
        # Targeted remediation instead of the generic ALARM (fabro-b03f):
        # the server is up but UNCONFIGURED — install mode is active, so
        # only /install and the static SPA serve; every /api/v1/* route
        # 404s. A one-time re-install after a version bump is expected
        # when the prior image predates the FABRO_HOME=/storage/.home
        # layout (older images kept settings outside the /storage
        # volume, so nothing adoptable survives the refresh).
        print -e ""
        print -e "╔══ SMOKE: server is in INSTALL MODE (unconfigured) ══╗"
        print -e "║ health + SPA serve, but the CLI API roundtrip got 404 ║"
        print -e "║ — every /api/v1/* route is closed until configured.  ║"
        print -e "║                                                       ║"
        print -e "║ This is expected ONCE after a version bump refreshed  ║"
        print -e "║ the stack: prior state is not adoptable (pre-layout   ║"
        print -e "║ images kept settings outside the /storage volume).    ║"
        print -e "║                                                       ║"
        print -e "║ Remediation — run the one-time install:               ║"
        print -e "║   docker compose logs fabro                           ║"
        print -e "║     -> 'Fabro server is unconfigured — install mode   ║"
        print -e "║        active' with the install URL + token           ║"
        print -e "║   open the URL, complete the wizard, the container    ║"
        print -e "║   restarts configured; re-run `just smoke`.           ║"
        print -e "╚═══════════════════════════════════════════════════════╝"
        exit 1
    }
    if ($failed | length) > 0 {
        print -e ""
        print -e $"╔══ ALARM: ($failed | length) of ($results | length) smoke checks failed ══╗"
        for r in $failed {
            print -e $"║ ✗ ($r.name): ($r.detail)"
        }
        print -e "╚══════════════════════════════════════════════════════════════╝"
        print -e "smoke: the deployment is NOT usable — see 'docker compose logs --tail 50'"
        exit 1
    }
    print $"smoke: all ($results | length) checks green"
}
