# Echo retirement report

**Latest outcome:** structural/native verification PASS; animation timing FAIL. The final continuation below supersedes the historical native failures. Multi-DPI / multi-monitor acceptance is EXCLUDED by user request.

## Baseline and decisions

- Audit package: `.local/devpacks/echo-retirement-audit-c0e8732` (audit SHA `c0e8732`).
- Actual clean baseline: `44fc2a0a723595a3ff51ca557bfa03653147e521`.
- Branch: `codex/retirement`; local commits only. Other worktrees preserved.
- Owner explicitly ended optional GPU/Skia maintenance and authorized isolated native/clipboard and ten-minute software performance verification.
- Baseline self-check, format and verify: PASS. Toolchain: Rust/Cargo 1.97.0, Go 1.26.2.
- Multi-DPI / multi-monitor acceptance is explicitly excluded by the user in the continuation. Earlier NOT_RUN records below remain historical.
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

Local batches: `0491079` tooling; `6f8d981` storage/smoke/CI; `baeadae` GPU/Skia retirement; `da8c200` documentation/notices.

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
| Native smoke | PASS; four checks including scoped reopen, acceptance flag unset (`final-smoke.log`) |
| Clipboard | PASS after user-authorized clipboard clear; original-format roundtrip/exclusions and History row copy (`native-retry/clipboard-v2.log`) |
| Quick Insert | FAIL; 41 distinct canonical cases executed across isolated fail-fast continuations: 38 PASS, 3 FAIL (`native-retry/quick-insert-case-summary.json`) |
| Software UI T/M | PASS; both canonical software datasets (`native-retry/ui-v2.log`) |
| Software T memory / animation | PASS memory: 34,082,816 bytes peak; FAIL animation: P95 28.712 ms (`perf-T/result.json`) |
| Software M memory / animation | PASS memory: 37,994,496 bytes peak; FAIL animation: P95 28.3254 ms (`perf-M/result.json`) |
| Portable/ZIP package | PASS; clean source da8c200, 319 manifest hashes and exact ZIP byte identity verified; icon/version/manifest resources present (`package-inspection.json`) |
| NSIS installer build | NOT_BUILT; independent compiler unavailable |
| Installation/uninstallation, physical IME, extra DPI/monitors/editors | NOT_RUN |

One verification attempt (`final-verify.log`) failed because the independent large-corpus test was still using its isolated temporary database. No data was deleted to hide this failure. After that test exited successfully, the full gate was rerun serially and passed (`frozen-verify.log`).

## Artifact identity and native boundary

- Final Release SHA256: `e66e351a6c24e9afa2b45d2ce749c3a000a0bdb0aa922a74cecefa80a15bdb27`.
- Package: `target/echo-package/0.1.0/20260911T102131146-f3d473c0`; portable `Echo.exe` exactly matches the measured Release binary. The packaged source commit is clean `da8c200c8ca6b727c1bc1b2ec6cda8deb9c05c06`; later report-only changes do not change these executable bytes.
- Package resources: 11 icon images, one group icon, one version resource and one manifest. This verifies resource presence, not taskbar/tray visual or installer acceptance.
- The initial `clipboard`, `quick-insert` and `ui` attempts stopped before mutation because the wrapper could not materialize EnterpriseDataProtectionId. Those BLOCKED logs remain preserved. The subsequent user-authorized clear and actual native results below supersede that preflight status.
- The initial UI command stopped on the T preflight and did not run M; the authorized retry executed and passed both. The independent T/M performance runner uses capture-disabled synthetic fixtures and never invokes clipboard operations; it is a separate, narrower observation.
- Current-head image payload and anchored neighbor bounds tests remain and passed in `frozen-verify.log` (`image_only_original_is_an_insert_payload`, `popup_anchors_front_and_contains_the_whole_neighbor_at_screen_edges`). Native insertion coverage and remaining failures are detailed below.

## Five-minute software performance results

Both runs used the final non-native-test Release binary, one owned process and a separate copy of synthetic data (2,000 History / 200 Saved Items; T: 0 images, M: 20 images). Each completed 120 seconds visible, 120 hidden and 60 restored, with all 300 one-Hz samples valid. These are sampled Private Bytes peaks, not a claim about unsampled allocation high-water marks or compositor GPU allocations.

