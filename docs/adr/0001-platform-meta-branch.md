# Platform work lives on a meta branch, product work on the product branch

Two worlds with two seed trackers: `denkhaus-lab` carries product code and
product seeds (`fabro-*`); `meta/denkhaus-lab` carries workflow assets,
platform seeds (`fabro-meta-*`), and the painpoint mailbox. Fabro itself
separates run content from run metadata the same way (`fabro/run/<id>` vs
`fabro/meta/<id>`); this lifts that pattern from per-run to per-project.

Rationale: run 01M0NA4J5H30CZAFZEBF7HHQX5 showed a planner diagnosing real
workflow bugs and routing the fixes through a product seed — the loop worked,
but the channel mixed platform evolution into product scope. A separate
world keeps each tracker honest and forces all platform criticism through
one channel (ADR-0002).

Consequences: the two `.seeds/` stores are NEVER synced or merged across
branches (merge=union corruption was observed live when run branches were
merged back). Each run reads exactly one tracker in its own checkout.
