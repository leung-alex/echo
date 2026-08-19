# Echo — Complete Engineering Ownership Development Pack

This is the **self-contained continuation pack** for the already extracted `Echo` repository.

It does not require the old decomposition package to understand the current task.

## Current status

Product/code extraction through stage 05 is treated as complete and is **not to be redone**.

The active work is:

```text
P06 Test Ownership Extraction
P07 Go Repository Tooling / Developer Experience Extraction
```

The objective is to finish **engineering ownership** so `Echo` is not merely a copied codebase, but an independently developable, testable, packageable repository with the same class of developer ergonomics previously available in Culsans.

## Start here

For the active Codex session, read:

```text
prompts/EXECUTE_ECHO_P06_P07.md
```

Then execute the tickets in dependency order without waiting for per-ticket confirmation.


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

