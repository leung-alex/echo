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

The older Memory50 GPU matrix below is historical and does not add extra timed
runs to the current approved software-card acceptance.

These Windows PowerShell scripts require an authorized idle Windows desktop and synthetic fixture directories, never the real Echo database. Preserve baseline executables and match fixture and observer hashes between versions.

`Measure-UiReady.ps1` retains archived dual-window/search assumptions and is not a Memory50 gate. For inline popup timing use `tests/native/Measure-PopupTiming.py` with its native pixel observer. Internal submit and UIA readiness alone do not establish a complete rendered first frame.

`Measure-Resident.ps1` supports Memory50 S0/S1/S2 and legacy D0/D1/D2 fixtures. It observes content in one native window, waits 35 seconds after hiding, samples resources, and checks database content signatures. Use at least 310 seconds to allow 300 valid 1 Hz samples. Exact run-specific executable paths supplement parent discovery. OS lifecycle subscriptions may be denied; UNAVAILABLE is retained and must not be treated as evidence that no helper existed. Independently review all process identities and lifecycle evidence before acceptance.

Scripts stop only the isolated product process they start. They do not measure GPU memory or physical input-method compatibility. Use the same script hashes, fixture data, Release mode and machine for both versions. A measurement record is not an automatic gate PASS.

The approved Memory50 long-run test is 600 seconds, replacing eight hours. It does not replace three independent R50 runs per dataset or the latency samples, and provides no eight-hour stability evidence.
