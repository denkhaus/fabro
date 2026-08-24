#!/usr/bin/env nu
# Install the staged release binary as the user CLI (thin launcher:
# `just install-cli <staged> <cli_bin>`). Skips the copy when the staged
# binary is byte-identical to the installed one.

def main [staged: path, cli_bin: path]: nothing -> nothing {
    if not ($staged | path exists) {
        print -e $"install-cli: no staged binary at ($staged) — run 'just build-binary' first"
        exit 1
    }
    mkdir ($cli_bin | path dirname)
    # cmp exits 0 only for identical files; a missing cli_bin (exit 2) means
    # "install it".
    let identical = ((do { ^cmp --silent $staged $cli_bin } | complete).exit_code == 0)
    if $identical {
        print $"install-cli: CLI already up to date: ($cli_bin)"
    } else {
        ^install -m 0755 $staged $cli_bin
        ^$cli_bin --version
    }
}
