# Echo retirement report

## Baseline and decisions

- Audit package: `.local/devpacks/echo-retirement-audit-c0e8732` (audit SHA `c0e8732`).
- Actual clean baseline: `44fc2a0a723595a3ff51ca557bfa03653147e521`.
- Branch: `codex/retirement`; local commits only. Other worktrees preserved.
- Owner explicitly ended optional GPU/Skia maintenance and authorized isolated native/clipboard and ten-minute software performance verification.
- Baseline self-check, format and verify: PASS. Toolchain: Rust/Cargo 1.97.0, Go 1.26.2.
- Existing product animation failure is not waived. This report does not certify physical input, all applications or all displays.

## Retirement inventory

| Items | Disposition |
| --- | --- |
| R01/R02 | Removed unreachable fixture preparation/helper and format forwarding wrapper. Canonical format remains. |
| R04/R06 | Software gate renamed; duplicate default-equivalent CI test removed; debug/Release builds and large-corpus gate retained. |
| R03 | Three assertions migrated to canonical storage tests; standalone harness tracked manifests/lockfiles/source and scanner exception removed. Unknown local build artifacts left untouched. |
| R05 | Canonical smoke uses a restricted driver and dedicated runner; no clipboard or acceptance flag. Shared drivers retained because current inline/software/performance runners compile them. Legacy direct UI script retained pending complete equivalent UI assertion coverage (see below). |
| R07 | Evidence CI merged; Windows collector/build/environment/failure logs retained. Old workflow and preparation whitelist entry removed. |
| R08/R09 | GPU/Skia features, probe, GPU modules and two vendor packages removed. Winit software and accessibility patches retained; GPU offscreen hook removed. ECHO_RENDERER accepts only software. |
| R10 | Renderer-independent Navigation retains identity/order/readiness; software slide remains the animation owner. GPU spring/projection/prewarm state and GPU-only UI capture tree removed. |
| R11 | Strict version-1 serialized settings and legacy fields retained with round-trip regression; no database migration or preference reset. |
| R12/R14 | CSS reference output and all accepted brand assets retained by scope decision. |
| R13 | 30 unused motion/GPU tokens removed after Rust/Slint consumer check; canonical generator regenerated Rust/Slint/CSS. Shared token global renamed DesignTokens. |
| R15/R16 | Current ADR/ownership/index updated; P08 archived; historical evidence retained. Cargo lock refreshed without package upgrades; notices regenerated from Windows runtime/build dependency graph. |

## Test migration

| Old assertion | Current owner / assertion | Gate |
| --- | --- | --- |
| r1 saved_file_name_uses_the_file_name_not_the_source_path | retained_regressions::r1_storage_harness, exact report.txt rather than source path; isolated on-disk DB | verify storage |
| r4 deleting_a_saved_item_removes_its_saved_item_fts_document | retained_regressions::r4_saved_item_fts, direct SQL count for deleted SavedItem is zero | verify storage |
| r5 storage_open_runs_startup_maintenance_once | retained_regressions::r5_storage_runtime, shutdown then maintenance metric samples equals one | verify storage |
| r5 obsolete_clipboard_startup_hook_is_absent | Temporary source-string assertion retired; startup-once and existing maintenance_runs_on_startup_and_delete_not_on_normal_insert behavior retained | verify storage |
| GPU Deck navigation barrier | navigation readiness/latest-intent, hidden rejection, invalid/deleted identities; software slide suite preserved | verify |
| GPU projected popup screen bounds | Real software side-card bounds across 96/120/144/192 DPI, screen edges and changing card heights | desktop tests |
| GPU sync/readback prohibition | Existing guard now checks active deck_controller and software_deck files, no retired poll exception | Go tests |
| Original legacy smoke | semantic-startup-one-window, History, close=hide, activate/reopen, graceful-exit; restricted UIA operations | smoke |

All three storage migrations were executed successfully before removing old harness sources. They use repository-local temporary directories and do not touch user data.

### Intentionally retained legacy material

`tests/native/Invoke-UiAcceptance.ps1` is no longer reached by canonical commands. Its saved-item CRUD/icon persistence, clear cancellation and activation replay checks are not all demonstrated as equivalent in the current software UI runner. It and the Go fixture remain pending that UI migration; deleting them now would discard evidence. Old two-window assertions are obsolete and are not current acceptance requirements. The shared EchoUi/EchoDriver/EchoBenchmarks/EchoComposition files remain active consumers, not retirement candidates.

Historical migration docs beyond P08 remain as evidence indexes with unresolved physical/performance items. Diagnostic export now uses echo.software.diagnostics.v1 with actual model/outgoing/side/thumbnail/queue/frame byte accounting; obsolete GPU counters are removed. Direct focus/caret/inline runners accept only software. The removed measure_flow.py benchmark depended on the retired GPU navigation counters; current software performance uses Measure-SoftwareDeck.ps1.

## Dependencies and evidence

Cargo lock entries: 668 -> 632; surviving package versions unchanged. Full resolved Windows metadata, active tree, removed-token list and command logs are in `.local/retirement-evidence/`. Notices were generated from 357 active runtime/build packages; declared metadata and missing standalone license texts remain explicitly listed for review.

Local batches: `0491079` tooling; `6f8d981` storage/smoke/CI; `baeadae` GPU/Skia retirement.

The notice generator also retains the OpenAI Apps SDK icon MIT notice directly from the shipped source asset license; 222 referenced Cargo license text hashes were verified.

Nineteen license texts newly unreferenced by the refreshed inventory were removed; unrelated pre-existing assets were preserved.

## Acceptance results

| Check | Result / evidence |
| --- | --- |
| Final self-check / format | PASS (`frozen-self-check.log`, `frozen-format.log`) |
| Final canonical verify | PASS (`frozen-verify.log`) |
| Native-test configuration unit tests | PASS (`frozen-native-unit.log`) |
| Large-corpus complete search/cancellation | PASS; 233.06 seconds (`large-corpus.log`) |
| Release | PASS; final source optimized build, 9m 29s (`frozen-release.log`) |
| Windows collector checks | PASS; 10 checks (`collector-tests.log`) |
| Native smoke / clipboard / quick-insert / ui | Pending |
| Software T/M memory and frame timing | Pending |
| Portable/ZIP package | Pending |
| Installation/uninstallation, physical IME, extra DPI/monitors/editors | NOT_RUN |

One verification attempt (`final-verify.log`) failed because the independent large-corpus test was still using its isolated temporary database. No data was deleted to hide this failure. After that test exited successfully, the full gate was rerun serially and passed (`frozen-verify.log`).
