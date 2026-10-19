Exact command reference (never invent flags):

| Command | Purpose |
|---|---|
| `seeds show <id> --format json` | One seed in full (the supported path — never parse `.seeds/issues.jsonl` by hand). JSON with no `description` key IS the empty body; do not re-probe the seed in other formats to confirm it. |