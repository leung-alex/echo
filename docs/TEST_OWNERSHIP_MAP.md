# Echo Test Ownership Map

## Echo-Owned Layers

| Layer | Location | Coverage |
| --- | --- | --- |
| Engine unit | `crates/echo-engine/src` | Ingestion races, normalization, fingerprints, retained representations, History/Saved Items contracts, Quick Insert actions, and target-session policy. |
| Storage adapter | `crates/echo-storage/src` and `crates/echo-storage/tests` | SQLite schema, FTS5/blob persistence, deduplication, migrations, Saved Item snapshots, writer/read runtimes, shutdown, and blob reconciliation. |
| Windows adapter | `crates/echo-windows/src` | Clipboard formats, native listener, target capture, focus validation, clipboard writes, paste delivery, named-pipe validation, tray, and window behavior where automation is safe. |
| Activation | `crates/echo-activation/src` | Echo envelope encoding, validation, and `--echo-activate` parsing. |
| Presentation | `crates/echo-presentation/src` | Query/load generations, bounded windows, opaque row keys, selection, keyboard/IME intent, activation epochs, and stale completion handling. |
| Desktop | `apps/desktop/src` | Typed worker/event contracts, Slint binding behavior that can be tested without launching the UI, activation routing, lifecycle decisions, and renderer selection policy. |
| Native UI acceptance | `tests/native` | Real native Echo executable and UI Automation against isolated synthetic data for the scenarios explicitly implemented by each gate. |
| Tooling | `tools/echo/*_test.go` | Command dispatch, flags, ownership planning, canonical storage leak-gate wiring, architecture boundaries, browser-stack retirement, bootstrap independence, packaging orchestration, and cleanup. |

Current retirement evidence is recorded in `RETIREMENT_REPORT.md`; `docs/migration/slint/execution-status.md` preserves historical migration evidence, not the current renderer contract. This map defines ownership, not an automatic passing status.

## Ownership Boundaries

- Engine tests use the same engine interfaces as production callers.
- Storage tests may exercise SQLite, blobs, FTS5, migrations, writer/read connections, maintenance, and shutdown directly.
- Windows tests may exercise native behavior only under the authorized Windows environment.
- Presentation tests remain UI-framework-independent and use opaque row keys at their public boundary.
- Desktop tests verify typed Rust work/event flow and lifecycle policy without moving Slint handles off the UI thread.
- Native UI tests use the real native executable, synthetic fixtures, an isolated `ECHO_DATA_DIR`, and Windows UI Automation where appropriate.
- The focused storage command, full verification, and changed-owner plans containing `storage` share the canonical storage leak gate. Full verification excludes `echo-storage` from its workspace test so that suite and the TEMP scanner run once; mixed owners do not schedule a duplicate storage package test.

## Smoke and Acceptance Isolation

`echo.cmd smoke` is a read-only startup and graceful-shutdown check. It uses isolated synthetic fixtures and `ECHO_DATA_DIR`; it must not read or remove user clipboard history.

Mutating native tests require separate authorization through `ECHO_WINDOWS_ACCEPTANCE=1` and are launched by `echo.cmd acceptance clipboard` or `echo.cmd acceptance quick-insert`. Each run must use an isolated evidence root, synthetic fixture state, and isolated `ECHO_DATA_DIR`. Tests must not uninstall a system WebView2 runtime and must not delete user clipboard data.

Build, smoke, unit, accessibility-tree, and UI Automation results are not substitutes for physical environment evidence. In particular, text input or UIA tests cannot certify physical Chinese IME behavior, mixed-DPI multi-monitor behavior, or an eight-hour soak unless those exact scenarios were actually run and recorded.

### Space home-screen regression

`tests/native/Invoke-SpaceChoiceAcceptance.py` exercises the real SpaceChoice popup,
including mouse row centers, accessibility default actions, keyboard cancellation,
saved activation/restart, and renamed/reordered/deleted space identities. Run it
separately with `ECHO_WINDOWS_ACCEPTANCE=1`, an executable built with
`cargo build -p echo-desktop --locked --features native-test`, and
`--executable <exe> --template <synthetic-fixture> --evidence <new-directory>`.
The template must contain History, Favorites and two custom spaces (one empty),
with capture disabled. `--repro-only` runs just the original crash regression.
The script copies the fixture and records process exits, calls and screenshots;
it never uses the installed application's data. This is automated native coverage,
not physical-input certification.

Inline completion's keyboard lease, target range, composition evidence and native
selection verification belong to `echo-windows`; query selection and stale row
gating belong to `echo-presentation`. See `docs/engineering/inline-completion.md`.
The explicit storage scale gate is `cargo test -p echo-storage --test fuzzy_search
fuzzy_search_large_corpus_keeps_tail_results_and_cancels --locked -- --ignored
--nocapture`; it exercises 1k/10k real SQLite corpora, including searchable bodies
beyond 16 MiB, and reports cold, normalized reuse and cancellation timings. Those
numbers do not certify desktop input latency or physical-input acceptance.

## Packaging Evidence

Portable directory/ZIP and optional NSIS packaging are separate outputs. Successfully producing one output does not imply that installer behavior, upgrade/uninstall behavior, or a final release-candidate pass was tested. Report every gate as PASS, FAIL, or NOT RUN from observed evidence only.

## Product Boundary

Echo tests start only Echo-owned processes and use isolated data directories. Favorites are backed by the distinct Saved Item model; no removed reusable-content product or compatibility route is part of the active architecture. Copy and insert assertions must verify retained original clipboard representations where the scenario depends on formats, rather than accepting preview text as equivalent.

## Retained retirement regressions

`echo-storage/tests/retained_regressions.rs` carries the former r1 filename,
r4 FTS-orphan deletion, and r5 maintenance-once assertions through the canonical
storage gate. Historical database migration fixtures remain supported.
`echo-presentation::navigation` protects space identity and insertion readiness;
`slide` protects finite software transitions. GPU spring/projection tests are retired.
`echo.cmd smoke` uses `Invoke-Smoke.ps1` and `EchoSmokeDriver.cs`, with no clipboard
operations or mutating acceptance flag. The shared UIA/input drivers remain in use.

## UI retirement assertion migration (2026-09-11)

The current software gate also owns `saved-content-create-edit-icon-delete`
(create, persisted content/name/Mail icon, edit, reopen, delete) and
`history-clear-cancel-retains-content`. Each runs against a fresh synthetic
fixture; the final original-content/blob signature must equal its original; only the three expected Favorites revisions and modification time may change.
These replace the corresponding current-product portions of the old
`history-favorite-create-edit-delete` and `batch-and-clear-cancel` checks.
The remaining current-product assertions now live in software checks for invalid
and valid settings, About, activation replay/malformed envelopes, and settings/
deleted-item persistence after a real process restart. The old script has retired.
The complete mapping, retained state tests and inapplicable product assumptions
are documented in [retirement-test-map.md](engineering/retirement-test-map.md).
