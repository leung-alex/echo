# G4/G5 Windows evidence operations

## G4: WC one-shot collector

Run this on a dedicated Windows desktop with a new evidence directory:

```powershell
pwsh -File tools/acceptance/Invoke-WeChatEvidence.ps1 `
  -RepositoryRoot D:\Project\echo `
  -EvidenceRoot D:\EchoEvidence\wc-<run-id>
```

The default run is inventory-only. It records `source.json` (Git SHA/status and tracked workspace EXE/DLL hashes), `environment.txt`, and `wc-cases.json`. WC01 through WC12 are emitted as `NOT_RUN`. No keys, clicks, clipboard reads/writes, UI Automation, or text injection occur. Existing `WeChat.exe`/`Weixin.exe` processes produce a blocked inventory and the collector refuses to attach, close, or launch another instance.

For a separately authorized one-shot launch, provide the exact executable and `-LaunchWeChat`. The script starts that process once, waits briefly for startup, collects the same evidence, and closes only that owned process. It never attaches to or closes an existing process. If an existing process is present, the report records a blocked inventory and WC01-WC12 remain `NOT_RUN`.

ChatGPT and Codex are explicit compatibility boundaries. `wc-cases.json` records both as `REJECTED`; this collector must never inject into either application or the Codex task composer. These records do not establish physical Chinese IME, clipboard, mixed-DPI, or product acceptance.

## G5: ordinary Release preflight

```powershell
pwsh -File tools/acceptance/Invoke-ReleasePreflight.ps1 `
  -RepositoryRoot D:\Project\echo `
  -EvidenceRoot D:\EchoEvidence\release-preflight-<run-id>
```

The wrapper records `release-preflight.json`. It checks that ordinary Release input excludes `--features native-test`, native fixture markers, and synthetic fixture injection. The isolated-data check is `BLOCKED` until a new `ECHO_DATA_DIR` and evidence root are supplied. Add `-RunReleaseBuild` only when the ordinary Release build is intended; the command is exactly `echo.cmd build --release` and the wrapper does not add test features.

Native-test binaries, fixture runs, smoke checks, and synthetic data cannot be promoted to ordinary Release acceptance by this report. Existing evidence remains untouched because both wrappers require a new evidence directory.
