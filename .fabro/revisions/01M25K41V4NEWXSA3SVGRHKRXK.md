# Revision — run 01M25K41V4NEWXSA3SVGRHKRXK

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M25K41V4NEWXSA3SVGRHKRXK.md
- seeds filed: fabro-9837 — Reviewer blob refs: state payload byte size in the marker; make a bounded nu substring read the mandatory first action
- basis: run 01M25K41V4NEWXSA3SVGRHKRXK, workflow version e825d8c827ce15defe0aa57d3586b0f369cab48cc7fe711d526776762f442aa1, commit 61d04c38f89a54168e02a12c4b1f82b0dc9c026b
- revised_at_commit: 61d04c38f89a54168e02a12c4b1f82b0dc9c026b (ADR-0015: engine drift signal for later judgement)

## Findings

### Reviewer blob refs: state payload byte size in the marker; make a bounded nu substring read the mandatory first action

- filed: fabro-9837
- supersession: closed fabro-a0fe (revision-labeled, @fabro) — its read_file offset/limit mechanism is exactly what truncated in this run
- change: (1) engine blob-ref/demotion marker states payload byte size; (2) rewrite LARGE VALUES paragraph in `.fabro/workflows/develop/prompts/reviewer.md` to mandate `nu -c 'open --raw <path> | str substring 0..20000'` as the first action before any read_file
- expected effect: one-shot evidence reads; removes the unread-blob verification-blocked bounce (~2–4 min plus a second reviewer visit) and the reviewer's only wasted tool call of this run
