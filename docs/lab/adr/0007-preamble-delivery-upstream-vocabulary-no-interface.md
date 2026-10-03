# Preamble delivery: upstream vocabulary, no interface, aggregate budget + deny-list

The context-size work is named in upstream's own terms and shaped as a
minimal extension of the existing mechanism, not a new subsystem: OFFLOAD
(persist a >100KB value into the content-addressed blob store), DEMOTE
(replace an inline preamble value with a preview+path marker,
`PROMPT_INLINE_VALUE_MAX` = 8KB per value), BLOB REF / ARTIFACT POINTER
(the references), OUTCOME (the upstream stage-result type, already captured
to `stages/{rank:03}-{node}@{visit}/response.md` + `status.json` on the
metadata branch), MATERIALIZE (placing a file into the worktree, which
demote already does). The invented vocabulary — persistor, handle, handle
delivery, `.fabro/outcomes/` directory, strategy trait with selectable
implementations — is retired. Code research killed the interface: the 8KB
demote was always active and our reviewer's 25.6k preamble grew past it
because the pass checks values INDIVIDUALLY (nine ~3KB responses sum past
any per-value threshold while each stays under it) and because summary:high
renders sub-8KB LLM responses as full blockquotes. The fix therefore adds
an aggregate pass at the one existing call site (`demote_large_values_for_prompt`
via `lifecycle/fidelity.rs`): sum serialized sizes, and while over a 16KB
budget demote largest-first with a 1KB floor (contract keys and enums stay
inline for free — no keep-inline list). Thematic selection comes from a
per-node deny-list attribute `preamble_stages_ignore = "planner,refiner"`
that filters stage-history sections render-only before `build_preamble`;
the context store stays untouched, so routing, `response.*` keys, and other
consumers are unaffected, and nonexistent stage refs warn (retry_target_exists
precedent: warnings never block a run). Allow-lists invert the default from
pass-through to opt-in and are deliberately deferred. Posture toward the
maintainer is fork-first: develop on our branch, verify on the lab loop, PR
upstream as an offer; adopt the maintainer's variant only if it is better,
never gate dev pace on upstream acceptance.
