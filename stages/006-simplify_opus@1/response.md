All gates pass. Here's a summary of the review and what I fixed.

## Review outcome

Three parallel agents reviewed the MCP settings UI change (reuse, quality, efficiency). The **efficiency review found nothing actionable** — regexes are module-level constants with no backtracking risk, SWR keys are correct, no leaks, and form helpers run in lazy `useState` initializers rather than per-render. The reuse and quality reviews surfaced real cleanups.

## Fixes applied

**Dead code removal**
- Deleted the exported `credentialWarnings()` function and `CredentialWarning` type from `mcp-server-form.tsx`. It was only consumed by its own test — the actual UI computes warnings inline in `EntryHint` via `looksLikeCredential`. Removed the corresponding test so the two derivations can't drift.

**Clearer abstraction / less parameter sprawl**
- Replaced the brittle `requireWriteOnlyValues = lockId && lockTransport` proxy with an explicit `isEdit` prop on `McpServerFormFields`. The new and edit pages already know their mode.
- Removed the redundant `setEntryValue` mutation path that was threaded four component layers deep. The "Store as secret" button now mutates through an `onStoreSecret` closure built from the `KeyValueEditor`'s existing `onChange`. `TransportFieldsProps` shrank to `{ values, patch, isEdit }`.

**De-duplication**
- Extracted the byte-for-byte duplicated `Label` component into `settings-panel.tsx` and reused it from both `mcp-server-form.tsx` and `environment-form.tsx` (also dropped a now-unused `ReactNode` import).
- Unified the near-identical stdio/sandbox command field into a shared `CommandRow`.

**Tighter typing**
- Derived `McpTransportKind` from the generated client (`McpTransportView["type"]`) instead of re-declaring the string union, so a new transport variant becomes a compile error here.
- Removed the unnecessary `McpHttpProtocolValue` type alias — `McpHttpProtocol` already serves as both value and type.
- The transport `<select>` now maps over `MCP_TRANSPORT_KINDS` instead of hardcoded `<option>`s, collapsing one of three drift-prone copies of the kind list.

**Minor**
- Removed the redundant inner `key={server.revision}` on the edit form (the parent already fully remounts on revision change).

## Not changed (deliberately skipped)
- Hoisting `MENU_ITEM_CLASS`/`NEW_BUTTON_CLASS` into shared `ui.tsx` — these follow an existing repo-wide per-file convention across 6+ files; changing it here would be inconsistent scope creep.
- Sharing `staleAwareMessage` across two route files — two short, page-specific strings; extracting a noun-parameterized helper is marginal over-abstraction.
- `hasWriteOnlyValues`/`transportSummary` transport switches — small, clear, and operate on the server view; not worth a forced abstraction.

Verification: `bun run typecheck` clean, `bun test` 649 pass / 0 fail, `bun run build` succeeds within budgets, and the untouched `fabro-api-client` still typechecks. No Rust files touched.