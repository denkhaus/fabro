# ADR-0033: English-only written artifacts (chat stays German)

- Status: accepted (user decision, 2026-10-24)
- Deciders: user + agent
- Related: AGENTS.md "Communication conventions", ADR-0032 (central work
  tracker), fabro-6041 (test-rig bench), `iterate` and `integrate` skills
  (review gates)
- Supersedes: the implicit habit "the file's language is the convention"

## Context

The line's convention has been "chat in German, written artifacts in
English" since the beginning, recorded as a single bullet in `AGENTS.md`.
In practice the rule kept eroding at the edges: an artifact written in a
German file (the fabro-test bench catalog `FACETS.md`, its `README.md`)
was extended in German because the surrounding text was German. The
result was mixed-language files — a German paragraph inside a catalog
whose rows were being filled with English text, plus new German rows next
to legacy German rows.

The user closed the ambiguity on 2026-10-24: "alles geschriebene muss
english sein" — otherwise "entsteht ein ungewolltes sprachwirrwarr".
Matching the file language is NOT a valid reading of the convention. A
one-line rule that lives in a single repository's `AGENTS.md` did not
travel: the violating work happened in a sibling repo (fabro-test), and
the agent read the file's language as the stronger local signal.

## Decision

1. **Chat replies are German. Every written artifact is English.** The
   split is by channel, not by file, repo, or habit.
2. **Scope is the whole line, every repository** (fabro, fabro-test,
   fabro-tofu, and any future line repo): docs, ADRs, READMEs, catalogs
   (`FACETS.md`-style), code comments, workflow prompts and prompt
   comments, seed titles/descriptions/notes, commit messages, PR
   titles/bodies, review notes, tracker comments.
3. **File language never licenses drift.** A German file does not permit
   German additions: add English, and translate the whole file in the
   same change. Mixed-language files are forbidden. A translated artifact
   is translated as a whole file, never partially.
4. **One exemption:** test DATA whose purpose is non-English model
   output (for example a prompt that deliberately asks the model to
   answer in German to prove a language behavior). Exemptions are named
   in the artifact, not assumed.
5. **Enforcement lives in the review gates.** The `iterate` skill checks
   language in Phase 3 (review) and before commiting in Phase 5
   (integrate); the `integrate` skill checks it inside the docs axis of
   its six-axis review. A German artifact in a reviewed diff is a
   finding, not a nit. New repositories carry the rule in their own
   `AGENTS.md` and README at creation time.

## Consequences

- Agents and humans get one rule that reads the same in every repo, so
  the "match the file" failure mode has no foothold.
- New German text is a review finding; legacy German files get
  translated when touched (whole file, one change) — the backlog of
  legacy German text shrinks by contact instead of by decree.
- The rule is greppable (`ADR-0033`, `written language`) and citable in
  seeds, reviews, and commit messages.
- Test data that intentionally exercises German output stays German and
  is marked as such, so a future audit does not "fix" it.

## Alternatives considered

- **Per-repo language (match the file).** Rejected: produces exactly the
  mixed-language jumble that triggered this ADR, and makes the rule
  depend on history rather than on a decision.
- **English everywhere including chat.** Rejected by the user: German
  chat is wanted; the split by channel is the point.
- **Rule only in the fabro repo's `AGENTS.md`.** Rejected: the rule did
  not travel to sibling repos, which is where the violation happened.
