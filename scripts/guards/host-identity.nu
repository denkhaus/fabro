#!/usr/bin/env nu
# Host-identity guard (mulch mx-9c1e77, the 2026-11-09 leak class):
# this repository is PUBLIC — deployment host identity (production
# addresses, `root@<ip>` user@address shapes) must never ride it, not
# even as recipe defaults. betterleaks cannot catch this class: it
# scans CREDENTIAL patterns; an address is configuration. This guard
# scans the files it is handed (lefthook passes {staged_files}; the
# pre-push hook passes {push_files}) and fails listing every hit with
# file and line. rg skips binary files on its own.
#
# Patterns are deliberately NARROW (the production literal and the
# root@<ipv4> shape); RFC1918/localhost addresses in fixtures and
# tests stay allowed.

# The production host's address — never legitimately in this repo.
const PRODUCTION_IP = '152\.53\.93\.90'
# `root@` against any literal IPv4 — the SSH target shape.
const ROOT_AT_IPV4 = 'root@[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+'

def main [...files: string] {
    if ($files | is-empty) {
        print 'host-identity: nothing to scan'
        return
    }
    let patterns = [$PRODUCTION_IP, $ROOT_AT_IPV4]
    mut hits = []
    for file in $files {
        if not ($file | path exists) { continue }
        for pattern in $patterns {
            let found = (do { ^rg -n $pattern $file } | complete)
            if $found.exit_code == 0 {
                for line in ($found.stdout | lines) {
                    $hits = ($hits | append $"($file):($line)")
                }
            }
        }
    }
    if ($hits | is-empty) {
        print $"host-identity: clean \(($files | length) files\)"
        return
    }
    # One line can match two patterns (the IP inside a root@<ip>); a hit
    # reports once.
    let hits = ($hits | uniq)
    print -e 'host-identity: REFUSING — deployment host identity in a public repo:'
    for hit in $hits {
        print -e $"  ($hit)"
    }
    print -e 'host-identity: host identity is caller-supplied (fabro-tofu envs/ is the identity home); see the host-release recipe and mulch mx-9c1e77.'
    exit 1
}
