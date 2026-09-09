# Revision — run 01M22VHABGCNWT7DKJC9HP1XKC

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M22VHABGCNWT7DKJC9HP1XKC.md
- seeds filed: fabro-a341 — Align the unemitted_allow_keys lint with read_context_key's context.-prefix tolerance
- basis: run 01M22VHABGCNWT7DKJC9HP1XKC, workflow version 14ab451ff2f9375f4927ce7d994c63bc96f48f3c617f3baff15119472f3af3e4, commit a546610719fbe1058f9f820d9eee65d03ed681e4
- revised_at_commit: a546610719fbe1058f9f820d9eee65d03ed681e4 (ADR-0015: engine drift signal for later judgement)

## Findings

### Align the unemitted_allow_keys lint with read_context_key's context.-prefix tolerance
- filed: fabro-a341
- The stage-envelope allow-keys lint (seed fabro-8bf4, `lib/components/fabro-workflow`) matches raw keys only for the unemitted check, while `read_context_key` also accepts `context.`-prefixed forms; reviewer@1 flagged the divergence as a future watch item. Normalize the lint's key matching to the reader's semantics so prefixed-form emissions are not reported unemitted and the code paths cannot drift. Complements open seed fabro-8bf4 (cross-referenced, not superseded — different concrete change). Expected effect: no false-positive completion warnings once the fabro-8bf4 lint merges; one shared key-normalization rule.
