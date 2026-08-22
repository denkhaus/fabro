# ADRs and CONTEXT.md are per-world, not synced

Architecture ADRs and the CONTEXT glossary live canonical on the world they
describe: platform decisions (ADR-0001..0005, dev-loop vocabulary) on
`meta/denkhaus-lab`, product decisions (gofib domain) on the product
branch. Each world keeps its own ADR numbering starting at 0001. The
product CONTEXT.md carries a pointer to this branch for platform vocabulary
— a reference, never a copy, so the two glossaries cannot drift into the
mixed state that motivated this split (the two-world work itself was
documented identically on both branches for a day before this ADR).

This extends ADR-0003: the sync scope is workflow assets and reviewer code
only; documentation of decisions follows the world that owns them.
