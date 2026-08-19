# Echo — Complete Engineering Ownership Master Plan


---

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


---

# Echo Completed Baseline (Stages E01–E05)

Treat product/code extraction as complete:

- Echo owns Clipboard platform/listener lifecycle.
- Echo owns representations, normalization, dedup/fingerprint and sensitive-source policy.
- Echo owns storage, blobs, reconciliation/GC, History, Favorites and Snippets.
- Echo owns Quick Insert retrieval/insertion orchestration and paste-target handling.
- Echo owns its own data directory/database/blob root.
- Echo runs as an independent Tauri process and can hide UI while its clipboard listener continues.
- Echo does not depend on Culsans source/runtime/storage/platform crates.
- Rust/domain tests exist across clipboard/storage/library/quick-insert.
- Current frontend test coverage is weaker than Culsans higher-level UI/system acceptance.

Do not add unrelated Echo product features during P06/P07.


---

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


---

# Echo P06 — Test Ownership Extraction

## Legacy Culsans assets to inspect

Primary candidates:

```text
tests/e2e/clipboard-first-open.spec.ts
tests/e2e/clipboard-system.spec.ts
tests/e2e/clipboard-visual.spec.ts
tests/e2e/quick-insert.spec.ts
```

Inspect imported helpers/fixtures and move only Echo-owned parts.

## Required confidence areas

- first open,
- listener active while UI hidden,
- text clipboard,
- HTML/RTF behavior,
- image clipboard,
- file-list clipboard,
- dedup/fingerprint,
- sensitive-source policy,
- blob storage,
- retention/GC/reconciliation,
- History,
- Favorites,
- Snippets,
- Quick Insert list/search,
- Copy,
- Insert/paste,
- invalid/closed paste targets,
- hide/reopen,
- single-instance behavior,
- legacy-data migration,
- packaging smoke.

## Boundary

Culsans Input Editor tests remain in Culsans if they prove Input Editor/shell behavior.

Echo owns only the receiving/persistence side of any snippet handoff.

## Frontend tests

The current frontend `test` script is effectively typechecking. Add real component and/or Playwright UI coverage so the destination confidence level is not weaker than the old Culsans tests.

Create `docs/TEST_OWNERSHIP_MAP.md`.


---

# P07 Shared Go Repository Tooling Requirements

## Goal

Recreate the **developer-experience class** of Culsans `cu.cmd + tools/cu`, but as a completely independent repository-local tool.

Do **not** share Go source between products.

## Bootstrap

Create a root `.cmd` public entry and a repo-local Go module. Keep the useful Culsans bootstrap pattern:

1. locate repository root,
2. validate a pinned Go version (default to the same `go1.26.2` baseline used by Culsans unless the repository has a documented reason to change it),
3. hash Go tool source inputs,
4. compile/cache the tool by source hash,
5. copy the cached executable into a per-run isolated session,
6. forward arguments unchanged,
7. clean the transient session executable.

The compiled repository tool is an implementation detail; the root `.cmd` is the public entry.

## Minimum public commands

- `help`
- `install`
- `format [--check]`
- `verify [--changed-from <sha>] [--profile <developer|ci>] [--explain]`
- `self-check`
- `build [--release]`
- `dev`
- `smoke`
- `acceptance <owner>`
- `package [--dir]`
- `release-candidate`
- `sync [--all | --branch <name>]`
- `clean`

Add `benchmark` only where the product already has a meaningful benchmark.

## Changed-owner verification

`verify --changed-from` must plan affected owners from Git changes, then run only relevant gates.

It must support:
- deterministic owner mapping,
- `--explain`,
- developer vs CI profile,
- validation that unknown/ambiguous changed paths fail safely rather than silently skipping coverage.

## Self-check

At minimum verify:
- root command/bootstrap integrity,
- Go tool tests,
- local-only paths,
- no sibling path dependency,
- expected manifests/scripts,
- owner graph consistency,
- generated-file drift if applicable,
- package/installer inputs,
- P06 test ownership map completeness.

## Tool self-tests

The Go tool itself must pass:

```text
go test ./...
go vet ./...
```

Add tests for dispatch, flags, repo-root resolution, changed-owner planning, command construction, independence guards, and no sibling mutation.



# Echo-specific P07 Requirements

Public entry:

```text
echo.cmd
```

Go tool root:

```text
tools/echo/
```

Required focused commands:

```text
echo.cmd verify clipboard
echo.cmd verify quick-insert
echo.cmd acceptance clipboard
echo.cmd acceptance quick-insert
```

Recommended owners:

```text
clipboard
storage
library
quick-insert
desktop
frontend
migration
installer
tests
tooling
```

`benchmark` is optional in P07 unless a meaningful existing benchmark is migrated.

Do not copy unrelated Culsans Browser/Input/Capture/Everything tooling.


---

# Echo Engineering Ownership Acceptance

## Completion definition

`Echo` is engineering-owned only when:

1. product-owned unit/component/integration/system/performance tests live in this repository,
2. those tests launch `Echo` directly,
3. no required product confidence still depends on Culsans test harnesses,
4. the repository has its own Go orchestration entry,
5. the Go tool can install, verify, build, run, smoke-test, acceptance-test and package `Echo`,
6. changed-owner verification is functional,
7. tool self-tests pass,
8. sibling repositories are unnecessary to build/test/package.

## Handoff artifact

Create:

```text
docs/P06_P07_HANDOFF.md
```

It must list:

- source baseline inspected,
- test ownership map,
- migrated/rewritten/replaced tests,
- tests intentionally staying in Culsans,
- new root command surface,
- Go owner graph,
- executed gates and results,
- Culsans legacy tests now safe to delete,
- Culsans `cu` product-specific gates now safe to delete,
- remaining blockers,
- final commit SHA.

Do not delete Culsans files in this agent.
