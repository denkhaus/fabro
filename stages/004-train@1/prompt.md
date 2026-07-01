You are running a **merge train**. Read `merge-train-plan.md` for the ordered
list of READY PRs, the base branch (input `base`), and the merge method (input
`merge_method`, `squash` or `merge`). Process the READY PRs **strictly in order**,
one at a time. Each PR is rebased onto the result of the previous merge, so the
train validates every PR against the cumulative state of `base`.

**Your job is to get every queued PR green and merged.** Do everything in your
power to make each PR mergeable — rebase it, resolve conflicts, run the tests, and
fix whatever is broken — and only escalate to a human when a decision genuinely
requires human judgement. If every PR can be made green, the end state is that
they are all merged.

## Resuming

You may be re-entered after a human answered the escalation gate. On entry:
1. Read `merge-train-state.md` to see which PRs are already `MERGED` / `SKIPPED`
   and which one you were stuck on.
2. If a human answer is present (context key `human.gate.human_gate.answer`), apply
   that guidance to the PR you were stuck on. If the human said to **stop**, finalize:
   write the state file, return outcome `succeeded`, and let the report stage run.
3. Otherwise continue from the first PR that is not yet resolved.

Keep `merge-train-state.md` current as you go (one line per PR: `MERGED` /
`SKIPPED` / `BLOCKED` / `IN PROGRESS` + a short reason) so progress survives an
interruption or hand-off to a human.

## Per-PR procedure

For each unresolved READY PR number `N`, in order:

1. **Sync base:** `git fetch origin main`.
2. **Check out the PR branch:** `gh pr checkout N`.
3. **Rebase onto the latest base:** `git rebase origin/main`.
   - Clean → continue.
   - Conflicts → **resolve them**: read the conflicting hunks, make the correct
     merged edits (preserve the intent of BOTH sides — never blindly take one),
     `git add -A`, `git rebase --continue`; repeat until the rebase completes.
   - If a conflict is a genuine **semantic** conflict where the right resolution is
     ambiguous or is a product/API judgement call, `git rebase --abort` and
     **escalate** (see "When to escalate").
4. **Run the project's tests LOCALLY, before pushing.** Do not push-and-pray.
   - If input `verify_cmd` is set, run exactly that. Otherwise discover what CI
     runs (read `.github/workflows/*` and `CLAUDE.md`) and run the same checks
     locally — e.g. for this repo: `cargo +nightly-2026-04-14 fmt --check --all`,
     `cargo +nightly-2026-04-14 clippy --workspace --all-targets -- -D warnings`,
     `cargo nextest run --workspace`, and the web `typecheck`/`test`. Running what
     CI runs makes a local pass predict a CI pass.
   - **If local checks fail, FIX them** (up to `max_fix_attempts` attempts): read
     the failure, make the code change, re-run the checks. Commit the fixes onto
     the PR branch (`git commit`) so they become part of the PR.
   - If you exhaust `max_fix_attempts` without going green, or the fix needs a
     product decision, **escalate**.
5. **Push the rebased, fixed branch:** `git push --force-with-lease origin HEAD`.
   (The rebase rewrote history, so a force push is required; `--force-with-lease`
   refuses to clobber unexpected upstream changes.)
6. **Wait for CI on the pushed head, then require green.** The push starts a fresh
   CI run, so do not trust pre-push results.
   - Note the head SHA (`git rev-parse HEAD`), then poll `gh pr checks N` every ~15s
     until it reports check runs for that head (wait up to ~2 min for them to
     appear). Then block on `gh pr checks N --watch --interval 30` until they
     finish. Do NOT pass `--required` (nothing is marked required here, so it would
     ignore every check).
   - **All green →** merge (step 7).
   - **A check fails →** pull the failing logs (`gh run view --log-failed` / the
     check's output), reproduce and **FIX locally** (this is why we test locally
     first — CI should rarely surface something new), re-run local checks, re-push,
     and wait again. Bounded by the same `max_fix_attempts`. If still red after
     that, **escalate** with the failing check name and a short diagnosis.
   - **No CI configured →** if, after the ~2 min window, the PR reports no check
     runs at all, there is nothing to wait for; proceed to merge.
7. **Merge:** `gh pr merge N --squash`.
   - Success → mark **MERGED**.
   - **If GitHub rejects the merge** (e.g. HTTP 405 / "review required" / "required
     status checks" / "at least 1 approving review"), do **NOT** retry, force, or
     alter branch protection. This is the branch-protection wall: the base requires
     a review/check the Fabro app cannot satisfy because it is not a ruleset bypass
     actor, or `require_last_push_approval` invalidated an approval after your
     force-push. Record the exact GitHub message and **escalate**.
8. **Advance the base:** `git checkout main && git pull --ff-only origin main`
   so the next PR rebases onto the just-merged result. Then go to the next PR.

## When to escalate (human gate)

Escalate — do not guess — when you hit any of:
- a semantic/ambiguous conflict whose correct resolution needs human intent;
- test or CI failures you could not fix within `max_fix_attempts`;
- a product, API-contract, or behaviour decision;
- a merge GitHub rejects for branch-protection reasons.

To escalate: write the **specific question and the current situation** for the
stuck PR into `merge-train-state.md` (mark it `BLOCKED — NEEDS HUMAN: <question>`),
then return the routing outcome with `preferred_label = "NeedsHuman"`. Do **not**
skip past the stuck PR to later ones — the train is ordered. The human's answer
comes back on the next entry (see "Resuming").

## Rules

- **Merge everything you can; escalate the rest — never freeze silently and never
  bypass protection.** Do not force-merge, disable checks, or edit rulesets/branch
  protection, and never push directly to `main`.
- Prefer `--force-with-lease` over `--force`. Never touch branches outside this train.
- Pending/running checks are not failures — wait for them.

## Output (routing)

Return a routing outcome:
- **`outcome = succeeded`** when every queued PR is resolved (MERGED or SKIPPED) or
  the human told you to stop. This routes to the report stage.
- **`preferred_label = "NeedsHuman"`** when you are stuck and need a decision. This
  routes to the human gate; you will be re-entered with the answer.

Include a concise per-PR summary (MERGED / SKIPPED / BLOCKED + reason) and the
final `main` SHA in your response.