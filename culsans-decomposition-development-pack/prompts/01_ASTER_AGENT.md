# Codex Prompt — Aster Extraction Agent

Workspace: `D:\Projects\aster`

Source reference: read-only access to `D:\Projects\culsans`.

Read:

- `README.md`
- `docs/01_TARGET_ARCHITECTURE.md`
- `docs/02_PARALLEL_EXECUTION.md`
- `docs/03_EXTERNAL_APP_PROTOCOL_V1.md`
- `docs/05_ASTER_PLAN.md`
- `docs/08_DATA_MIGRATION.md`
- `docs/09_TEST_AND_ACCEPTANCE.md`
- `docs/12_CODEX_OPERATING_RULES.md`

Mission:

Build Aster as a fully independent File Search product by migrating the existing Culsans File Search behavior.

Before editing:

1. inspect current Culsans File Search source, tests, routes, commands, settings, Everything integration and packaging;
2. write `docs/MIGRATION_MAP.md` in Aster listing source ownership and destination ownership;
3. record the Culsans baseline SHA.

Then execute the Aster tickets in order.

Hard rules:

- no dependency on Culsans source;
- no shared Cargo/pnpm workspace;
- Aster owns Everything resources/lifecycle;
- Aster owns its settings;
- Culsans is not Aster's runtime;
- preserve File Search behavior during extraction;
- implement activation v1;
- keep local tests green;
- do not run unrelated full-suite tests.

Definition of Done:

Aster builds/runs/tests with `D:\Projects\culsans` temporarily unavailable, and Culsans can remove its File Search implementation.