| Dataset | Sampled peak bytes | Strict < 50,000,000 | Animation P95 | Required <= 20 ms | Reclaim acknowledgment |
| --- | ---: | --- | ---: | --- | ---: |
| Text T | 34,082,816 | PASS | 28.712 ms | FAIL | 30.0021818 s |
| Mixed images M | 37,994,496 | PASS | 28.3254 ms | FAIL | 30.0010228 s |

Both runs confirmed software renderer selection, complete synthetic History readiness, navigation after restoration, zero reclaimed row/thumbnail/frame bytes, worker cache acknowledgment, unchanged original-representation database signatures and normal exit. Each result contains only the animation-P95 error; thresholds were not weakened. Raw samples, lifecycle traces, actions, before/after signatures and the independent aggregate are retained in `.local/retirement-evidence/perf-T`, `perf-M` and `performance-summary.json`.

**Outcome: structural cleanup PASS; animation timing FAIL. Overall product acceptance is not PASS.** Native clipboard and software UI now PASS; Quick Insert remains FAIL on three assertions listed below. Installation/uninstallation, physical IME, multiple DPI/monitors and additional real editors are NOT_RUN. The retained legacy direct UI assertions still need equivalent migration before their source can retire.

## Authorized native continuation (2026-09-11)

The user explicitly authorized discarding the current clipboard without backup. The clipboard was cleared and verified empty (`native-retry/clear.log`). The ordinary wrapper then protected each actual test run; no backup opt-out was added to repository code. Synthetic fixtures and owned test instances were used throughout, with original production History untouched.

The empty clipboard exposed a preexisting PowerShell pipeline issue: an empty `if` result assigned `$null` to `formats`. Wrapping the entire expression in `@(...)` preserves the zero/single/multiple-format array contract. The native tests then exposed stale fixture/selection assumptions: D2 has a pinned multiline SQL entry 0199, whereas T/M use text 1999; preexisting selected text is a replacement range rather than an initial query. The scripts now share a strict dataset-specific expected original, use Win32 CRLF in input comparisons, and preserve full Unicode/duplicate-text/newline boundary assertions. Unknown datasets fail explicitly. No product code, visual thresholds or protected-input focus assertions changed.

| Native scope | Result | Evidence under `.local/retirement-evidence/native-retry` |
| --- | --- | --- |
| Clipboard | PASS | `clipboard-20260911T115740.867925200`: text/HTML/RTF/image/files roundtrip, five capture exclusions, invalid preparation preservation, History row copy |
| Software UI T | PASS | `software-deck-20260911T115847.616954000/T/summary.json` |
| Software UI M | PASS | `software-deck-20260911T115847.616954000/M/summary.json` |
| Quick Insert | FAIL: 38 PASS / 3 FAIL across all 41 non-stress canonical cases | `quick-insert-case-summary.json`; every source summary and failed attempt retained |

Quick Insert remaining failures:

1. `fuzzy-words-and-trailing-spaces-keep-actions-stable`: reproduced in two full canonical attempts. Copy-action pixels changed in 1/99 and 2/102 samples respectively; internal row/selection/visible/busy state stayed coherent in all 133/135 corresponding samples. Threshold unchanged; no claim that the cause is environmental.
2. `unsupported-protected-input-is-explicit-compatibility`: reproduced in a fresh isolated run (`quick-insert-remaining-password`). F6 entered manual History but Echo did not become the foreground keyboard target. The failure occurs during the password subcase; the later readonly subcase was not reached and is NOT_RUN.
3. `streaming-filter-never-clears-the-panel`: reproduced independently (`quick-insert-remaining-streaming`). In the first run, 1/143 frame-state samples reported `navigation_busy`; all frames remained visible, rows never became zero, and 119 physical-header samples showed no flash. The assertion's generic "hid/blanked" error must not be presented as proof of a visually blank panel.

