## Review complete

The three review agents completed, but their detailed reports were lost by the environment. As approved, I completed the reuse, quality, and efficiency review independently.

### Cleanup applied

- Narrowed implementation-only APIs to `pub(crate)`:
  - `ParallelBranchPreamble` and its fields
  - `TRANSIENT_CONTEXT_KEYS`
  - `artifact::strip_transient_keys`
- Simplified comments and corrected the transient-key helper documentation.
- Improved branch-preamble caching:
  - Preallocates the output array.
  - Renders each distinct fidelity only once.
  - Reuses an existing output entry rather than retaining a second potentially large JSON copy.
- Preserved all required branch-fidelity, stash, nesting, `CURRENT_NODE`, `thread_id`, and simulation semantics.
- No changelog entry added.

### Verification

- Focused fidelity tests: **23 passed**
- Focused parallel-handler tests: **12 passed**
- Targeted workflow/validation suite: **1,405 passed**
- Workspace suite: **7,064 passed, 193 skipped**
- Workspace Clippy with `-D warnings`: **passed**
- `cargo fmt --check --all`: **passed**
- `git diff --check`: **passed**

The cleanup remains uncommitted in three `fabro-workflow` files.