# ADRs and CONTEXT.md are per-world, not synced

Architecture ADRs and the CONTEXT glossary live canonical on the world they
describe: platform decisions (ADR-0001..0005, dev-loop vocabulary) on
`meta/denkhaus-lab`, product decisions (gofib domain) on the product
branch. Each world keeps its own ADR numbering starting at 0001. The
product CONTEXT.md carries a pointer to this branch for platform vocabulary
— a reference, never a copy, so the two glossaries cannot drift into the
mixed state that motivated this split (the two-world work itself was
documented identically on both branches for a day before this ADR).

This extends ADR-0003: platform-only assets on meta are the reviewer-agent
(`.prime/agent/skills/`), the engine docs snapshot (`docs/fabro/`), and the
review reports (`docs/reviews/`) — the platform reviews the product, so
review tooling and its output live where the review happens, not in the
scanned world. Product reviews are written with meta's reviewer code and
stored on meta with a header naming the scanned product commit.