All 41 named cases were attempted, using fresh isolated continuations to reach cases after fail-fast exits; this is not a claim that one uninterrupted Quick Insert gate passed. Corrected fixture/range cases passed native regression. Installed Chinese IME driven by synthetic keys passed its automated case; physical keyboard/IME acceptance remains NOT_RUN. Browser/extra real-editor and stress suites were not added to this run.

Native-test executable SHA256 remained `a1b653cfebb3010f3357dc37e9766cb2b0c3d670ae1824120ff7518079bc1a8c`; Release/package bytes and the earlier five-minute performance evidence are unchanged. Both focused reviews accepted the script corrections. Current self-check, format and full verify PASS (`native-retry/self-check.log`, `format.log`, `verify.log`).

## Native repair continuation (2026-09-11, evening)

The earlier native and performance tables are historical results for their stated executable hashes. This continuation repairs the observed failures without changing the acceptance thresholds. Multi-DPI / multi-monitor work is EXCLUDED by explicit user instruction; physical IME and additional real editors remain separate NOT_RUN boundaries.

Changes:

- F6 uses a one-shot Windows hotkey notification to acquire the foreground permission required by explicit manual History. The dedicated hook thread drains queued old hotkey messages before ID reuse. Registered key-down and key-up both reach Windows, auto-repeat is suppressed, and the passed-key lease is reconciled on rearm and discarded when its hook is removed. Foreground failures are now propagated to the caller.
- Query-content readiness remains guarded by tickets, epochs, loading/dirty state and `deck.can_insert`. `navigation_busy` now describes actual software slide loading/motion, so ordinary query refresh does not disable row actions as if it were a spatial transition.
- Full-sequence testing exposed framework updates removing TOPMOST after manual History. At the failed pointer location, the owned input fixture was hit; Echo was uncloaked and had EXSTYLE `0x080C0110`, without TOPMOST. The native inline window hook now enforces TOPMOST plus NOACTIVATE during window-position changes. Manual History and editor modes disable that hook policy. A Slint-property experiment did not fix the issue and was removed.
- The action sampler now includes pointer coordinates, hit HWND, styles and DWM cloaking state in its existing failure. No ownership check, visual assertion or tolerance was relaxed.

Regression mapping and evidence under `.local/retirement-evidence/fixes`:

| Regression | Evidence / result |
| --- | --- |
| Stale F6 message versus reused registration | `reusing_manual_history_hotkey_discards_old_session_messages_only` |
| Registered F6 repeat versus matching release | `registered_f6_suppresses_repeat_but_releases_windows_key_state` |
| Hook removal before release | `removing_hook_discards_passed_f6_lease_when_release_can_no_longer_be_observed` |
| Windows unit suite | 90 PASS, 1 ignored (`windows-tests-final.log`) |
| Protected password and readonly F6 | PASS in `quick-insert-remaining/summary.json` |
| Streaming filter | PASS in `quick-insert-remaining/summary.json` |
| Manual History then fuzzy action pixels | Reproduced FAIL before the native position fix (`pixels-window-probe`); PASS after it (`pixels-windowpos-2`) |

The initial focus-style experiment and failed full runs are retained. The first hotkey experiment left an unmatched synthetic F6 down; the test's held-key guard caught it, the pairing was fixed, and only that owned synthetic release was sent (`owned-f6-release.json`). One rebuild overlapped an old-binary diagnostic and failed with access denied; that attempt (`pixels-windowpos`) is not evidence for the new position fix. The successful `debug-windowpos-build-2.log` precedes `pixels-windowpos-2`.

A temporary frame-scheduling probe found late timer callbacks but did not establish a complete root cause. Its 76-second diagnostic is not a five-minute acceptance run, uses a native-test executable despite the copied runner's default `native_test` field, and intentionally fails the full sample-count checks. All frame instrumentation was removed from the product. The earlier five-minute P95 failures remain valid historical evidence.

Installer preparation remains blocked: no local NSIS compiler was found; official SourceForge download attempts returned HTML rather than a ZIP; a session-preserving automatic-redirect attempt also failed with transport EOF. Archive validation rejected the HTML files before extraction or execution. No installer or uninstaller was run, and no installed application, shortcut, user History or runtime was removed.

