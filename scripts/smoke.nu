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
#
# ANY red check prints an ALARM block and exits 1, so `just up` aborts
# instead of shipping a broken instance.

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
    $results = ($results | append (probe "health endpoint" $"($base)/health"))

    # 2. SPA index + asset references
    let index = (do { ^curl -sS -m 5 $base } | complete)
    let body = ($index.stdout | str trim)
    $results = ($results | append {
        name: "SPA index serves"
        ok: (($index.exit_code == 0) and (($body | str length) > 0))
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
    $results = ($results | append {
        name: "CLI API roundtrip (ps)"
        ok: ($ps.exit_code == 0)
        detail: ($ps.stderr | str trim | if ($in | is-empty) { "exit ($ps.exit_code)" } else { $"exit ($ps.exit_code): ($in)" })
    })

    # verdict
    let failed = ($results | where not $it.ok)
    for r in $results {
        if $r.ok {
            print $"smoke: ✓ ($r.name)"
        } else {
            print -e $"smoke: ✗ ($r.name) — ($r.detail)"
        }
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
