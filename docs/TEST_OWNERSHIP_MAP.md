# Echo Test Ownership Map

## Baseline

Echo owns the Clipboard, Library, Quick Insert, activation, storage, and
desktop lifecycle behavior described by the E01-E05 extraction. The inspected
Culsans baseline is `e625306a8b23e3cc1326738e528612455f0a4db6`. Echo tests run
from this repository and never start Culsans.

## Legacy test decisions

| Legacy asset                             | Echo decision                  | Destination / reason                                                                                                                                                 |
| ---------------------------------------- | ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tests/e2e/clipboard-first-open.spec.ts` | Rewrite                        | Echo owns first visible frame, activation, focus, hide, and reopen. The Culsans File Search cases stay in Culsans.                                                   |
| `tests/e2e/clipboard-system.spec.ts`     | Rewrite                        | Echo owns listener capture, representations, normalization, deduplication, sensitive-source policy, persistence, copy, insert, and fail-closed targets.              |
| `tests/e2e/clipboard-visual.spec.ts`     | Partial rewrite                | Echo keeps first-frame, History/Favorites/Snippets layout, and responsive overflow coverage. Culsans command-shell and unrelated surface assertions stay in Culsans. |
| `tests/e2e/quick-insert.spec.ts`         | Rewrite                        | Echo owns the single Quick Insert surface, all three Library views, search, favorites, snippets, copy, insert, target failure, and hide/reopen behavior.             |
| `tests/e2e/tauri.ts`, `evidence.ts`      | Replace                        | Echo uses a minimal CDP/IPC helper and run-scoped evidence writer under `tests/e2e/`; no Culsans import is permitted.                                                |
| Culsans clipboard/WPF fixtures           | Replace only required behavior | Echo may use a small Windows fixture for clipboard formats and target revalidation. Input Editor and shell fixtures remain Culsans.                                  |

## Echo-owned test layers

| Layer             | Location               | Coverage                                                                                                                                                  |
| ----------------- | ---------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Domain/unit       | `backend/crates/*/src` | Listener races, self-write suppression, normalization, fingerprint, storage/blob GC, migration, Library views, Quick Insert actions, activation protocol. |
| Browser UI        | `tests/ui/ui.spec.ts`  | Real DOM rendering with a controlled Tauri IPC boundary: History, Favorites, Snippets, search, copy, settings, escaping, and item-scoped actions.         |
| Native acceptance | `tests/e2e/*.spec.ts`  | Direct Echo process over WebView2 CDP with isolated data, activation, clipboard system behavior, persistence, and target-safe insertion.                  |
| Tooling           | `tools/echo/*_test.go` | Command dispatch, flags, root resolution, owner planning, bootstrap independence, and owned cleanup.                                                      |

## Tests intentionally staying in Culsans

- Input Editor draft recovery and editor lifecycle.
- Culsans command scope, File Search, Browser, Capture, and unrelated shell
  visual/system acceptance.
- Cross-product activation compatibility beyond Echo's already implemented
  activation v1 contract.

These files are not deleted by Echo work. They become safe deletion candidates
only after Culsans performs its own cleanup review.

## Acceptance isolation

Native tests require `ECHO_WINDOWS_ACCEPTANCE=1` and are launched by
`echo.cmd acceptance clipboard` or `echo.cmd acceptance quick-insert`. Each run
gets an isolated `ECHO_DATA_DIR`, optional `ECHO_LEGACY_DATA_DIR`, WebView2
profile, CDP port, process record, and evidence directory. The runner validates
the Echo executable identity and terminates only the owned process tree.

Physical Windows acceptance remains separately authorized. Local unit, browser,
build, and smoke gates must not be reported as physical clipboard acceptance.