The final debug/native-test Quick Insert run passed all 41 canonical non-stress cases in one uninterrupted run (`quick-insert-final/summary.json`), including the installed synthetic IME case, image insertion, protected inputs, query/selection/range races, cancellation/rearm, hidden reclamation and streaming filtering. Executable SHA256: `f1ec6a0932df1c9c14349de50a072d514f316924b1f40a7e4315acfcf17e04cb`. This is real isolated Windows execution using the canonical runner, on the debug/native-test build; it is not a physical-keyboard or additional-editor certification. Current self-check, format and full verify passed (`final-self-check.log`, `final-format.log`, `final-verify.log`).

## Final continuation results and delivery

Product source commit: `c44b328059adffb45b210b7741379f31b8220840` on `codex/retirement`. Local `main` remains `44fc2a0a723595a3ff51ca557bfa03653147e521`; no push or merge was performed. Subsequent report-only commits do not change executable contents.

| Check | Final result / evidence under `.local/retirement-evidence/fixes` |
| --- | --- |
| Self-check, format, verify | PASS (`final-self-check.log`, `final-format.log`, `final-verify.log`) |
| Windows feature graph and toolchain | Recorded (`windows-features-final.txt`, `toolchain-final.txt`) |
| Quick Insert | PASS, 41/41 in one run (`quick-insert-final/summary.json`); debug/native-test executable identified above |
| Clipboard | PASS, original format/exclusion gate and History row copy; clipboard restored (`clipboard-final/summary.json`, `clipboard-final/clipboard-preservation.json`) |
| Software UI | PASS for T and M (`ui-final-T/summary.json`, `ui-final-M/summary.json`) |
| Final Release | PASS (`release-final-build.log`); package command rebuilt after reverted diagnostic files changed timestamps (`package-final.log`) |
| Packaged EXE smoke | PASS, acceptance flag unset (`smoke-final-2/summary.json`); first attempt stopped before launch because its fixture-generator environment was missing |
| Portable/ZIP | PASS, 319 manifest file hashes, full ZIP byte identity, icon/version/manifest resources and icon license (`package-inspection-final.json`) |
| Installer build | NOT_BUILT; NSIS unavailable |
| Install/uninstall, physical IME, extra real editors | NOT_RUN |
| Multi-DPI / multi-monitor | EXCLUDED by user request |

The actual portable artifact is `target/echo-package/0.1.0/20260911T132939748-833d36d9`, built from clean source `c44b328`. Packaged `Echo.exe` exactly matches `target/release/echo-desktop.exe`, SHA256 `d3d0502523c02de485c92125dbc99c78934b9d7c25085f3e479a68271839e74e`. Resource verification found 11 icon entries, one group icon, one version entry and one manifest. This remains portable/ZIP acceptance, not installation or visual branding acceptance.

Both final five-minute runs used this packaged, non-native-test EXE with no Cargo/rustc build running. Each had 300 valid one-Hz samples: 120 visible, 120 hidden, 60 restored. Each preserved the original-representation signature, confirmed software selection and reclamation, restored complete History/navigation, and exited normally. Each result has exactly one error: animation P95 above the unchanged 20ms limit.

| Dataset | Sampled peak bytes | Strict < 50,000,000 | Animation P95 | <= 20ms | Reclaim |
| --- | ---: | --- | ---: | --- | ---: |
| Text T | 35,467,264 | PASS | 28.5783ms | FAIL | 30.0003828s |
| Mixed images M | 34,762,752 | PASS | 28.7108ms | FAIL | 30.0015087s |

Evidence: `perf-final-T`, `perf-final-M`, `performance-final-summary.json`; these sampled peaks do not claim unsampled allocation high-water proof. The prior performance failures and diagnostic experiments remain preserved.

**Final outcome: structural cleanup PASS; animation timing FAIL. Overall product acceptance is not PASS.** The three previously failing Quick Insert assertions are now repaired and pass the full native run. Animation timing remains unresolved. Installer/physical-input/extra-editor boundaries above remain unaccepted, and the legacy direct UI script's unique assertions still require equivalent migration before that script can retire. R12 CSS and R14 icon aliases remain intentionally retained.
