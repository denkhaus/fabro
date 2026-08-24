#!/usr/bin/env nu
# Block until the local server answers /health (thin launcher:
# `just wait-healthy`). On timeout, dump compose ps/logs as diagnostics.

def main [
    port: string = "32276"   # server port
    --timeout(-t): int = 90   # seconds to wait
]: nothing -> nothing {
    let url = $"http://127.0.0.1:($port)/health"
    print $"wait-healthy: waiting for ($url) ..."
    mut attempt = 0
    while $attempt < $timeout {
        let res = (do { ^curl -fsS -m 2 $url } | complete)
        if $res.exit_code == 0 {
            print $"wait-healthy: healthy after ($attempt + 1) attempt\(s)"
            exit 0
        }
        sleep 1sec
        $attempt = ($attempt + 1)
    }
    print -e $"wait-healthy: server did not become healthy within ($timeout)s"
    # Best-effort diagnostics; the explicit exit 1 below stays the verdict.
    print ((do { ^docker compose ps } | complete).stdout)
    print ((do { ^docker compose logs --tail 50 } | complete).stdout)
    exit 1
}
