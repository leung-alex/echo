# Software card carousel acceptance — 2026-09-11

Overall **PARTIAL**: ordinary-use memory and native functional checks pass;
animation timing fails the approved 20 ms P95 limit. The two raw combined
performance results remain FAIL. This is not a complete 60 FPS acceptance.

The software Release SHA256 is
`7F0BB0E9213169948062B464033D63FF06636897EE865EF65B4DECCE68B6C2C2`.
It was built by `echo.cmd build --release`, without default graphics features
or `native-test`. Runtime evidence identifies Slint software / CPU rasterizer.

| Metric | Text fixture | Mixed fixture |
| --- | ---: | ---: |
| Valid 1 Hz samples | 300 / 300 | 300 / 300 |
| Stable visible maximum, bytes | 34,508,800 | 38,576,128 |
| Reclaimed hidden maximum, bytes | 24,313,856 | 25,935,872 |
| Whole-run sampled maximum, bytes | 34,508,800 | 38,576,128 |
| Stable visible / hidden samples | 180 / 85 | 180 / 85 |
| Reclamation completion, seconds | 30.0018267 | 30.0022424 |
| Observed product CPU time, seconds | 0.859375 | 1.046875 |
| Presented-frame interval P95, ms | **32.4722 FAIL** | **32.1448 FAIL** |
| Presented-frame interval maximum, ms | 33.6850 | 34.3634 |

Each isolated fixture contains 2,000 History and 200 Saved Items. The mixed
fixture replaces exactly 20 History entries with ordinary 1920×1080 images.
Each run observes visible 120 s, hidden 120 s, restored 60 s. Hidden samples
are gated from 35 s. All stable product Private Bytes samples are strictly
below 50,000,000; process path, creation time, identity, count and graceful
exit were checked. Original representation and membership signatures match.
The native window is 1600×800 pixels at Slint scale factor 1.0.

The 1 Hz maxima do not prove an allocation high-water mark or capture every
180 ms animation/decode transient. GPU observers produced 262 and 268 records
without product counters (`NO_COUNTERS_NOT_ZERO`); absent counters are not zero,
and DWM memory has not been attributed to Echo.

`self-check`, `format --check`, full `verify`, Release build and canonical smoke
pass. The isolated native deck suite has 14 passing checks, and the original
input fixture has 14, including both neighbor filters, stale-result rejection,
Enter/cancel safety, focus/caret preservation, deep reclaim, paging and alpha
composition. The real in-motion screenshot is `motion-r7/active.png`; earlier
screenshots that waited for Idle are not motion evidence. Physical IME,
additional DPI/monitors and universal external-editor compatibility are NOT_RUN.

A separate Release check also passes three controlled captures after deep
reclamation: History remains capped at 2,000, Saved Items and memberships are
unchanged, and the prior clipboard is restored after graceful shutdown.

Raw evidence, baseline/source preservation, hashes and rollback details are in
`.local/devpack-evidence/software-deck/20260911-042720/REPORT.md` under the local
repository. The baseline software Release hash is
`5AB3D5D84B60FE8AE31482D771596310B9AEE9E2FB8E5B7F2D887555A65F2359`.
Rollback uses that preserved executable and keeps all real history and Saved
Items. No daily installation replacement or push is part of this delivery.

Implementation ownership and the user's final wide-card/real-preview decision
are recorded in [ADR 003](../architecture/ADR-003-software-cards.md).
