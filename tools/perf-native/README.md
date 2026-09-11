# M0 Windows collectors

These scripts prepare and record evidence. None grants G0 or proves the Slint
migration has been performed. They never change Git branches, reset/clean a
worktree, install dependencies, trim working sets, or uninstall WebView2.
Use PowerShell 7 on a dedicated Windows test account/desktop. The evidence
parent directory must already exist; each run uses a NEW output directory.
Keep real product evidence outside the repository and do not upload personal
clipboard databases or screenshots to CI artifacts.

## Existing product gates and Release artifact

From a clean `codex/ui` checkout containing only M0 changes relative to
`0dc699e42d8d667e502938d71e33216f92513e5a`, after installing the repository's
existing prerequisites (including its pinned Go 1.26.2):

```powershell
pwsh -NoProfile -File tools/perf-native/Invoke-BaselineGates.ps1 `
  -RepositoryRoot 'D:\Worktrees\echo\ui' `
  -EvidenceRoot 'D:\EchoEvidence\A0-run-001' `
  -DedicatedTestDesktop -IncludeNativeAcceptance
```

The runner refuses an existing Echo process, a dirty/wrong checkout, product
changes before A0, overwritten evidence, and redirected/reparse paths. The
existing native tests MAY replace the test desktop's clipboard; never run them
against a working desktop with private clipboard contents. No existing process
is killed by the runner. A failed gate stops collection and preserves its log.
The previous process-level `ECHO_WINDOWS_ACCEPTANCE` value is restored.

The output preserves the environment, source SHA, gate logs, and hashed Release
EXE. The EXE is **not** a full installed-runtime inventory. The existing
`echo.cmd smoke` launches Debug; its result is deliberately named
`debug-bootstrap-smoke`, not Release startup. Existing acceptance deletes its
temporary evidence, so console success alone does not satisfy native evidence.

D1/D2 fixture snapshots, installer/installed-runtime inventory, externally
observed semantic UI readiness, visual inspection, ownership checks, GPU,
observer overhead, and matched repeat runs are still required for G0. The runner
does not invent those results or generate an automatically passing g0.json.

## Read-only process-tree sampler

After independently starting an isolated Echo instance and recording its PID:

```powershell
pwsh -NoProfile -File tools/perf-native/Measure-EchoProcessTree.ps1 `
  -RootProcessId 12345 -OutputDirectory 'D:\EchoEvidence\hidden-run-001' `
  -DurationSeconds 300 -IntervalMilliseconds 1000
```

This does not start/stop Echo or access clipboard payloads. Outputs are
`processes.csv`, `aggregate.csv`, and `metadata.json`. Private Bytes is private
commit; Private Working Set is private resident memory. Total working-set sums
may double-count shared pages. The first/new-process CPU interval is unknown,
not zero; root exit or missing metrics produces incomplete evidence, not a
spurious low-memory success. CPU is reported both as one logical-core equivalent
and normalized by the collector's available logical processor count.

Discovery is polling-based and uses PID + creation time, but can miss short-lived
or initially orphaned WebView2 descendants. `process_set_verified` is always
false until independent ownership evidence exists. Reads are sequential; use
`collection_duration_ms` and separate observer-overhead runs to assess bias.
GPU and UI-ready timings are not measured by this sampler. A marker ending in
`.claim` reserves each output path and is intentionally retained.

## Tool integration tests (not Echo acceptance)

```powershell
pwsh -NoProfile -File tools/perf-native/Test-Collectors.ps1 `
  -OutputDirectory 'D:\EchoEvidence\collector-tools-001'
```

Tests parse the scripts, verify no-clobber/reparse/containment behavior, collect
an environment with a synthetic binary, and sample test-owned PowerShell sleep
processes (live and exiting). Only those test-owned processes are stopped; Echo
and the clipboard are never used. Test output is explicitly synthetic. The
`Echo native Rust and Slint` workflow runs these tests on Windows, separately from
product acceptance. A green workflow does not release G0.
