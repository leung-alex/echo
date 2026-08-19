# EXECUTE — Echo Engineering Ownership P06/P07

You are continuing an already completed product extraction.

Repository: the current `Echo` repository.
Legacy reference: Culsans may be read only when needed.

Do **not** redo stages 01–05.

Read, in order:

1. `README.md`
2. `docs/01_COMPLETED_BASELINE.md`
3. `docs/02_LOCKED_ARCHITECTURE_RULES.md`
4. `docs/06_TEST_OWNERSHIP.md`
5. `docs/07_GO_REPOSITORY_TOOLING.md`
6. `docs/08_ACCEPTANCE_AND_HANDOFF.md`

Then execute the tickets under `tickets/` in filename order.

Before coding:

- inspect the current repository state,
- record current HEAD,
- inspect the relevant legacy Culsans test/tooling assets,
- confirm no local changes would be overwritten,
- create/update `docs/TEST_OWNERSHIP_MAP.md`.

Hard constraints:


# Global Locked Rules

These rules are mandatory for this repository.

1. This repository is a fully independent product.
2. Do not introduce source dependencies on Culsans, Aster, Iris, Echo, or any sibling repository.
3. Do not introduce a parent Cargo workspace or pnpm workspace spanning sibling repositories.
4. Do not introduce npm `file:../...`, Cargo `path = "../..."`, Git submodules, shared mutable databases, or shared mutable runtime state.
5. Small duplicated helpers are acceptable. Do not create a shared `common` repository during this phase.
6. Culsans may be read only as legacy reference material. It must not be edited from this agent.
7. Do not add new product features or redesign UI while finishing engineering ownership.
8. Preserve product behavior while migrating tests/tooling.
9. Every legacy test must have an explicit ownership decision; no silent deletion.
10. Product acceptance must run without starting Culsans.
11. Culsans should later discover this product the same way it discovers any normal installed Windows application. No new Culsans-specific coupling is required here.
12. Existing Culsans-specific activation compatibility from earlier extraction work is not a P06/P07 completion criterion. Do not expand that surface in this phase.
13. Root repository tooling must only mutate files/processes/data owned by this repository.
14. The repository must still build/test if sibling source directories are temporarily unavailable.


Do not stop after scaffolding. Finish the active stages, run the required gates, create focused commits, and write `docs/P06_P07_HANDOFF.md`.

Only stop for a real architecture blocker, destructive action requiring user approval, or an external system test that requires explicit authorization.
