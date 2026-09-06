# Echo Recall

Echo is a standalone native Windows clipboard history and Saved Items application. Clipboard is the ingestion engine, History is the captured timeline, Saved Items are the distinct durable reusable-content model, and Quick Insert is the retrieval and insertion surface.

The desktop is written in Rust and uses a compiled Slint interface. It has no active Node, pnpm, React, TypeScript, Tauri, WebView2, or CDP runtime dependency.

## Local development

Install the repository-owned Rust dependencies once:

```powershell
.\echo.cmd install
```

Canonical local gates and focused verification are exposed through the independent Echo command:

```powershell
.\echo.cmd self-check
.\echo.cmd format --check
.\echo.cmd verify
.\echo.cmd verify clipboard
.\echo.cmd verify quick-insert
.\echo.cmd verify storage
.\echo.cmd perf
```

The changed-owner planner can explain or execute affected gates:

```powershell
.\echo.cmd verify --changed-from HEAD~1 --explain
.\echo.cmd verify --changed-from HEAD~1 --profile developer
```

Lower-level Rust and tooling checks remain available when working inside one layer:

```powershell
cargo test --workspace --locked
go -C tools/echo test ./...
go -C tools/echo vet ./...
```

## Run and build

```powershell
.\echo.cmd dev
.\echo.cmd build
.\echo.cmd build --release
```

The normal build includes `cover-flow`: Slint + FemtoVG-WGPU on one DX12 device, using a DirectComposition visual for transparent floating cards. Each card owns its search, content and navigation. History and Favorites are fixed spaces; users can create other spaces without duplicating the Saved Item payload model. Production card textures are rendered directly on the GPU, never read back through the CPU. Integrated GPUs and battery use a bounded economical raster policy; settled native text remains at the window's original DPI.

`ECHO_RENDERER=software` selects the flat native compatibility path; `cargo build -p echo-desktop --no-default-features --locked` builds a software-only executable. The old `gpu` Cargo feature is an alias for `cover-flow`. `native-test` is an explicit test-only feature and is excluded from normal release/package builds. See `docs/architecture/cover-flow.md` for resource limits, fallback behavior and the offscreen adapter seam.

Activation uses the canonical `--echo-activate` flag and the versioned Echo envelope. Only one resident Echo host is allowed per Windows user, logon session, and canonical data directory. Secondary launches forward bounded arguments over the local named pipe. Closing Echo hides its one window and keeps capture resident; use Quit in the UI or native tray to stop capture and shut down.

## Smoke and native acceptance

`smoke` is a read-only startup and graceful-shutdown check using isolated synthetic data and `ECHO_DATA_DIR`:

```powershell
.\echo.cmd smoke
```

Tests that mutate the Windows clipboard or exercise physical target insertion are separately authorized:

```powershell
$env:ECHO_WINDOWS_ACCEPTANCE = "1"
.\echo.cmd acceptance clipboard
.\echo.cmd acceptance quick-insert
```

Acceptance must use isolated synthetic fixtures and an isolated `ECHO_DATA_DIR`. These commands must not uninstall a system WebView2 runtime and must not delete user clipboard data. Automated text and UI Automation checks do not certify physical Chinese IME behavior, mixed-DPI multi-monitor behavior, or an eight-hour soak unless those scenarios were actually run and recorded.

## Packaging

```powershell
.\echo.cmd package --dir
.\echo.cmd package
```

The packaging family supports an independent portable directory/ZIP flow and an optional NSIS installer flow. A produced artifact is not proof that installer tests or a final release-candidate pass were performed; report only gates actually observed.

## Attribution

The About view embeds Slint's `AboutSlint` component and identifies Echo as a Rust + Slint application. Distribution notices should match the exact dependency versions resolved by Cargo, including Slint `1.17.1` in the current manifest. This is a packaging requirement to review, not a statement of legal sufficiency.

### Native desktop acceptance

Use a dedicated desktop/test data directory and set `ECHO_WINDOWS_ACCEPTANCE=1` before `echo.cmd acceptance ui`, `echo.cmd acceptance clipboard`, `echo.cmd acceptance quick-insert`, or `echo.cmd smoke`. These commands compile their own test drivers and synthetic fixtures, retain evidence, and never reuse the normal Echo database. Do not run clipboard mutation checks while relying on the current clipboard.
