# Desktop Ownership

Desktop is Echo's native Rust composition root. Read `docs/architecture/overview.md`, `docs/architecture/dependency-rules.md`, and `crates/echo-presentation/AGENTS.md` before changing it.

## Ownership

- `src/app.rs` owns UI-thread orchestration for one floating native Slint window. `src/app/deck_controller.rs` and `src/app/software_deck.rs` own space navigation and two-panel software motion. `src/cover_flow` is the preserved optional diagnostic GPU path.
- `src/app/bindings.rs` connects generated Slint callbacks to Rust behavior.
- `src/service.rs` owns the background worker and its typed Rust work/result messages.
- `src/events.rs` owns typed events delivered back to the Slint event loop.
- `ui/*.slint` owns visual structure, component properties, callbacks, and accessibility metadata.
- `build.rs` compiles the Slint interface and embeds Windows resources.

Business policy belongs to `echo-engine`. Framework-independent presentation state, interaction interpretation, opaque row keys, pagination generations, and session epochs belong to `echo-presentation`. Win32 clipboard, focus, paste, named-pipe, tray, and HWND behavior belongs to `echo-windows`.

## Boundary Rules

- Slint component handles and models remain on the UI thread.
- Background work crosses the worker boundary through typed Rust enums; do not introduce string command names, JSON IPC, browser transport DTOs, or polling.
- Treat `echo_presentation::RowKey` values as opaque strings. Do not parse a source or numeric database ID from a key in Slint.
- Preview models are display-only. Copy and insert actions must continue through engine services that retrieve the retained original clipboard representations.
- Target capture for Quick Insert completes before Echo windows are shown.
- Closing or dismissing a window hides it. Explicit Quit closes the hub/event loop and allows worker, clipboard, storage, pipe, and tray shutdown.
- Normal builds and canonical commands use software rendering with `--no-default-features`. Do not enable WGPU/Skia or `native-test` in release packaging. The card-only window keeps native DPI and alpha shadows without a global opaque frame.

The About view uses Slint's `AboutSlint` component. Packaging attribution and license notices must be checked against the exact Cargo-resolved Slint and other dependency versions; do not claim legal completeness from the UI string alone.
