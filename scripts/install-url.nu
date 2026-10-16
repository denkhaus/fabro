#!/usr/bin/env nu
# One-command install-mode recovery (thin target of `just install-url`):
# print the install URL + token from the running container's logs — the
# server logs the full URL on boot in install mode, before /api/v1/*
# exists. Open it, finish the wizard; the container exits and its restart
# policy reboots it configured, then `just smoke` goes green.

def main [] {
    # --no-color keeps the URL regex ANSI-free; docker stays an external.
    # try/complete: a missing/unreachable docker degrades to "no URL"
    # (same failure surface as the old bash `|| true`), never a nu error.
    let logs = try {
        (^docker compose logs --no-color fabro | complete)
    } catch {
        { stdout: "", stderr: "", exit_code: 127 }
    }
    let matches = ($logs.stdout | lines | parse --regex '(?<url>https?://[^\s]+/install\?token=[A-Za-z0-9_-]+)')
    let url = ($matches | get -o url | last | default "")
    if $url == "" {
        print -e "No install URL found in container logs — the server may already be"
        print -e "configured, or it is not running. Check: docker compose ps; just logs"
        exit 1
    }
    print $url
}
