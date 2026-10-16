# One-command install-mode recovery (fabro-828c): print the install URL +
# token from the running container's logs (the server logs the full URL on
# boot in install mode, before /api/v1/* exists). Open it, finish the
# wizard; the container exits and its restart policy reboots it
# configured, then `just smoke` goes green. Thin-wrapped by the justfile
# `install-url` recipe.

def main [] {
    # docker absent / daemon unreachable lands in the same guidance branch
    # the old bash body's `2>/dev/null || true` produced.
    let logs = (try { do { ^docker compose logs --no-color fabro } | complete } catch { { stdout: '', stderr: 'docker unavailable' } })
    let rows = ($logs.stdout | lines | parse --regex '(?<url>https?://[^ ]+/install\?token=[A-Za-z0-9_-]+)')
    if ($rows | is-empty) {
        print -e 'No install URL found in container logs — the server may already be'
        print -e 'configured, or it is not running. Check: docker compose ps; just logs'
        exit 1
    }
    print ($rows | last).url
}
