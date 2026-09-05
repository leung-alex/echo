# Native migration execution status

Target branch: `codex/ui` (the remote UI branch); `main` is not modified.
Original product source: `0dc699e42d8d667e502938d71e33216f92513e5a`.

## Committed slices

| Slice | Commit | Scope |
| --- | --- | --- |
| M0-A | `9eea318` | Go preflight, artifact inventory and G0 evidence-integrity guard. |
| M0-B | `b3a8a9c` | Windows evidence-tool CI with pinned action SHAs and Go 1.26.2. |
| M0-C | `3f50bae` | Environment capture, immutable evidence helpers, existing-gate runner, read-only process-tree sampler and Windows tool tests. |

## Verified Windows tooling results

The tested code commit is `3f50bae062dbc5d4638ca06fb372e9bb027f4008`.
GitHub Actions run `33963952742`, job `101300597883`, completed successfully.
The earlier M0-B run `33963605664` also passed its Go tests/build; collector tests
were correctly skipped there because the scripts had not yet been committed.

| Check | Actual result |
| --- | --- |
| Runner | GitHub-hosted Windows Server 2022, build 10.0.20348.0. |
| Toolchains | Go 1.26.2; PowerShell 7.6.5. |
| Go evidence-tool tests | PASS: 13 top-level tests and 40 rejection subtests. |
| Go vet and Windows tool build | PASS. |
| Collector integration tests | PASS: 10 cases, 0 failures. |
| PowerShell parser | PASS for all collector/test scripts. |
| Evidence protection | PASS: UTF-8/no-overwrite, reservation, containment and junction rejection. |
| Baseline runner authorization refusal | PASS; its normal Echo execution path is NOT RUN. |
| Live-process sampling | PASS on test-owned PowerShell/conhost processes; 6 valid samples; ownership remains explicitly unverified. |
| Root exit handling | PASS: incomplete evidence is recorded instead of a false low-memory result. |

The downloaded artifact was inspected; its JSON/JSONL agrees with the results
above. Artifact ID: `9968839010`.
Artifact name: `echo-m0-tools-3f50bae062dbc5d4638ca06fb372e9bb027f4008-1`.
Artifact ZIP SHA-256:
`a10940a339b07fce6c8e97d5f3c638c8586d146ce0723db05368518ea9a792f2`.

**All binaries and sampled processes in these tests are synthetic test fixtures,
not the Echo product. The tests did not start Echo or touch the clipboard.**
No fixture screenshot or resource value may be presented as Echo acceptance.

## Gate and environment boundary

G0 remains NOT RUN. The original Echo Release baseline, actual Echo screenshots,
Release startup acceptance, before/after performance, data upgrade and Slint
migration have not been executed/accepted. No production Rust, React, Tauri,
storage, clipboard behavior or dependency versions have been changed. No old
UI/runtime has been deleted.

Remote Desktop Commander installation was confirmed after the user connected
it. The current conversation's tool catalog did not expose its filesystem or
terminal actions despite that installed state. This is a missing execution
interface in this conversation, not a request to reinstall the plugin. The
GitHub-hosted Windows tool run is not evidence from the user's own machine.

## Execution entry points and next gate

- `go -C tools/echo run ./perf-native help`
- `tools/perf-native/README.md` for the guarded Windows collectors and tests.
- `tools/perf-native/Invoke-BaselineGates.ps1` preserves existing-gate logs and a
  Release EXE but does NOT manufacture a complete G0 baseline. Its successful
  product-gate path still requires validation on the authorized Windows desktop.

Next: complete and independently review the original Release baseline on the
authorized Windows desktop. Remaining evidence includes D1/D2 fixtures,
installer/installed-runtime inventory, externally observed semantic UI readiness,
actual Echo screenshots, verified WebView2 process ownership, observer overhead,
and matched repeated resource/startup runs. Do not proceed with M1 or remove the
old stack until those G0 requirements are satisfied.
