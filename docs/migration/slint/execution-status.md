# Native migration execution status

Target branch: `codex/ui` (the remote UI branch); `main` is not modified.
Original product source: `0dc699e42d8d667e502938d71e33216f92513e5a`.

## Committed slices

- M0-A, `9eea318`: Go preflight, artifact inventory and G0 evidence-integrity guard.
- M0-B, `b3a8a9c`: Windows evidence-tool CI, pinned action SHAs and Go 1.26.2.
  Run `33963605664`, job `101299679851`: tests, vet and Windows tool build PASS.
  That run contained no collector scripts and correctly skipped their test step.
- M0-C: PowerShell environment capture, no-clobber evidence helpers, existing-gate
  runner, read-only process-tree sampler and synthetic Windows integration tests.
  At this commit their Windows execution is pending the next CI run.

## Gate and environment boundary

G0 remains NOT RUN. No original Echo Release performance baseline, actual Echo
screenshots, Release startup acceptance, data upgrade, or Slint migration has
been accepted. No production Rust, React, Tauri, storage, clipboard behavior or
dependency versions have been changed, and no old UI/runtime has been deleted.

Remote Desktop Commander installation was confirmed after the user connected
it. The current conversation's tool catalog did not expose its filesystem or
terminal actions despite that installed state. This is a missing execution
interface in this conversation, not a request to reinstall the plugin. The
GitHub-hosted Windows tool run is not evidence from the user's own machine.

## Execution entry points

- `go -C tools/echo run ./perf-native help`
- `tools/perf-native/README.md` for the guarded Windows collectors and tests.
- `tools/perf-native/Invoke-BaselineGates.ps1` preserves existing-gate logs and a
  Release EXE, but explicitly does NOT manufacture a complete G0 baseline.

All tool test fixtures are synthetic. Complete and review the original Release
baseline on the authorized Windows desktop before M1 or removal of the old stack.
