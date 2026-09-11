# Quick Insert regression and activation search

Evidence root: `.local/devpack-evidence/insert-regression/20260911-081334`.
Baseline main: `ff08d75b25554e2376441d580c373776f4131f2d`.
Frozen failing release SHA-256:
`7F0BB0E9213169948062B464033D63FF06636897EE865EF65B4DECCE68B6C2C2`.

## Behavior

Alt+V retains the original selection as the exact replacement range, but starts
with an empty search. Subsequent entered text filters the active and side cards.
Opening or cancelling Echo does not delete the original selection.

The Windows adapter confirms an already correct selection by reading it back,
without redundantly calling UIA Select. For a provider explicitly returning
E_NOTIMPL, a collapsed caret at the query end can extend left with bounded
Shift+Left steps. Each step must preserve all text and make monotonic progress
toward the exact requested range. There are at most 64 steps within the existing
deadline. Unsupported longer ranges can still fail safely; this is not a
universal provider guarantee.

Empty Editor Kit documents now recognize bounded, readonly hint siblings before
the existing three structural zero-width leaves. Literal zero-width characters
and original clipboard representations remain intact. This accommodates the
installed Feishu voice hint without matching an application name.

Unchanged provider observations retain preflight state. The commit guard covers
the initial verification read as well as selection and paste; physical input
still independently invalidates the ticket. Paste consumes preflight once and
rechecks target, range, clipboard and deadline. An unknown write outcome retains
its warning and repeat-insertion block across later caret observations.

## Diagnosis and regression evidence

- Codex: the original exact-selection error was reproduced, but was intermittent;
  later baseline empty insertions also succeeded. Do not infer that every original
  failure had the same root cause.
- The frozen failing software release was compared in the current environment.
  Re-running the older pre-software executable as a real-app control is NOT_RUN;
  these observations do not establish that software rendering caused the errors.
- WeChat: real UIA Select returned E_NOTIMPL. Existing-caret confirmation and
  typed-query range selection have actual-editor red/green evidence.
- Feishu: the new readonly voice hint polluted the empty snapshot. The resulting
  paste could succeed while acknowledgement failed. `feishu-red.log` and
  `feishu-green.log` verify the structural projection independently.
- `preflight-red.log` reproduces an unchanged real-provider observation discarding
  preflight. `preflight-green-r2.log` verifies the fix. The first green attempt
  was rejected because foreground had changed, and is retained as such.
- `unknown-red-r2` reproduces an unknown-outcome warning being overwritten after
  caret movement; `unknown-green-r6` verifies its correction.
- `search-selection-red.log` and `search-selection-green.log` cover the changed
  activation search contract.
- `native-r7b` passes 16 targeted cases, including focus and clipboard races,
  held Enter, stale results, cancellation, selection refusal, unknown outcome,
  and both-side filtering after an initially selected range. Earlier aborted
  foreground runs remain in the evidence root.

## Real application observations

Computer Use operated unsent synthetic drafts with explicit user authorization.
No message was sent. Tabbit used a new local fixture tab, whose submission
counter stayed zero; that tab was subsequently closed. This proves the real
browser host with these local controls, not every remote site's editor.

| Application | Empty insertion | Middle caret | Selected replacement | Typed search insertion | New activation-search contract |
| --- | --- | --- | --- | --- | --- |
| Codex | PASS r6 | PASS r6 | PASS r6 | PASS r6 | PASS r7 |
| WeChat, File Transfer Assistant | PASS r2 | PASS r6 | PASS r2 | PASS r2/r7 | PASS r7 |
| Feishu, self draft | PASS r5 | PASS r5 | PASS r5 | PASS r5/r7 | PASS r7 |
| Tabbit, local input fixture | PASS r7 | PASS r7 | PASS r7 | PASS r7 | PASS r7 |

These are native-test diagnostic executables from the successive fixes, not
release-memory evidence. Raw bridge snapshots are under `actual-native/`.
All original input prefixes/suffixes checked in the middle-caret cases survived;
successful insertion closed Echo. Test drafts were cleaned and clipboard backup
wrappers reported restoration. Physical IME and mixed-monitor/DPI coverage are
NOT_RUN. The prior animation frame-time acceptance gap remains open.

## Release validation

Release SHA-256:
`015107363B7AB98E98A53EFE56B77E3B6361920C190E992309AA9CF18D079000`.
Self-check, format, verify and the final canonical smoke passed. Production
input acceptance passed exact replacement, the new preselection behavior,
held Enter, cancellation and rearming without a native-test bridge.
`production-reclaim` also passed insertion after a matching, acknowledged deep
reclamation event within the 30-35 second contract.

The ten-minute release memory run used two capture-disabled synthetic libraries
(2000 History and 200 Saved Items; the second included 20 ordinary 1920x1080
images), each visible 120 s, hidden 120 s and restored 60 s. Capturing new user
clipboard content during these samples was not tested. The observed window was
1600 x 800 physical pixels on the current display configuration.

| Dataset | Stable visible maximum | Reclaimed hidden maximum | Sampled peak | Reclaim delay | Valid samples |
| --- | ---: | ---: | ---: | ---: | ---: |
| Text | 37,158,912 bytes | 18,239,488 bytes | 37,158,912 bytes | 30.0019 s | 300 |
| Mixed | 37,167,104 bytes | 20,361,216 bytes | 37,167,104 bytes | 30.0005 s | 300 |

Memory: PASS. Each dataset had 180 stable-visible and 85 stable-hidden samples,
no missing sample, unchanged original-representation signatures, acknowledged
reclamation and a graceful exit. Product CPU time was 0.796875/0.890625 seconds.
The runtime identified itself as the CPU software renderer. GPU compositor
allocation was not independently measured.

Animation remains FAIL: presented interval P95 was 33.5533/33.473 ms against
20 ms. Accordingly both raw combined measurement results remain FAIL; reporting
memory PASS does not reclassify animation or the combined gate. Overall broader
acceptance remains PARTIAL. Raw samples are in `formal-T-final` and `formal-M-final`.

Package paths and the local commit are recorded in the evidence root's final
report. A native-test executable must not be shipped.
The memory contract remains strictly below 50,000,000 aggregate Private Bytes
for stable visible and reclaimed-hidden samples. Sampling does not prove an
allocation high-water mark or behavior beyond the measured ten minutes.

Rollback uses the retained baseline executable or previous portable package.
No database migration, history deletion, push, installation replacement or
shutdown is part of this change.
