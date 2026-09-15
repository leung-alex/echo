# Agent Workflow

1. Confirm the worktree, branch, base SHA, owned paths, and intentionally dirty files before editing.
2. Read `AGENTS.md`, the architecture overview, dependency rules, locality guide, and test ownership map for the affected seam. For UI work, read current Rust desktop, `echo-presentation`, Windows shell, and Slint source rather than retired browser-stack assumptions.
3. Make the smallest coherent change in the owning module. Keep domain policy, adapters, framework-independent presentation, desktop worker coordination, and Slint visuals at their boundaries.
4. Add module-level tests at the same typed interface used by callers. Other workers may own test/tool changes during a coordinated migration; do not edit outside assigned ownership.
5. Run only authorized gates. Never manufacture results or prewrite PASS. Report every relevant gate as PASS, FAIL, or NOT RUN, and identify who owns independent acceptance.

Changed paths are classified by `tools/echo`. Use:

```powershell
.\echo.cmd verify --changed-from <base> --explain
```

## Canonical Command Families

```powershell
.\echo.cmd self-check
.\echo.cmd format --check
.\echo.cmd verify
.\echo.cmd verify clipboard
.\echo.cmd verify quick-insert
.\echo.cmd verify storage
.\echo.cmd build
.\echo.cmd build --release
.\echo.cmd smoke
.\echo.cmd acceptance clipboard
.\echo.cmd acceptance quick-insert
.\echo.cmd package --dir
.\echo.cmd package
.\echo.cmd release-candidate
```

The command families remain stable while their implementation becomes fully native. Active gates must not require Node, pnpm, React, TypeScript, Tauri, WebView2, CDP, or browser fixtures.

## Storage Verification

Storage verification is a single canonical gate. `.\echo.cmd verify storage` runs the storage package tests together with the system-TEMP and repository-local residue scanner. Full verification runs non-storage workspace tests with `--exclude echo-storage`, then invokes that same canonical storage gate. A changed-owner plan containing `storage` invokes it once and does not add a duplicate `cargo test -p echo-storage`.

Preserve the storage runtime contract: one bounded writer actor, one read connection, ordered migrations, FTS5 ownership, debounced reconciliation, and clean shutdown. Normal capture must not scan the entire blob store.

## Smoke and Mutating Acceptance

`echo.cmd smoke` is read-only and uses isolated synthetic fixtures and `ECHO_DATA_DIR`. Mutating clipboard/target acceptance is separately authorized and requires `ECHO_WINDOWS_ACCEPTANCE=1`, isolated synthetic fixtures, and an isolated `ECHO_DATA_DIR`.

No gate may uninstall a system WebView2 runtime or delete user clipboard data. UIA or text-input automation cannot be reported as physical Chinese IME, mixed-DPI multi-monitor, or eight-hour soak evidence unless the exact physical scenario was run.

## Build and Package Reporting

Development builds cache generated Slint code in `echo-desktop-ui`, separate from
the desktop host. Continue using `echo.cmd dev` / `echo.cmd build`; no special
cache command is needed. The first build after this boundary changes pays the UI
compilation cost once. Subsequent Rust host edits reuse it. Slint sources,
translations, dependency versions, toolchain or profile changes can still require
recompilation; do not run `cargo clean` during ordinary development.

For build diagnostics, use `cargo build -p echo-desktop --locked
--no-default-features --timings`. Repeat without edits to check warm-cache reuse;
after changing a host Rust source, `cargo build -vv` should report
`Fresh echo-desktop-ui`. Timing reports live under `target/cargo-timings`.

The default desktop build and canonical build/package commands use Slint software rendering without WGPU or Skia. The card carousel moves and resizes native Slint components; the outside of the card stage is transparent. Optional GPU/Skia features are retired; ECHO_RENDERER accepts only software. `native-test` is restricted to the isolated acceptance executable and must never be enabled in a distribution build. Do not claim universal hardware support from compilation.

Packaging supports portable directory/ZIP output and an optional NSIS installer. Report which output was actually produced and tested. Do not infer installer coverage or a final release-candidate pass from compilation or archive creation.

## Attribution Review

The About view uses Slint's `AboutSlint` component. Before distribution, notices and license materials must be reconciled with exact versions in `Cargo.lock`, including the active Slint version. Treat this as a coordinator/legal-review requirement rather than asserting legal certainty in code or documentation.

## Remote execution recovery

For this project, a missing remote reply must not be treated as proof that the tool
is unavailable or the operation never executed. Recheck tool discovery, device
identity, current process IDs, on-disk changes and logs before replaying any write
or test. When a response is missing, wait approximately 30 seconds in the active session,
then retry discovery and inspect on-disk evidence before replaying an operation.
A missing reply alone establishes neither failure nor the cause of the outage;
never report that no writes occurred when earlier tool or disk evidence shows them. Do not start duplicate builds or foreground
test suites, and respect another session's declared file/UI ownership. Report
observed results rather than attributing all failures to network instability.
