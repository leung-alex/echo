# Echo Recall

Echo is a standalone clipboard history and Saved Items application. Clipboard
is the ingestion engine, History and Saved Items are its durable views, and
Quick Insert is the retrieval and insertion surface.

## Local development

Install the repository-owned toolchain and dependencies once:

```powershell
.\echo.cmd install
```

Normal local gates are exposed through the independent Echo command:

```powershell
.\echo.cmd verify
.\echo.cmd verify clipboard
.\echo.cmd verify quick-insert
.\echo.cmd self-check
.\echo.cmd format --check
.\echo.cmd bindings --check
.\echo.cmd smoke
.\echo.cmd perf
```

Activation uses the canonical `--echo-activate` flag and the versioned Echo
envelope.

The changed-owner planner can explain or execute only affected gates:

```powershell
.\echo.cmd verify --changed-from HEAD~1 --explain
.\echo.cmd verify --changed-from HEAD~1 --profile developer
```

The lower-level commands remain useful when working inside one layer:

```powershell
cargo test --workspace --locked
pnpm --dir apps/ui test
pnpm --dir apps/ui build
go -C tools/echo test ./...
go -C tools/echo vet ./...
```

On Windows, the native desktop package is built from `apps/desktop`.
Native clipboard and target acceptance is intentionally opt-in:

```powershell
$env:ECHO_WINDOWS_ACCEPTANCE = "1"
.\echo.cmd acceptance clipboard
.\echo.cmd acceptance quick-insert
```

These commands create isolated Echo data, WebView2, CDP, process, and evidence
roots.
