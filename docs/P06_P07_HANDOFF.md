# Echo P06/P07 Handoff

## Completion scope

P06 extracted Echo-owned Clipboard, Library, Quick Insert, activation, and
native acceptance test ownership. P07 added the independent repository tool
and root command surface. E01-E05 were not redone.

Echo remains independent of the Culsans source tree, database, blobs, runtime,
and platform crates. No file under `D:\Project\culsans` was modified.

## Baselines and commits

- Culsans read-only source baseline: `e625306a8b23e3cc1326738e528612455f0a4db6`
- Echo continuation-pack commit: `6b278c018b99de1ffc0996b5716df7a39b976a23`
- Echo P06 implementation commit: `cdb523bb2554667c822da7a7247c925833766a1c`
- Echo P07 implementation commit: `c5ef14657fc8094c821d80834b3c372280b6d3ec`
- Handoff documentation commit: recorded by Git when this document is committed

The original migration boundary and legacy data rules remain authoritative in
`docs/MIGRATION_MAP.md`.

## Test ownership

`docs/TEST_OWNERSHIP_MAP.md` is the detailed source-to-owner record. The Echo
test layers are:

| Owner | Echo coverage |
| --- | --- |
| `tests/ui` | Real browser DOM with controlled Tauri IPC stubs for Library views, search, copy, settings, escaping, and scoped actions. |
| `tests/e2e/clipboard` | Echo activation, clipboard representations, normalization, deduplication, persistence, copy, insert, and invalid targets. |
| `tests/e2e/quick-insert` | History/Favorites/Snippets views, snippet CRUD, search, copy/insert, target failure, hide/reopen, and single-instance behavior. |
| `tools/echo` | Bootstrap, command dispatch, owner planning, independence checks, and owned cleanup. |

The four legacy Clipboard/Quick Insert E2E files are rewritten or partially
rewritten for Echo. Echo replaces the Culsans Tauri/evidence helpers and uses
its own Go Windows clipboard and native-target fixture. Native runs are
isolated with Echo data, WebView2, CDP, process, and evidence roots.

The following remain Culsans-owned and were not deleted: Input Editor draft
recovery and editor lifecycle, command scope, File Search, Browser, Capture,
and unrelated shell visual/system acceptance. No Culsans helper is safe to
delete wholesale until its remaining consumers have been audited. The
Echo-only portions of the four named Clipboard/Quick Insert specs and fixture
cases are deletion candidates during Culsans cleanup review.

## Native fixture migration

The former test-only PowerShell fixtures were replaced by the Echo-owned Go
binary under `tools/echo/fixture`. It provides the real Windows clipboard
formats and a Win32 target window with primary, secondary, and password Edit
controls. Each authorized acceptance run builds the fixture into its isolated
run root and passes its absolute path to Playwright; no system PowerShell
process or Culsans helper is required.

## P07 command surface

The root `echo.cmd` bootstraps a cached Go 1.26.2 executable from
`tools/echo/`, hashes the Go inputs, copies the executable into a per-run
`.local/echo/run/<session>` directory, forwards arguments unchanged, and
cleans the session executable on exit. It only writes Echo-owned generated
paths.

Implemented commands:

`help`, `install`, `format [--check]`,
`verify [--changed-from <sha>] [--profile <developer|ci>] [--explain]`,
`self-check`, `build [--release]`, `dev`, `smoke`,
`acceptance clipboard`, `acceptance quick-insert`, `package [--dir]`,
`release-candidate`, `sync [--all | --branch <name>]`, and `clean`.

Changed-owner planning maps platform/clipboard to Clipboard and desktop,
storage to storage/library/migration, library to library/Quick Insert,
quick-insert to Quick Insert, protocol/desktop to desktop/activation, frontend
to frontend/Quick Insert, and tests to tests plus the relevant owner. Root
manifests, tooling, unknown paths, and ambiguous paths use conservative full
verification. `--explain` prints the changed files, owners, fallback reasons,
and gates.

## Executed gates

All results below are local Echo gates; no unrelated Culsans full regression
was run.

- PASS: `cargo test --workspace --locked`
- PASS: `pnpm --dir frontend/app test` (TypeScript plus 2 real-DOM Playwright tests)
- PASS: `pnpm --dir frontend/app build`
- PASS: `cargo fmt --all -- --check`
- PASS: `go -C tools/echo test ./...`
- PASS: `go -C tools/echo vet ./...`
- PASS: `echo.cmd self-check`
- PASS: `echo.cmd format --check`
- PASS: `echo.cmd verify clipboard`
- PASS: `echo.cmd verify quick-insert`
- PASS: `echo.cmd verify --changed-from cdb523b --profile developer --explain`
- PASS: `echo.cmd package --dir`
- PASS: `echo.cmd smoke` with isolated Echo data
- PASS: `echo.cmd release-candidate`
- PASS: native Playwright test listing (2 tests discovered)
- PASS: acceptance guard rejects execution without `ECHO_WINDOWS_ACCEPTANCE=1`

Physical Windows clipboard/system acceptance was not run. It remains a
separately authorized blocker and must be reported as such; unit, browser,
build, package, smoke, and acceptance-guard results are not physical clipboard
evidence.

## Deletion and migration boundary

The safe post-cutover cleanup candidates are the Culsans test code that only
asserts Echo-owned Clipboard/Quick Insert behavior, plus fixture operations
used exclusively by those tests, after a consumer audit. Culsans Input Editor,
shell, File Search, Browser, Capture, and shared helper consumers remain
protected. Echo's migration owns only the Clipboard/reusable-content tables,
referenced blobs, settings, saved items, and snippets listed in
`docs/MIGRATION_MAP.md`; it does not read or mutate `input_draft` or other
Culsans shell data.

## Remaining blocker

The implementation and local gates are complete. The remaining acceptance
blocker is explicit authorization and a real Windows environment for physical
clipboard formats, target focus/revalidation, and WebView2 activation runs.
