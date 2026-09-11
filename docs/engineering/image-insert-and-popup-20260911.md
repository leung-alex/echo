# Image insertion and anchored carousel repair

Base: `c0e8732cc4b5ad205e7a747e5eca859f2cda3c12` on local `main`.
Evidence: `.local/devpack-evidence/image-insert/`.

## Behavior

- Quick Insert retains original BMP/PNG image representations instead of requiring a text representation. PNG originals remain on the clipboard alongside the compatible native DIB. Validation hashes the retained representation without decoding another full frame.
- Selection, target identity, clipboard sequence/content, cancellation and single-write checks remain in force. UIA preflight reads use the same redundant-provider-notification guard as paste; physical input still invalidates the operation.
- Image receipt requires exact embedded-object readback (including the verified Editor Kit block projection), or exactly one new substantial attachment in the same input container with existing attachments and surrounding text preserved. Small toolbar glyphs cannot acknowledge a paste. Unconfirmed delivery prevents retries and leaves Echo visible.
- Successful insertion hides Echo in both inline and manual insertion paths. Typing/filtering alone keeps the popup visible.
- When an anchored popup has room on one side, the other space uses that available slot. The main card retains its position. Previews keep the current search generation.

## Evidence obtained

Real application tests used isolated synthetic data and unsent drafts. No messages were sent; test drafts were cleaned. Clipboard wrappers recorded restoration.

| Check | Result | Evidence |
| --- | --- | --- |
| Codex: blank draft, one PNG attachment, automatic hide | PASS | `20260911-r7/codex-after.json` and Computer Use observation |
| Tabbit / ChatGPT: blank draft, one PNG attachment, automatic hide | PASS | `20260911-r7/tabbit-after.json` and Computer Use observation |
| WeChat 3.9.9.43: blank draft, one image, automatic hide | PASS | `20260911-two-spaces/wechat-after.json` |
| Feishu 7.76.0.97: blank draft, one image, automatic hide | PASS | `20260911-r9/feishu-after.json` |
| Tabbit: search `199`, insert matching Saved Item, replace only query, hide | PASS | `20260911-r7/tabbit-text-after.json` |
| Two-space carousel: Favorites becomes main, History preview remains visible | PASS | `20260911-two-spaces/favorites-history-visible.json` |
| Four-space carousel: fixed main bounds and visible next preview | PASS | `20260911-r7/carousel-before.json`, `carousel-after.json` |
| Native input safety regression, 16 selected cases | PASS | `20260911-final/native-final/summary.json` |
| Original clipboard formats and capture exclusions | PASS | `20260911-final/clipboard-roundtrip.log` |
| Production input regression, including hidden reclamation, 5 cases | PASS | `20260911-final/production-input/summary.json` |
| Self-check, format, full verify | PASS | Gate logs retained in evidence |
| Production smoke: semantic startup, manual History, close-to-hide, graceful exit | PASS | `20260911-final/gates/smoke.log` |

Real application checks used a separate `native-test` executable. They do not certify every editor, every image size, or image replacement at every possible preexisting selection. Native safety regression covers exact text replacement, preselection, stale results, selection refusal, focus/clipboard races, unknown delivery, held Enter and cancellation.

The initial Feishu result did not survive a later image-block representation; that failure was retained and repaired using exact readback evidence. Earlier Codex and WeChat failed/unknown-result runs also remain in the evidence directories.

## Release verification

The production software build passed. SHA-256: `FE66E6B52A166D70E36D6B4C8DBDEE6D808DE49B4EDF8ED965F99AB1437130D4`. Build command: `echo.cmd build --release`, without `native-test` or GPU/Skia features. The dependency inventory is retained in `20260911-final/release-dependencies.txt`.

The production ten-minute run completed: two isolated fixtures, each visible for 120 seconds, hidden for 120 seconds and restored for 60 seconds. Each has 300 valid 1 Hz samples; process identity and software renderer were verified, and original database/blob signatures were unchanged. Memory alone is **PASS** against the strict 50,000,000-byte limit.

| Fixture | Stable visible maximum | Hidden after 35s maximum | Sampled peak | Reclaim completion | CPU time during sampling |
| --- | ---: | ---: | ---: | ---: | ---: |
| 2,000 text History / 200 Saved Items | 34,209,792 bytes | 18,898,944 bytes | 34,209,792 bytes | 30.001s | 0.703s |
| Same counts, 20 ordinary 1920x1080 images | 37,785,600 bytes | 22,839,296 bytes | 37,785,600 bytes | 30.002s | 0.625s |

Each fixture has 180 stable visible samples and 85 samples after the hidden grace period. Window bounds were (480,320)-(2080,1120) on the current desktop. Raw evidence is in `20260911-final/memory-T`, `memory-M` and `memory-summary.json`. The figures prove only this duration and environment. Separate desktop compositor GPU memory was **NOT_RUN**.

Both combined sampler results remain **FAIL** because animation P95 was 30.6344ms / 30.5358ms, exceeding the existing 20ms threshold. No other sampler errors occurred. Overall acceptance remains **PARTIAL**; this input repair does not close the prior animation gap.

Physical Chinese IME and mixed-monitor/DPI testing remain **NOT_RUN**. The previous animation frame-time acceptance gap remains open. Image insertion transients and 1 Hz sampled memory peaks are not allocation high-water proofs.

## Rollback

The preserved pre-image release is `20260911-r9/baseline-before-images.exe`, SHA-256 `015107363B7AB98E98A53EFE56B77E3B6361920C190E992309AA9CF18D079000`. Run the retained old program to roll back; do not delete or rewrite History, Saved Items or the database. The daily installation is not replaced by this task.
