# Echo Test Ownership Map

## Echo-Owned Layers

| Layer | Location | Coverage |
| --- | --- | --- |
| Engine unit | `crates/echo-engine/src` | Ingestion races, normalization, fingerprinting, History/Saved Items contracts, Quick Insert actions, and target-session policy. |
| Storage adapter | `crates/echo-storage/src` | SQLite schema, FTS/blob persistence, deduplication, migration, saved-item snapshots, and blob reconciliation. |
| Windows adapter | `crates/echo-windows/src` | Clipboard formats, native listener, target capture, focus validation, clipboard writes, and paste delivery. |
| Activation | `crates/echo-activation/src` | Echo envelope encoding, validation, and `--echo-activate` parsing. |
| Browser UI | `tests/ui/ui.spec.ts`, `tests/ui/visual.spec.ts`, `tests/ui/echo-fixture.ts` | History, Favorites, search, copy, insert recovery, settings, activation, responsive layout, and no-markup regressions. |
| Native acceptance | `tests/e2e/*.spec.ts` | Actual Echo process over WebView2 CDP with isolated data, clipboard persistence, activation, and target-safe insertion. |
| Tooling | `tools/echo/*_test.go` | Command dispatch, flags, root resolution, ownership planning, architecture boundaries, generated transport drift, bootstrap independence, and cleanup. |

Concrete native specs are `tests/e2e/clipboard.spec.ts` and
`tests/e2e/quick-insert.spec.ts`; both are launched only through the authorized
native acceptance gates below.

## Ownership Boundaries

- `echo-engine` tests use the engine interfaces used by callers.
- `echo-storage` tests may exercise SQLite and blob behavior directly.
- `echo-windows` tests may exercise native behavior only under the authorized
  Windows environment.
- UI tests use the controlled IPC boundary and are not native acceptance.
- Native tests use the real Echo executable and the Go-owned Windows fixtures.

## Acceptance Isolation

Native tests require `ECHO_WINDOWS_ACCEPTANCE=1` and are launched by
`echo.cmd acceptance clipboard` or `echo.cmd acceptance quick-insert`. Each run
gets isolated data, WebView2, CDP, process, and evidence roots. Local unit,
browser, build, and smoke gates are not physical Windows acceptance.

## Product Boundary

Echo tests start only Echo-owned processes and use isolated data directories.
Favorites are backed by Saved Items; no removed reusable-content product,
compatibility route, or external runtime is part of this repository.
