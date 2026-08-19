# Echo Recall

Echo is the standalone reusable-content application extracted from Culsans.
Clipboard is the ingestion engine, Library contains History/Favorites/Snippets,
and Quick Insert is the retrieval and insertion surface.

The repository intentionally contains no dependency on Culsans source, runtime,
storage, platform crates, or data directories.

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
.\echo.cmd smoke
```

The changed-owner planner can explain or execute only affected gates:

```powershell
.\echo.cmd verify --changed-from HEAD~1 --explain
.\echo.cmd verify --changed-from HEAD~1 --profile developer
```

The lower-level commands remain useful when working inside one layer:

```powershell
cargo test --workspace --locked
pnpm --dir frontend/app test
pnpm --dir frontend/app build
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
roots. They do not start Culsans or access the Culsans database.
