# Memory50 lifecycle and evidence

Historical GPU comparison. The software-card implementation and current
acceptance contract are defined by [ADR 003](../architecture/ADR-003-software-cards.md).

The original GPU experiment targeted aggregate product Private Bytes strictly below 50,000,000
after hidden reclamation, without changing normal GPU Cover Flow or input safety.
Short hides retain warm resources for 30 seconds; reclamation should finish by
35 seconds. Visible and transient memory are separate results. The approved soak
duration is 600 seconds, not eight hours.

## Implemented resource boundaries

`--background` starts the existing single-instance shell and domain worker, then
waits on the typed event hub's condition variable before selecting a Slint backend.
Capture/storage/hotkeys remain on their existing owners. Coalesced invalidations
and bootstrap events are retained for first activation; explicit Quit wakes the
coordinator without creating graphics. This is initial lazy initialization, not
proof that a previously initialized backend can be destroyed.

Hidden trim invalidates preview/image/page generations, releases page payloads and
Slint image/model references, and retires offscreen renderer/compositor owners.
Reopening rebuilds those owners against the existing shared device. The main
Slint backend is retained; no repeated backend selection or device destruction is
attempted. Pending mutation/modal state prevents unsafe reclamation.
Normal dismissal publishes the new session epoch before arming the hidden timer.
The timer and worker acknowledgment must match both that epoch and the hidden
generation. `check-reclaim.py` rejects a missing/late acknowledgment even when the
window successfully hides; visibility alone cannot establish reclamation.
An in-progress input transaction defers reclamation and retries within the same
hidden epoch/generation. The retry is discarded after a new activation. A busy
transaction can still miss the 35-second deadline; that remains a measured FAIL.
Automatic input-inspection fallback without a safe paste target keeps the
original input foreground. An explicit F6 request remains activating manual
History, including when it arrives before inspection completes. These origins
are carried explicitly rather than inferred from the notice text or pending flag.
The unavailable-input guard stays in the same session during an ordinary-target
probe; a verified target retires it before activating Echo, while copy-only
fallback retains Enter/Esc/F6 protection. Inspection timeout cancels the session
instead of opening a fallback that may race an unacknowledged or late provider.

Fuzzy search continues scanning the full space when its cache budget is exceeded.
Cache estimates include string capacities; overflowing candidate storage is
released during the scan. Original representations and storage durability are
unchanged.

Search candidates keep the full searchable document once, while ranked/list
results omit the duplicate editable body. Catalog rows do the same; Inspect
rehydrates the complete editor content, and execution still reads original
representations by ID. Full-document matching and pagination remain unchanged.

Worker display results use a 16 MiB weighted queue budget, including results
already dequeued for delivery. The producing worker may hold one additional
materialized result while waiting; control/activation/quit never wait for these
credits. An indivisible oversized editor result is delivered alone and explicitly
recorded as `oversized_display_result`, preserving the full content. Thus this is
backpressure, not a strict 16 MiB process/in-flight allocation ceiling or a P50
pass. Large-image transient allocations also remain separately reportable.

## Diagnostics and acceptance

`memory-diagnostics` is an attribution-only feature. With
`ECHO_MEMORY_NO_OFFSCREEN=1` it skips compositor and offscreen construction while
keeping the main GPU backend in that historical comparison. Current distribution
uses software-only builds with no default features, as specified by ADR 003;
diagnostic modes cannot substitute for current production acceptance.

For synthetic fixtures, `ECHO_WINDOWS_ACCEPTANCE=1` plus an existing
`ECHO_MEMORY_TRACE_DIR` enables bounded lifecycle JSONL without clipboard/query
content. Region inventories from `tests/performance/memory-map.py` read no memory
contents and are not allocation-stack evidence.

`tests/performance/Invoke-Memory50.ps1` orchestrates an isolated run using the
development pack's immutable wrapper. It checks retained database content and
semantic restore, but does not certify a complete rendered first frame. Use the
native pixel timing observer for that separate endpoint.
The historical GPU formal runs reject inherited renderer/diagnostic overrides and require
the actual perspective GPU selection event. Baseline builds without that event
remain explicitly diagnostic.

R50 requires three independent runs for each S0/S1/S2 fixture, with at least 300
valid samples over at least 300 seconds after the 35-second settling interval.
`Invoke-Memory50Soak.ps1` runs controlled capture/show/hide for 600 seconds through
the existing clipboard-preservation wrapper. Neither short diagnostics nor a
process exiting unexpectedly can satisfy acceptance.
`Invoke-Memory50Cycles.ps1` exercises 100 owned-window cycles and crosses the
reclamation threshold every twentieth cycle. `Invoke-Memory50LargeImages.ps1`
observes isolated 4K/8K capture and repeats both images at a requested 100 ms
sampling interval. Actual collection cost and missing samples remain observable;
this does not prove an allocation high-water mark.
The image check compares retained bytes to the actual Windows DIB representation
and applies the existing size policy. It rejects partial input/measurement runs.
Capture tests register independent owned-process handles with the clipboard
preservation wrapper. Personal clipboard restoration requires those processes to
have exited; forced cleanup of a failed test is recorded as a test failure.

Process lifecycle subscriptions may require privileges unavailable in the current
session. Preserve UNAVAILABLE/Access denied and independently review ownership;
never turn missing metrics into zero or silently upgrade SAMPLED_UNVERIFIED.
The M05 process-split decision must still be based on measured retention and a
prototype that meets the approved latency and session-safety gates.
