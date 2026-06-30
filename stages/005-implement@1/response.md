Implemented the MCP servers settings UI under `apps/fabro-web/app/` only.

Summary:
- Added `/settings/mcps`, `/settings/mcps/new`, and `/settings/mcps/:id/edit`.
- Added MCP list/create/edit pages with CRUD API usage, revision/If-Match handling, delete confirmation, stale 409 messaging, and settings nav link.
- Added shared `KeyValueEditor` extraction and reused it in environments + MCP forms.
- Added MCP form model helpers, validation, read-model-to-form mapping, write-only value guard, and credential warning/nudge flow.
- Added MCP query keys, SWR hooks, and `MCPServersApi` wiring.
- Added tests for credential heuristics, key/value editor, MCP form helpers, list/new/edit route smoke behavior, query keys, and router registration.
- Did not touch Rust, OpenAPI, generated clients, or docs.

Validation:
- `cd apps/fabro-web && bun run typecheck` ✅
- `cd apps/fabro-web && bun test` ✅ — 650 pass
- `cd apps/fabro-web && bun run build` ✅
- `cd lib/packages/fabro-api-client && bun run typecheck` ✅

The implementation follows the requested write-only edit behavior: existing env/header keys are shown with empty values, the edit banner explains full replacement, and saving is blocked on edit until every non-empty key has a value.