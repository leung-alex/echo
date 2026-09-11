# Matched native performance observations

## Current software-card contract

`Measure-SoftwareDeck.ps1` is the software-card Release gate. Generate fixtures
with `native_fixture <new-directory> --software-deck`. Run it once for `T` and
once for `M`, each in a new output directory: exactly 120 seconds visible,
120 hidden and 60 restored, at 1 Hz. The two runs total ten minutes. Both contain
2,000 History and 200 Saved Items; M replaces exactly 20 entries with ordinary
1920x1080 images. No large-image stress dataset is used for this gate.

All valid stable visible/restored and hidden-after-35-second samples must be
strictly below 50,000,000 aggregate product Private Bytes. Unknown identities,
missing samples, exits, unacknowledged reclamation and incomplete content fail.
The renderer must be software; motion requires a prepared-frame event and P95
presented frame intervals no greater than 20 ms. Original representation and blob
hashes are checked. Raw and classified samples remain alongside the exact binary
hash. The optional GPU observer is separate so a slow counter provider cannot
delay memory sampling; absent counters do not mean zero GPU usage.

## Supported tools

- `Measure-SoftwareDeck.ps1`: the single current ten-minute Release memory and
  motion contract above. Run T and M once each; do not add the retired GPU matrix.
- `Invoke-SoftwareCapture.ps1`: separately authorized synthetic capture and
  retained-payload checks, with clipboard preservation. This is not extra soak time.
- `tests/native/Invoke-SoftwareDeckGate.ps1`: current software carousel, side
  previews, theme, navigation and lifecycle regression, invoked by `echo.cmd acceptance ui`.
- `tests/native/Invoke-InlineAcceptance.py`: insertion, cancellation, selection,
  stale results, synchronized search and deep-reclaim recovery regression.
- `tests/native/Measure-PopupTiming.py` with `Analyze-PopupTiming.py`: first/warm
  activation and deep-hide recovery timing. Re-establish the owned target after
  the hidden wait; require 98% main-card appearance as well as content/side pixels.
- `tests/native/Measure-ColdPopup.py`: new process per timing sample, using the
  same isolated timing harness directory (echo-timing.exe, EchoInlineDriver.exe,
  EchoInlineFixture.exe and capture-disabled data). Run `python
  tests/native/Measure-ColdPopup.py <harness-directory> --count 30`. OS caches are
  not forcibly emptied. A capture region that omits any popup edge fails.
- `tools/perf-native/Measure-EchoProcessTree.ps1`: optional general diagnostic
  collector. Use exact run-specific `-ExpectedExecutable` paths. Missing lifecycle
  access stays UNAVAILABLE; SAMPLED_UNVERIFIED is never acceptance PASS.

These tools require authorized isolated synthetic data; desktop input runs also
require an idle desktop. Preserve fixture, binary and observer hashes. Pixel
observation, diagnostic builds and sampled peaks do not prove physical IME,
allocation high-water marks or universal application compatibility.

The old Cover Flow suite, dual-window readiness/resident wrappers and exploratory
GPU/Skia/large-image Memory50 runners have been retired. Their historical results
are not current gates. Reclamation is checked by the software memory gate and
inline recovery regression; it needs no separate legacy runner. Ten minutes only
proves that observation duration. Existing failed frame-time results remain FAIL.
