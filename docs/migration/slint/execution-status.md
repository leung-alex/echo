# Echo native migration execution status

Worktree: `D:\Worktrees\echo\ui`; branch: `codex/ui`. Main is not merged or modified.

## Implemented and pushed

- `986a040`: compiled Slint dual-window UI, typed Rust worker/presentation state, native window integration, bounded hidden-state resources, and the pinned accessibility fix preserving numeric-looking text.
- `e0641d5`: removed React/Vite/Node/Tauri source and build inputs; replaced browser tests with Windows UIAutomation/native fixture gates and retained storage leak checks.

## Evidence already observed on the authorized Windows machine

Evidence root: `D:\EchoMigrationEvidence\completion-20260906`. The entry worktree and retired source are archived outside the repository. Original Release/baseline evidence remains in `D:\EchoMigrationEvidence\20260905-slint` at original source `0dc699e42d8d667e502938d71e33216f92513e5a`.

- Rust workspace tests and the exact patched accessibility regression tests: PASS.
- `echo.cmd verify`: PASS, including Go tests/vet, Rust non-storage workspace tests, and the canonical storage test/leak gate (zero new TEMP and repository-local residuals).
- `C1-ui-v2`: 11 native UI/activation/persistence checks PASS. Enhanced final UI run additionally captures editing and dark theme; its final result will be referenced in the completion report.
- `C1-insert-v4`: actual paste into the owned Win32 input, rejection of a read-only target while preserving clipboard content, startup and shutdown PASS.
- `C1-clipboard`: real clipboard copy/readback PASS.
- `C1-package-v2`: actual independent NSIS installer and portable ZIP built; isolated installation, installed application smoke and uninstall PASS. Unknown files survived uninstall, and no shared runtime or normal Echo data was removed.

The current tested native binary SHA256 is `81e38187230443f9763a14a5f92ca3025cb1b89203d546d695e509368a873449`. It has no WebView subprocess/runtime dependency. Final clean-source packaging and matched performance measurements are in progress, not automatically PASS.

## Explicit acceptance boundaries

Physical Chinese IME, a hardware mixed-DPI/multi-monitor matrix, GPU renderer benchmarking, boot-cold startup, and an eight-hour soak must not be marked as passed without actual runs. The default renderer is software with an opaque theme fallback; Mica is conditional on a compatible optional GPU backend and Windows settings. Screenshot inspection is not a claim of pixel-identical rendering to the former browser implementation.

The installed Remote Desktop Commander terminal/file interface is functioning and was used for all current local builds and desktop acceptance. Earlier M0-only notes about its unavailable interface are historical and no longer apply.
